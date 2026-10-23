import { useEffect, useMemo, useRef, useState } from "react";
import { chooseExportFolder, errorText, onBackupLog, pg, toAtlasError, type ToolsInfo } from "../backend";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, categoryIcon } from "../icons";
import type { Category } from "../model";
import { readToolsDir, saveToolsDir } from "./RestoreDialog";

/** Grupos da seleção de objetos: o `pg_dump` deixa de fora tabelas, views e sequências por nome; funções e tipos entram sempre. */
type GroupId = "table" | "view" | "matview" | "function" | "sequence" | "type";
const GROUPS: { id: GroupId; label: MessageKey; selectable: boolean }[] = [
  { id: "table", label: "tb.table", selectable: true },
  { id: "view", label: "tb.view", selectable: true },
  { id: "matview", label: "cat.matview", selectable: true },
  { id: "function", label: "tb.function", selectable: false },
  { id: "sequence", label: "cat.sequence", selectable: true },
  { id: "type", label: "cat.type", selectable: false },
];

interface Obj { schema: string; name: string; rows?: number }

export interface BackupTarget {
  connId: string;
  database: string;
  /** Sem schema o backup é do banco inteiro. */
  schema?: string;
  /** "Postgres 9.6", como o servidor se apresenta. */
  server: string;
  /** Objetos do schema, por grupo (nome e quantidade estimada de linhas). */
  objects: Record<GroupId, Obj[]>;
}

interface Props {
  target: BackupTarget;
  onClose: () => void;
  /** Chamado ao terminar um backup, para a lista da categoria Backup se atualizar. */
  onDone: () => void;
}

const pad = (n: number) => String(n).padStart(2, "0");
/** Aplica `%Y %m %d %H %M %S` (ano, mês, dia, hora, minuto e segundo) ao padrão de nome do arquivo. */
export function formatBackupName(pattern: string, d = new Date()): string {
  return pattern
    .replace(/%Y/g, String(d.getFullYear())).replace(/%m/g, pad(d.getMonth() + 1)).replace(/%d/g, pad(d.getDate()))
    .replace(/%H/g, pad(d.getHours())).replace(/%M/g, pad(d.getMinutes())).replace(/%S/g, pad(d.getSeconds()));
}
const DEFAULT_PATTERN = "Backup_%Y%m%d%H%M%S";

type Tab = "general" | "objects" | "advanced" | "log";

