import { useEffect, useState } from "react";
import { chooseExportFolder, errorText, pg, toAtlasError } from "../backend";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon } from "../icons";
import { buildFormat, defaultOptions, formatOf, FORMATS, type Kind } from "./export/exportModel";
import { MessageDialog } from "./MessageDialog";
import {
  describeInterval, formatWhen, nextRunMs, scheduleProblem, uniqueName, type Schedule, type ScheduledTask,
} from "../tasks";

export interface TaskConn {
  id: string;
  name: string;
}

interface Props {
  tasks: ScheduledTask[];
  conns: TaskConn[];
  /** Tarefas em execução agora (por id). */
  running: Set<string>;
  onChange: (tasks: ScheduledTask[]) => void;
  onRun: (id: string) => void;
  onClose: () => void;
}

type Draft = {
  id: string | null;
  name: string;
  kind: "backup" | "export";
  connId: string;
  database: string;
  schema: string;
  table: string;
  format: Kind;
  namePattern: string;
  comment: string;
  folder: string;
  scheduleType: Schedule["type"];
  time: string;
  every: string;
  unit: "minutes" | "hours" | "days";
  once: string;
  enabled: boolean;
};

const UNIT_MIN = { minutes: 1, hours: 60, days: 1440 } as const;
const todayLocal = () => {
  const d = new Date(Date.now() + 3600_000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:00`;
};

function blank(conns: TaskConn[]): Draft {
  return {
    id: null, name: "", kind: "backup", connId: conns[0]?.id ?? "", database: "", schema: "", table: "", format: "csv",
    namePattern: "", comment: "", folder: "", scheduleType: "daily", time: "02:00", every: "1", unit: "days", once: todayLocal(), enabled: true,
  };
}

function toDraft(task: ScheduledTask): Draft {
  const s = task.schedule;
  const iv = s.type === "interval" ? (() => { const d = describeInterval(s.minutes); return { every: String(d.n), unit: d.unit }; })() : { every: "1", unit: "days" as const };
  const exp = task.job.kind === "export" ? task.job : null;
  return {
    id: task.id, name: task.name, kind: task.job.kind, connId: task.connId, database: task.database, schema: task.schema ?? "",
    table: exp?.table ?? "", format: (exp?.format as Kind) ?? "csv", namePattern: task.job.namePattern, comment: task.job.kind === "backup" ? task.job.comment : "",
    folder: task.folder, scheduleType: s.type, time: s.type === "daily" ? s.time : "02:00", ...iv, once: s.type === "once" ? s.at : todayLocal(), enabled: task.enabled,
  };
}

const DEFAULT_BACKUP_PATTERN = "Backup_%Y%m%d%H%M%S";
const defaultExportPattern = (table: string) => `${table || "export"}_%Y%m%d%H%M%S`;

function scheduleOf(d: Draft): Schedule {
  if (d.scheduleType === "daily") return { type: "daily", time: d.time };
  if (d.scheduleType === "once") return { type: "once", at: d.once };
  return { type: "interval", minutes: Math.round(Number(d.every) * UNIT_MIN[d.unit]) };
}

/** "Scheduled Tasks": backups e exportações que o Atlas roda sozinho enquanto está aberto. */
export function TasksDialog({ tasks, conns, running, onChange, onRun, onClose }: Props) {
  const t = useT();
  const [sel, setSel] = useState<string | null>(tasks[0]?.id ?? null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [databases, setDatabases] = useState<string[]>([]);
  const [schemas, setSchemas] = useState<string[]>([]);
  const [tables, setTables] = useState<string[]>([]);
  const [confirmDelete, setConfirmDelete] = useState<ScheduledTask | null>(null);
  const [, tick] = useState(0);

  // A lista de "próxima execução" acompanha o relógio.
  useEffect(() => {
    const id = window.setInterval(() => tick((n) => n + 1), 30_000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !draft && !confirmDelete) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, draft, confirmDelete]);

  // Bancos, schemas e tabelas do formulário vêm do servidor (a conexão escolhida precisa estar aberta).
  const connId = draft?.connId;
  useEffect(() => {
    if (!connId) return setDatabases([]);
    let alive = true;
    pg.listDatabases(connId).then((r) => { if (alive) { setDatabases(r.databases); setError(null); } }).catch(() => { if (alive) { setDatabases([]); setError(t("tk.openConnection")); } });
    return () => { alive = false; };
  }, [connId, t]);
  const database = draft?.database;
  useEffect(() => {
    if (!connId || !database) return setSchemas([]);
    let alive = true;
    pg.listSchemas(connId, database).then((r) => alive && setSchemas(r.map((s) => s.name))).catch(() => alive && setSchemas([]));
    return () => { alive = false; };
  }, [connId, database]);
  const schema = draft?.schema;
  const exporting = draft?.kind === "export";
  useEffect(() => {
    if (!connId || !database || !schema || !exporting) return setTables([]);
    let alive = true;
    pg.listObjects(connId, database, schema).then((o) => alive && setTables(o.tables.map((x) => x.name))).catch(() => alive && setTables([]));
    return () => { alive = false; };
  }, [connId, database, schema, exporting]);

  const set = (p: Partial<Draft>) => { setError(null); setDraft((d) => (d ? { ...d, ...p } : d)); };

  const connName = (id: string) => conns.find((c) => c.id === id)?.name ?? t("tk.unknownConn");
  const targetOf = (x: ScheduledTask) => {
    const where = `${connName(x.connId)} / ${x.database}${x.schema ? ` / ${x.schema}` : ""}`;
    return x.job.kind === "export" ? `${where} / ${x.job.table} (${x.job.format})` : where;
  };
  const scheduleText = (s: Schedule): string => {
    if (s.type === "daily") return t("tk.sched.daily", { time: s.time });
    if (s.type === "once") return t("tk.sched.once", { when: s.at.replace("T", " ") });
    const d = describeInterval(s.minutes);
    return t("tk.sched.interval", { n: d.n, unit: t(`tk.unit.${d.unit}` as MessageKey) });
  };

  const save = () => {
    if (!draft) return;
    const name = draft.name.trim();
    if (!name) return setError(t("tk.needName"));
    if (!draft.connId || !draft.database) return setError(t("tk.needTarget"));
    if (draft.kind === "export" && (!draft.schema || !draft.table)) return setError(t("tk.needTable"));
    const sch = scheduleOf(draft);
    const problem = scheduleProblem(sch);
    if (problem) return setError(t(problem === "time" ? "tk.badTime" : problem === "interval" ? "tk.badInterval" : "tk.badOnce"));
    const pattern = draft.namePattern.trim() || (draft.kind === "backup" ? DEFAULT_BACKUP_PATTERN : defaultExportPattern(draft.table));
    const fmt = formatOf(draft.format);
    const old = tasks.find((x) => x.id === draft.id);
    const task: ScheduledTask = {
      id: draft.id ?? crypto.randomUUID(), name: uniqueName(name, tasks, draft.id ?? undefined), enabled: draft.enabled,
      connId: draft.connId, database: draft.database, schema: draft.schema || undefined, folder: draft.folder.trim(),
      job: draft.kind === "backup"
        ? { kind: "backup", namePattern: pattern, comment: draft.comment }
        : { kind: "export", table: draft.table, format: draft.format, exportFormat: buildFormat(draft.format, defaultOptions(), draft.table), extension: fmt.ext[0], namePattern: pattern },
      schedule: sch, createdMs: Date.now(),
      // Editar o agendamento recomeça a contagem; editar só o nome ou o destino mantém o histórico.
      ...(old && JSON.stringify(old.schedule) === JSON.stringify(sch) ? { createdMs: old.createdMs, lastRunMs: old.lastRunMs, lastStatus: old.lastStatus, lastMessage: old.lastMessage } : {}),
    };
    onChange(draft.id ? tasks.map((x) => (x.id === draft.id ? task : x)) : [...tasks, task]);
    setSel(task.id);
    setDraft(null);
  };

  const selected = tasks.find((x) => x.id === sel) ?? null;
  const choose = async () => {
    const dir = await chooseExportFolder();
    if (dir) set({ folder: dir });
  };

  const form = draft && (
    <div className="modal-overlay" role="presentation">
      <div className="modal tk-form" role="dialog" aria-label={t(draft.id ? "tk.edit" : "tk.new")}>
        <div className="modal-header">
          <span className="modal-title">{t(draft.id ? "tk.edit" : "tk.new")}</span>
          <button type="button" className="modal-close" aria-label={t("tab.close")} onClick={() => setDraft(null)}><SmallIcon name="close" /></button>
        </div>
        <div className="modal-body tk-grid">
          <label htmlFor="tk-name">{t("tk.name")}:</label>
          <input id="tk-name" className="field" value={draft.name} spellCheck={false} autoFocus onChange={(e) => set({ name: e.target.value })} />
          <label htmlFor="tk-kind">{t("tk.type")}:</label>
          <select id="tk-kind" className="field" value={draft.kind} onChange={(e) => set({ kind: e.target.value as Draft["kind"] })}>
            <option value="backup">{t("tk.backup")}</option>
            <option value="export">{t("tk.export")}</option>
          </select>
          <label htmlFor="tk-conn">{t("tk.connection")}:</label>
          <select id="tk-conn" className="field" value={draft.connId} onChange={(e) => set({ connId: e.target.value, database: "", schema: "", table: "" })}>
            {conns.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
          </select>
          <label htmlFor="tk-db">{t("tk.database")}:</label>
          <select id="tk-db" className="field" value={draft.database} onChange={(e) => set({ database: e.target.value, schema: "", table: "" })}>
            <option value="" />
            {draft.database && !databases.includes(draft.database) && <option value={draft.database}>{draft.database}</option>}
            {databases.map((d) => <option key={d} value={d}>{d}</option>)}
          </select>
          <label htmlFor="tk-schema">{t("tk.schema")}:</label>
          <select id="tk-schema" className="field" value={draft.schema} onChange={(e) => set({ schema: e.target.value, table: "" })}>
            <option value="">{draft.kind === "backup" ? t("tk.wholeDatabase") : ""}</option>
            {draft.schema && !schemas.includes(draft.schema) && <option value={draft.schema}>{draft.schema}</option>}
            {schemas.map((s) => <option key={s} value={s}>{s}</option>)}
          </select>
          {draft.kind === "export" && (
            <>
              <label htmlFor="tk-table">{t("tk.table")}:</label>
              <select id="tk-table" className="field" value={draft.table} onChange={(e) => set({ table: e.target.value })}>
                <option value="" />
                {draft.table && !tables.includes(draft.table) && <option value={draft.table}>{draft.table}</option>}
                {tables.map((x) => <option key={x} value={x}>{x}</option>)}
              </select>
              <label htmlFor="tk-format">{t("tk.format")}:</label>
              <select id="tk-format" className="field" value={draft.format} onChange={(e) => set({ format: e.target.value as Kind })}>
                {FORMATS.map((f) => <option key={f.kind} value={f.kind}>{t(f.label)}</option>)}
              </select>
            </>
          )}
          <label htmlFor="tk-folder">{t("tk.folder")}:</label>
          <div className="tk-row">
            <input id="tk-folder" className="field" value={draft.folder} placeholder={t("tk.folderDefault")} spellCheck={false} onChange={(e) => set({ folder: e.target.value })} />
            <button type="button" className="btn" onClick={() => void choose()}>…</button>
          </div>
          <label htmlFor="tk-pattern">{t("tk.fileName")}:</label>
          <input id="tk-pattern" className="field" value={draft.namePattern} placeholder={draft.kind === "backup" ? DEFAULT_BACKUP_PATTERN : defaultExportPattern(draft.table)} spellCheck={false} onChange={(e) => set({ namePattern: e.target.value })} />
          {draft.kind === "backup" && (
            <>
              <label htmlFor="tk-comment">{t("tk.comment")}:</label>
              <input id="tk-comment" className="field" value={draft.comment} onChange={(e) => set({ comment: e.target.value })} />
            </>
          )}
          <label htmlFor="tk-when">{t("tk.schedule")}:</label>
          <div className="tk-row">
            <select id="tk-when" className="field" value={draft.scheduleType} onChange={(e) => set({ scheduleType: e.target.value as Schedule["type"] })}>
              <option value="daily">{t("tk.optDaily")}</option>
              <option value="interval">{t("tk.optInterval")}</option>
              <option value="once">{t("tk.optOnce")}</option>
            </select>
            {draft.scheduleType === "daily" && <input type="time" className="field" value={draft.time} onChange={(e) => set({ time: e.target.value })} />}
            {draft.scheduleType === "interval" && (
              <>
                <input className="field tk-num" inputMode="numeric" value={draft.every} onChange={(e) => set({ every: e.target.value.replace(/[^\d]/g, "") })} />
                <select className="field" value={draft.unit} onChange={(e) => set({ unit: e.target.value as Draft["unit"] })}>
                  <option value="minutes">{t("tk.unit.minutes")}</option>
                  <option value="hours">{t("tk.unit.hours")}</option>
                  <option value="days">{t("tk.unit.days")}</option>
                </select>
              </>
            )}
            {draft.scheduleType === "once" && <input type="datetime-local" className="field" value={draft.once} onChange={(e) => set({ once: e.target.value })} />}
          </div>
          <span />
          <label className="check-row"><input type="checkbox" checked={draft.enabled} onChange={(e) => set({ enabled: e.target.checked })} /><span>{t("tk.enabled")}</span></label>
        </div>
        {error && <div className="doc-banner is-warn">{error}</div>}
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={save}>OK</button>
          <button type="button" className="btn" onClick={() => setDraft(null)}>{t("opt.cancel")}</button>
        </div>
      </div>
    </div>
  );

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal tk-dialog" role="dialog" aria-label={t("tk.title")}>
        <div className="modal-header">
          <span className="modal-title">{t("tk.title")}</span>
          <button type="button" className="modal-close" aria-label={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button>
        </div>
        <div className="tk-toolbar">
          <button type="button" className="ot-button" disabled={!conns.length} onClick={() => { setError(null); setDraft(blank(conns)); }}><SmallIcon name="new" /><span>{t("tk.new")}</span></button>
          <button type="button" className="ot-button" disabled={!selected} onClick={() => { if (selected) { setError(null); setDraft(toDraft(selected)); } }}><SmallIcon name="design" /><span>{t("tk.edit")}</span></button>
          <button type="button" className="ot-button" disabled={!selected || running.has(selected.id)} onClick={() => selected && onRun(selected.id)}><SmallIcon name="run" /><span>{t("tk.runNow")}</span></button>
          <button type="button" className="ot-button" disabled={!selected} onClick={() => selected && onChange(tasks.map((x) => (x.id === selected.id ? { ...x, enabled: !x.enabled } : x)))}>
            <SmallIcon name={selected?.enabled ? "stop" : "apply"} /><span>{t(selected?.enabled ? "tk.disable" : "tk.enable")}</span>
          </button>
          <button type="button" className="ot-button" disabled={!selected} onClick={() => setConfirmDelete(selected)}><SmallIcon name="delete" /><span>{t("tk.delete")}</span></button>
        </div>
        <div className="tk-list" role="grid" aria-label={t("tk.title")}>
          <div className="tk-head tk-cols"><span>{t("tk.name")}</span><span>{t("tk.type")}</span><span>{t("tk.target")}</span><span>{t("tk.schedule")}</span><span>{t("tk.next")}</span><span>{t("tk.last")}</span></div>
          {tasks.length === 0 && <div className="hist-empty">{t("tk.none")}</div>}
          {tasks.map((x) => {
            const next = x.enabled ? nextRunMs(x) : null;
            return (
              <div key={x.id} role="row" className={`tk-cols tk-row-item${sel === x.id ? " is-selected" : ""}${x.enabled ? "" : " is-off"}`} onClick={() => setSel(x.id)} onDoubleClick={() => { setSel(x.id); setError(null); setDraft(toDraft(x)); }}>
                <span title={x.name}>{x.name}</span>
                <span>{t(x.job.kind === "backup" ? "tk.backup" : "tk.export")}</span>
                <span title={targetOf(x)}>{targetOf(x)}</span>
                <span>{scheduleText(x.schedule)}</span>
                <span>{running.has(x.id) ? t("tk.runningNow") : !x.enabled ? t("tk.off") : next === null ? "—" : formatWhen(next)}</span>
                <span className={x.lastStatus === "error" ? "tk-bad" : undefined} title={x.lastMessage}>
                  {x.lastRunMs === undefined ? "—" : `${formatWhen(x.lastRunMs)} · ${t(x.lastStatus === "error" ? "tk.failed" : "tk.ok")}`}
                </span>
              </div>
            );
          })}
        </div>
        {selected?.lastMessage && <div className={`tk-msg${selected.lastStatus === "error" ? " is-bad" : ""}`}>{selected.lastMessage}</div>}
        <div className="muted tk-note">{t("tk.note")}</div>
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={onClose}>{t("tab.close")}</button>
        </div>
      </div>
      {form}
      {confirmDelete && (
        <MessageDialog
          title={t("tk.delete")}
          message={t("tk.confirmDelete", { name: confirmDelete.name })}
          danger
          okLabel={t("tk.delete")}
          onOk={() => { onChange(tasks.filter((x) => x.id !== confirmDelete.id)); setSel(null); setConfirmDelete(null); }}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}

export const taskErrorText = (e: unknown): string => errorText(toAtlasError(e));