export function BackupDialog({ target, onClose, onDone }: Props) {
  const t = useT();
  const [tab, setTab] = useState<Tab>("general");
  const [comment, setComment] = useState("");
  const [useName, setUseName] = useState(false);
  const [pattern, setPattern] = useState(DEFAULT_PATTERN);
  const [tools, setTools] = useState<ToolsInfo | null>(null);
  const [toolsDir, setToolsDir] = useState(readToolsDir);
  const [folder, setFolder] = useState("");
  const [excluded, setExcluded] = useState<Set<string>>(new Set());
  const [collapsed, setCollapsed] = useState<Set<GroupId>>(new Set(["view", "matview", "function", "type"]));
  const [running, setRunning] = useState(false);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [log, setLog] = useState<string[]>([]);
  const [processed, setProcessed] = useState<{ objects: number; records: number }>({ objects: 0, records: 0 });
  const [finished, setFinished] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const jobId = useRef("");
  const logEnd = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    setTools(null);
    void pg.toolsLocate(toolsDir).then((r) => { if (alive) setTools(r); }).catch(() => { if (alive) setTools({ dir: null, version: null, others: [] }); });
    return () => { alive = false; };
  }, [toolsDir]);

  useEffect(() => {
    if (!running) return;
    const started = Date.now();
    const id = window.setInterval(() => setElapsedMs(Date.now() - started), 100);
    return () => window.clearInterval(id);
  }, [running]);

  useEffect(() => { logEnd.current?.scrollIntoView({ block: "end" }); }, [log]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !running) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, running]);

  const key = (g: GroupId, o: Obj) => `${g}:${o.schema}.${o.name}`;
  const label = (o: Obj) => (target.schema ? o.name : `${o.schema}.${o.name}`);
  const rowsOf = useMemo(() => {
    const m = new Map<string, number>();
    for (const g of GROUPS) for (const o of target.objects[g.id]) m.set(`${o.schema}.${o.name}`, o.rows ?? 0);
    return m;
  }, [target]);
  const total = GROUPS.reduce((n, g) => n + target.objects[g.id].filter((o) => !g.selectable || !excluded.has(key(g.id, o))).length, 0);

  const toggleItem = (g: GroupId, o: Obj) =>
    setExcluded((s) => { const n = new Set(s); const k = key(g, o); if (n.has(k)) n.delete(k); else n.add(k); return n; });
  const setGroup = (g: GroupId, on: boolean) =>
    setExcluded((s) => { const n = new Set(s); for (const o of target.objects[g]) { if (on) n.delete(key(g, o)); else n.add(key(g, o)); } return n; });
  const setAll = (on: boolean) => { for (const g of GROUPS) if (g.selectable) setGroup(g.id, on); };
  const toggleOpen = (g: GroupId) => setCollapsed((s) => { const n = new Set(s); if (n.has(g)) n.delete(g); else n.add(g); return n; });

  const chooseFolder = async () => {
    const dir = await chooseExportFolder();
    if (dir) setFolder(dir);
  };

  const chooseTools = async () => {
    const dir = await chooseExportFolder();
    if (!dir) return;
    saveToolsDir(dir);
    setToolsDir(dir);
  };

  const start = async () => {
    setError(null);
    setLog([]);
    setFinished(false);
    setProcessed({ objects: 0, records: 0 });
    setElapsedMs(0);
    setTab("log");
    setRunning(true);
    const id = crypto.randomUUID();
    jobId.current = id;
    let objects = 0;
    let records = 0;
    const unlisten = await onBackupLog(id, (line) => {
      setLog((l) => (l.length > 2000 ? [...l.slice(-1000), line] : [...l, line]));
      // "pg_dump: dumping contents of table "public.tb"" e "executing SEQUENCE SET": um objeto de dados processado.
      const m = /dumping contents of table "?([^"]+)"?/.exec(line);
      if (m) { objects++; records += rowsOf.get(m[1]) ?? 0; setProcessed({ objects, records }); }
      else if (/executing SEQUENCE SET/.test(line)) { objects++; setProcessed({ objects, records }); }
    });
    const exclude = GROUPS.flatMap((g) => (g.selectable ? target.objects[g.id].filter((o) => excluded.has(key(g.id, o))).map((o) => `${o.schema}.${o.name}`) : []));
    try {
      const r = await pg.backup({
        jobId: id, sessionId: target.connId, database: target.database, schema: target.schema, toolsDir: toolsDir || undefined,
        fileName: formatBackupName(useName ? pattern : DEFAULT_PATTERN), comment, exclude, folder: folder || undefined,
      });
      setElapsedMs(r.elapsedMs);
      setProcessed({ objects: total, records });
      setFinished(true);
      setLog((l) => [...l, "", t("bk.doneBackup", { name: r.name })]);
      onDone();
    } catch (e) {
      const err = toAtlasError(e);
      setError(err.kind === "canceled" ? t("bk.canceled") : errorText(err));
    } finally {
      unlisten();
      setRunning(false);
    }
  };

  const cancel = () => { void pg.backupCancel(jobId.current).catch(() => undefined); };
  const canStart = !running && !!tools?.dir && total > 0 && (!useName || pattern.trim() !== "");
  const started = running || finished || log.length > 0;
  const dash = (v: string) => (started ? v : "-");
  const pct = total > 0 ? Math.min(100, Math.round((processed.objects / total) * 100)) : 0;

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal backup-dialog bk-nav" role="dialog" aria-label={t("bk.titleBackup")}>
        <div className="modal-header"><span className="modal-title">{t("bk.titleBackup")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose} disabled={running}><SmallIcon name="close" /></button></div>
        <div className="dlg-tabs" role="tablist">
          {([["general", "bk.tabGeneral"], ["objects", "bk.tabObjects"], ["advanced", "bk.tabAdvanced"], ["log", "bk.tabLog"]] as const).map(([id, label]) => (
            <button key={id} type="button" role="tab" aria-selected={tab === id} className={`dlg-tab${tab === id ? " is-active" : ""}`} onClick={() => setTab(id)}>{t(label)}</button>
          ))}
        </div>
        <div className="modal-body bk-body">
          {tab === "general" && (
            <>
              <div className="bk-info">
                <span>{t("bk.server")}:</span><span>{target.server}</span>
                <span>{t("bk.database")}:</span><span>{target.schema ? `${target.database}/${target.schema}` : target.database}</span>
              </div>
              <fieldset className="bk-group">
                <legend>{t("bk.options")}</legend>
                <label htmlFor="bk-comment">{t("bk.comment")}:</label>
                <textarea id="bk-comment" className="field bk-comment" value={comment} disabled={running} onChange={(e) => setComment(e.target.value)} />
              </fieldset>
              <div className="bk-note">{t("bk.note")}</div>
              {tools && !tools.dir && (
                <div className="doc-banner is-warn bk-tools">
                  {t("bk.toolsMissing")} <button type="button" className="btn" onClick={() => void chooseTools()}>{t("bk.toolsChoose")}</button>
                </div>
              )}
            </>
          )}

          {tab === "objects" && (
            <>
              <div>{t("bk.objectsHint")}</div>
              <div className="bk-objects-label">{t("bk.objects")}</div>
              <div className="bk-tree">
                {GROUPS.map((g) => {
                  const items = target.objects[g.id];
                  const sel = items.filter((o) => !excluded.has(key(g.id, o))).length;
                  const open = !collapsed.has(g.id) && items.length > 0;
                  return (
                    <div key={g.id}>
                      <div className="bk-row">
                        <button type="button" className="bk-twisty" aria-label={t(g.label)} onClick={() => toggleOpen(g.id)} disabled={items.length === 0}>
                          {items.length > 0 && <SmallIcon name={open ? "chevronDown" : "chevronRight"} />}
                        </button>
                        <input
                          type="checkbox" disabled={running || !g.selectable} title={g.selectable ? undefined : t("bk.alwaysIncluded")}
                          checked={!g.selectable || sel > 0}
                          ref={(el) => { if (el) el.indeterminate = g.selectable && sel > 0 && sel < items.length; }}
                          onChange={(e) => setGroup(g.id, e.target.checked)}
                        />
                        <SmallIcon name={categoryIcon(g.id as Category)} />
                        <span>{t(g.label)} ( {items.length === 0 ? 0 : g.selectable ? sel : "*"} )</span>
                      </div>
                      {open && items.map((o) => (
                        <div key={`${o.schema}.${o.name}`} className="bk-row bk-child">
                          <input type="checkbox" disabled={running || !g.selectable} checked={!g.selectable || !excluded.has(key(g.id, o))} onChange={() => toggleItem(g.id, o)} />
                          <SmallIcon name={categoryIcon(g.id as Category)} />
                          <span>{label(o)}</span>
                        </div>
                      ))}
                    </div>
                  );
                })}
              </div>
              <div className="bk-buttons">
                <button type="button" className="btn" disabled={running} onClick={() => setAll(true)}>{t("bk.selectAll")}</button>
                <button type="button" className="btn" disabled={running} onClick={() => setAll(false)}>{t("bk.unselectAll")}</button>
              </div>
            </>
          )}

          {tab === "advanced" && (
            <>
              <div className="bk-folder-label">{t("bk.folder")}</div>
              <div className="bk-folder">
                <input className="field" value={folder} placeholder={t("bk.folderDefault")} readOnly disabled={running} />
                <button type="button" className="btn" disabled={running} onClick={() => void chooseFolder()}>{t("exp.browse")}</button>
                <button type="button" className="btn" disabled={running || folder === ""} onClick={() => setFolder("")}>{t("bk.folderReset")}</button>
              </div>
              <label className="check-row"><input type="checkbox" checked={useName} disabled={running} onChange={(e) => setUseName(e.target.checked)} /><span>{t("bk.useName")}</span></label>
              <input className="field bk-pattern" value={useName ? pattern : DEFAULT_PATTERN} disabled={!useName || running} onChange={(e) => setPattern(e.target.value)} spellCheck={false} />
              <div className="bk-format">
                <div>{t("bk.nameFormat")}</div>
                <div>%Y {t("bk.fmtYear")}, %m {t("bk.fmtMonth")}, %d {t("bk.fmtDay")}</div>
                <div>%H {t("bk.fmtHour")}, %M {t("bk.fmtMinute")}, %S {t("bk.fmtSecond")}</div>
              </div>
            </>
          )}

          {tab === "log" && (
            <>
              <div className="bk-info bk-stats">
                <span>{t("bk.lblObjects")}:</span><span>{dash(String(total))}</span>
                <span>{t("bk.lblProcessed")}:</span><span>{dash(String(processed.objects))}</span>
                <span>{t("bk.lblRecords")}:</span><span>{dash(processed.records.toLocaleString())}</span>
                <span>{t("bk.lblTime")}:</span><span>{dash(`${(elapsedMs / 1000).toFixed(3)}s`)}</span>
              </div>
              <div className="bk-log bk-log-fill" aria-label={t("bk.log")}>
                {log.map((l, i) => <div key={i}>{l}</div>)}
                {error && <div className="bk-error-line">{error}</div>}
                <div ref={logEnd} />
              </div>
              <div className="bk-progress" role="progressbar" aria-valuenow={finished ? 100 : pct} aria-valuemin={0} aria-valuemax={100}>
                <div className="bk-progress-bar" style={{ width: `${finished ? 100 : pct}%` }} />
              </div>
            </>
          )}
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          {running
            ? <button type="button" className="btn" onClick={cancel}>{t("exp.cancel")}</button>
            : <button type="button" className="btn btn-primary" disabled={!canStart} onClick={() => void start()}>{t("bk.backup")}</button>}
          <button type="button" className="btn" disabled={running} onClick={onClose}>{t("tab.close")}</button>
        </div>
      </div>
    </div>
  );
}
