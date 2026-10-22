import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MenuEntry } from "../../commands";
import {
  chooseExportFolder, chooseExportPath, defaultExportDir, errorText, local, onExportProgress, pathSeparator, pg, toAtlasError,
} from "../../backend";
import { useT, type MessageKey } from "../../i18n";
import { SmallIcon } from "../../icons";
import { MessageDialog } from "../MessageDialog";
import { PopupMenu } from "../PopupMenu";
import { ExportOptionsDialog } from "./ExportOptionsDialog";
import {
  FORMATS, PROFILES_KEY, UNAVAILABLE_FORMATS, buildFormat, defaultPath, formatBytes, formatClock, formatOf, loadLast, mergeOptions,
  saveLast, swapExtension, withFolder, type ExportProfile, type ExportTarget, type Kind, type Options,
} from "./exportModel";

export type { ExportTarget } from "./exportModel";

interface Props {
  target: ExportTarget;
  onClose: () => void;
}

interface Row {
  checked: boolean;
  path: string;
}

type ColumnsState = string[] | "loading" | { error: string };

const STEPS = 4;
const LAST = STEPS - 1;
const HEADINGS: MessageKey[] = ["exp.step0", "exp.step1", "exp.step2", "exp.step3"];

/**
 * Assistente de exportação em etapas: formato → tabelas e arquivos de destino → campos → resumo
 * com andamento. Exporta várias tabelas em sequência; cada uma vira um arquivo.
 */
export function ExportWizard({ target, onClose }: Props) {
  const t = useT();
  const last = useMemo(loadLast, []);
  const sep = useMemo(pathSeparator, []);

  const [step, setStep] = useState(0);
  const [kind, setKind] = useState<Kind>(last.kind);
  const [opts, setOpts] = useState<Options>(last.options);
  const [dir, setDir] = useState<string | null>(null);
  const [rows, setRows] = useState<Record<string, Row>>({});
  /** `undefined`/`null` = todos os campos. */
  const [fields, setFields] = useState<Record<string, string[] | null | undefined>>({});
  const [columns, setColumns] = useState<Record<string, ColumnsState>>({});
  const [fieldTable, setFieldTable] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [menu, setMenu] = useState<"select" | "profiles" | null>(null);
  const [profiles, setProfiles] = useState<ExportProfile[]>([]);
  const [namePrompt, setNamePrompt] = useState(false);

  const [running, setRunning] = useState(false);
  const [log, setLog] = useState<{ text: string; error?: boolean }[]>([]);
  const [current, setCurrent] = useState<{ table: string; total: number | null; processed: number } | null>(null);
  const [overall, setOverall] = useState({ done: 0, total: 0 });
  const [elapsed, setElapsed] = useState(0);
  const stopRef = useRef(false);
  const exportId = useRef("");
  const startedAt = useRef(0);
  const logRef = useRef<HTMLDivElement>(null);
  const columnsRef = useRef(columns);
  columnsRef.current = columns;

  const selected = useMemo(() => target.tables.filter((tb) => rows[tb.name]?.checked), [target.tables, rows]);
  const scope = target.scope;
  const scopeActive = !!scope && (!!scope.filter?.expr.trim() || scope.orderBy.length > 0 || scope.visibleColumns.length < scope.allColumns.length);

  /* ---------- Inicialização ---------- */
  useEffect(() => {
    let alive = true;
    void defaultExportDir().then((d) => alive && setDir(d));
    void local.getPref<ExportProfile[]>(PROFILES_KEY).then((p) => alive && setProfiles(Array.isArray(p) ? p : [])).catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (dir === null) return;
    setRows(Object.fromEntries(target.tables.map((tb) => {
      const on = tb.name === target.initial;
      return [tb.name, { checked: on, path: on ? defaultPath(dir, tb.name, kind, sep) : "" }];
    })));
    // Do grid: só as colunas visíveis vêm marcadas.
    if (scope && scope.visibleColumns.length < scope.allColumns.length) setFields({ [scope.table]: scope.visibleColumns });
    // Só quando a pasta padrão fica conhecida (uma vez).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dir]);

  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => setElapsed(performance.now() - startedAt.current), 100);
    return () => window.clearInterval(id);
  }, [running]);

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [log]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || running || advancedOpen || namePrompt) return;
      if (menu) setMenu(null);
      else onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [running, advancedOpen, namePrompt, menu, onClose]);

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as Element | null)?.closest?.(".exp-menu-wrap")) setMenu(null);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  /* ---------- Formato e linhas ---------- */
  const changeKind = (k: Kind) => {
    setKind(k);
    setRows((r) => Object.fromEntries(Object.entries(r).map(([name, row]) => [name, { ...row, path: row.path ? swapExtension(row.path, k) : row.path }])));
  };

  const patchRow = (name: string, patch: Partial<Row>) => setRows((r) => ({ ...r, [name]: { ...r[name], ...patch } }));

  const toggleRow = (name: string, checked: boolean) => {
    setRows((r) => {
      const row = r[name];
      const path = checked && !row.path && dir !== null ? defaultPath(dir, name, kind, sep) : row.path;
      return { ...r, [name]: { checked, path } };
    });
  };

  const setAll = (pick: (tb: { name: string; view: boolean }, was: boolean) => boolean) => {
    setRows((r) => Object.fromEntries(target.tables.map((tb) => {
      const row = r[tb.name];
      const checked = pick(tb, row.checked);
      const path = checked && !row.path && dir !== null ? defaultPath(dir, tb.name, kind, sep) : row.path;
      return [tb.name, { checked, path }];
    })));
  };

  const browseFile = async (name: string) => {
    try {
      const f = formatOf(kind);
      const chosen = await chooseExportPath(`${name}.${f.ext[0]}`, f.ext);
      if (chosen) patchRow(name, { path: chosen, checked: true });
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  };

  const chooseFolder = async () => {
    try {
      const folder = await chooseExportFolder();
      if (!folder) return;
      setRows((r) => Object.fromEntries(Object.entries(r).map(([name, row]) => [
        name,
        row.checked ? { ...row, path: row.path ? withFolder(row.path, folder, sep) : defaultPath(folder, name, kind, sep) } : row,
      ])));
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  };

  /* ---------- Campos ---------- */
  const queryColumns = target.query?.columns;
  const loadColumns = useCallback(async (table: string) => {
    if (columnsRef.current[table]) return;
    // Resultado de consulta: as colunas já são conhecidas (as do resultado).
    if (queryColumns && table === target.query?.name) {
      setColumns((c) => ({ ...c, [table]: queryColumns }));
      return;
    }
    setColumns((c) => ({ ...c, [table]: "loading" }));
    try {
      const d = await pg.tableDetail(target.connId, target.database, target.schema, table);
      setColumns((c) => ({ ...c, [table]: d.columns.map((col) => col.name) }));
    } catch (e) {
      setColumns((c) => ({ ...c, [table]: { error: errorText(toAtlasError(e)) } }));
    }
  }, [target.connId, target.database, target.schema, queryColumns, target.query?.name]);

  useEffect(() => {
    if (step !== 2) return;
    const names = selected.map((tb) => tb.name);
    const table = names.includes(fieldTable) ? fieldTable : names[0] ?? "";
    if (table !== fieldTable) setFieldTable(table);
    if (table) void loadColumns(table);
  }, [step, selected, fieldTable, loadColumns]);

  // Consulta com colunas de mesmo nome (ex.: um JOIN com `id` duas vezes): não dá para escolher por nome.
  const forceAll = !!queryColumns && new Set(queryColumns).size !== queryColumns.length;
  const colsState = columns[fieldTable];
  const tableColumns: string[] | null = Array.isArray(colsState) ? colsState : null;
  const columnsError = colsState && typeof colsState === "object" && !Array.isArray(colsState) ? colsState.error : null;
  const allFields = forceAll || fields[fieldTable] == null;
  const setTableFields = (v: string[] | null) => setFields((f) => ({ ...f, [fieldTable]: v }));

  /* ---------- Navegação ---------- */
  /** Primeira etapa (anterior a `upTo`) com algo faltando, ou `null`. */
  const firstProblem = (upTo: number): { step: number; message: string } | null => {
    if (upTo > 1) {
      if (selected.length === 0) return { step: 1, message: t("exp.needTable") };
      if (selected.some((tb) => !rows[tb.name].path.trim())) return { step: 1, message: t("exp.needPath") };
    }
    if (upTo > 2) {
      const empty = selected.find((tb) => Array.isArray(fields[tb.name]) && (fields[tb.name] as string[]).length === 0);
      if (empty) return { step: 2, message: t("exp.needFields", { table: empty.name }) };
    }
    return null;
  };

  const goTo = (to: number) => {
    if (to > step) {
      const problem = firstProblem(to);
      if (problem) {
        setStep(problem.step);
        setMessage(problem.message);
        return;
      }
    }
    setMessage(null);
    setMenu(null);
    setStep(to);
  };

  /* ---------- Perfis ---------- */
  const persistProfiles = (next: ExportProfile[]) => {
    setProfiles(next);
    void local.setPref(PROFILES_KEY, next).catch(() => undefined);
  };

  const saveProfile = (name: string) => {
    const profile: ExportProfile = {
      name, kind, options: opts,
      items: selected.map((tb) => ({ table: tb.name, path: rows[tb.name].path, fields: Array.isArray(fields[tb.name]) ? (fields[tb.name] as string[]) : null })),
    };
    persistProfiles([...profiles.filter((p) => p.name !== name), profile].sort((a, b) => a.name.localeCompare(b.name)));
    setNamePrompt(false);
    setLog((l) => [...l, { text: t("exp.profileSaved", { name }) }]);
  };

  const applyProfile = (p: ExportProfile) => {
    const known = new Set(target.tables.map((tb) => tb.name));
    setKind(FORMATS.some((f) => f.kind === p.kind) ? p.kind : "csv");
    setOpts(mergeOptions(p.options));
    setRows((r) => {
      const next = Object.fromEntries(Object.entries(r).map(([name, row]) => [name, { ...row, checked: false }]));
      for (const it of p.items) if (known.has(it.table)) next[it.table] = { checked: true, path: it.path };
      return next;
    });
    setFields(Object.fromEntries(p.items.filter((it) => known.has(it.table)).map((it) => [it.table, it.fields])));
    setLog((l) => [...l, { text: t("exp.profileLoaded", { name: p.name }) }]);
  };

  /* ---------- Execução ---------- */
  /** Consulta com colunas repetidas: sempre todos os campos. */
  const forceAllFor = (table: string) => !!queryColumns && table === target.query?.name && new Set(queryColumns).size !== queryColumns.length;

  const start = async () => {
    if (running) return;
    const problem = firstProblem(STEPS);
    if (problem) {
      setStep(problem.step);
      setMessage(problem.message);
      return;
    }
    saveLast(kind, opts);
    const items = selected;
    const add = (text: string, error = false) => setLog((l) => [...l, { text, error }]);
    stopRef.current = false;
    startedAt.current = performance.now();
    setElapsed(0);
    setRunning(true);
    setLog([{ text: t("exp.log.start") }, { text: t("exp.log.format", { format: t(formatOf(kind).label) }) }]);
    setOverall({ done: 0, total: items.length });
    let done = 0;
    let failed = false;
    for (const tb of items) {
      if (stopRef.current) break;
      const path = rows[tb.name].path.trim();
      const tableScope = scope?.table === tb.name ? scope : null;
      const isQuery = !!target.query && tb.name === target.query.name;
      setCurrent({ table: tb.name, total: null, processed: 0 });
      add(t("exp.log.table", { table: tb.name }));
      add(t("exp.log.to", { path }));
      // Consulta: contar exigiria executá-la duas vezes, então o total fica em branco.
      if (!isQuery) {
        void pg.tableCount(target.connId, target.database, target.schema, tb.name, tableScope?.filter ?? null)
          .then((n) => setCurrent((c) => (c && c.table === tb.name ? { ...c, total: n } : c)))
          .catch(() => undefined);
      }
      const id = crypto.randomUUID();
      exportId.current = id;
      const unlisten = await onExportProgress(id, (n) => setCurrent((c) => (c && c.table === tb.name ? { ...c, processed: n } : c)));
      try {
        const result = await pg.export({
          exportId: id, sessionId: target.connId, database: target.database, schema: target.schema,
          table: isQuery && kind === "sql" && opts.sql.tableName.trim() ? opts.sql.tableName.trim() : tb.name,
          columns: forceAllFor(tb.name) ? [] : fields[tb.name] ?? [], orderBy: tableScope?.orderBy ?? [], filter: tableScope?.filter ?? null,
          query: isQuery && target.query ? { sql: target.query.sql, schema: target.query.schema ?? null } : undefined,
          format: buildFormat(kind, opts, tb.name), path,
        });
        setCurrent({ table: tb.name, total: result.rows, processed: result.rows });
        add(t("exp.log.rows", { n: result.rows.toLocaleString(), size: formatBytes(result.bytes), s: (result.elapsedMs / 1000).toFixed(1) }));
        done++;
        setOverall({ done, total: items.length });
      } catch (e) {
        const err = toAtlasError(e);
        if (err.kind === "canceled") add(t("exp.log.canceled"));
        else {
          add(t("exp.log.error", { message: errorText(err) }), true);
          failed = true;
        }
        break;
      } finally {
        unlisten();
      }
    }
    if (!failed && !stopRef.current && done === items.length) add(t("exp.log.done"));
    else add(t("exp.log.partial", { done, total: items.length }));
    setElapsed(performance.now() - startedAt.current);
    setRunning(false);
  };

  const stop = () => {
    stopRef.current = true;
    void pg.exportCancel(exportId.current).catch(() => undefined);
  };

  /* ---------- Menus ---------- */
  const selectMenu: MenuEntry[] = [
    { kind: "item", text: t("exp.selectAll"), run: () => setAll(() => true) },
    { kind: "item", text: t("exp.unselectAll"), run: () => setAll(() => false) },
    { kind: "item", text: t("exp.invert"), run: () => setAll((_, was) => !was) },
    { kind: "item", text: t("exp.onlyTables"), run: () => setAll((tb) => !tb.view) },
    { kind: "item", text: t("exp.onlyViews"), run: () => setAll((tb) => tb.view) },
    { kind: "sep" },
    { kind: "item", text: t("exp.setFolder"), run: () => void chooseFolder() },
  ];
  const profileMenu: MenuEntry[] = profiles.length
    ? [
        ...profiles.map((p): MenuEntry => ({ kind: "item", text: p.name, run: () => applyProfile(p) })),
        { kind: "sep" },
        ...profiles.map((p): MenuEntry => ({ kind: "item", text: t("exp.profileDelete", { name: p.name }), run: () => persistProfiles(profiles.filter((x) => x.name !== p.name)) })),
      ]
    : [{ kind: "item", text: t("exp.profileNone") }];

  /* ---------- Render ---------- */
  const total = current?.total ?? null;
  const fraction = overall.total === 0 ? 0 : (overall.done + (running && current && total ? Math.min(1, current.processed / Math.max(1, total)) : 0)) / overall.total;

  const formatStep = (
    <div className="exp-formats" role="radiogroup" aria-label={t("exp.formatLabel")}>
      <div className="exp-formats-title">{t("exp.formatLabel")}</div>
      {FORMATS.map((f) => (
        <label key={f.kind} className={`exp-format${kind === f.kind ? " is-active" : ""}`}>
          <input type="radio" name="exp-format" checked={kind === f.kind} onChange={() => changeKind(f.kind)} />
          <SmallIcon name={f.icon} size={18} />
          <span>{t(f.label)}</span>
        </label>
      ))}
      {UNAVAILABLE_FORMATS.map((f) => (
        <label key={f.label} className="exp-format is-disabled" title={t("exp.unavailable")}>
          <input type="radio" name="exp-format" disabled />
          <SmallIcon name={f.icon} size={18} />
          <span>{t(f.label)}</span>
        </label>
      ))}
    </div>
  );

  const allChecked = target.tables.length > 0 && target.tables.every((tb) => rows[tb.name]?.checked);
  const someChecked = target.tables.some((tb) => rows[tb.name]?.checked);
  const filesStep = (
    <>
      <div className="exp-files">
        <table className="exp-files-table">
          <colgroup><col style={{ width: 28 }} /><col style={{ width: 24 }} /><col style={{ width: "34%" }} /><col /></colgroup>
          <thead>
            <tr>
              <th>
                <input
                  type="checkbox"
                  aria-label={t("exp.selectAll")}
                  checked={allChecked}
                  ref={(el) => { if (el) el.indeterminate = !allChecked && someChecked; }}
                  onChange={(e) => setAll(() => e.target.checked)}
                />
              </th>
              <th />
              <th>{t("exp.colSource")}</th>
              <th>{t("exp.colExportTo")}</th>
            </tr>
          </thead>
          <tbody>
            {target.tables.map((tb) => {
              const row = rows[tb.name] ?? { checked: false, path: "" };
              return (
                <tr key={tb.name} className={row.checked ? "is-checked" : undefined} onDoubleClick={() => row.checked && void browseFile(tb.name)}>
                  <td><input type="checkbox" checked={row.checked} onChange={(e) => toggleRow(tb.name, e.target.checked)} aria-label={tb.name} /></td>
                  <td><SmallIcon name={tb.query ? "query" : tb.view ? "view" : "table"} /></td>
                  <td className="exp-name" title={tb.name}>{tb.name}</td>
                  <td className="exp-path-cell">
                    {row.checked && (
                      <>
                        <input
                          className="exp-path-input"
                          value={row.path}
                          spellCheck={false}
                          placeholder={t("exp.pickFile")}
                          title={row.path}
                          onChange={(e) => patchRow(tb.name, { path: e.target.value })}
                        />
                        <button type="button" className="exp-path-browse" aria-label={t("exp.browse")} onClick={() => void browseFile(tb.name)}>…</button>
                      </>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="exp-files-actions">
        <div className="exp-menu-wrap split-button">
          <button type="button" className="btn" onClick={() => setAll(() => true)}>{t("exp.selectAll")}</button>
          <button type="button" className="btn exp-arrow" aria-label={t("exp.selectAll")} onClick={() => setMenu((m) => (m === "select" ? null : "select"))}>
            <SmallIcon name="chevronDown" size={10} />
          </button>
          {menu === "select" && <PopupMenu items={selectMenu} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%" }} />}
        </div>
        <span className="spacer" />
        <button type="button" className="btn" onClick={() => setAdvancedOpen(true)}>{t("exp.advanced")}</button>
      </div>
      {scopeActive && scope && <div className="muted exp-note">{t("exp.scopeNote", { table: scope.table })}</div>}
      {target.query && <div className="muted exp-note">{t("exp.queryNote")}</div>}
    </>
  );

  const fieldsStep = (
    <>
      <div className="exp-field-row">
        <label>{t("exp.sourceTable")}</label>
        <select className="field" value={fieldTable} onChange={(e) => setFieldTable(e.target.value)}>
          {selected.map((tb) => <option key={tb.name} value={tb.name}>{tb.name}</option>)}
        </select>
      </div>
      <div className="exp-fields-title">{t("exp.availableFields")}</div>
      <div className={`exp-fields-list${allFields ? " is-disabled" : ""}`}>
        {columnsError ? (
          <div className="msg-error">{columnsError}</div>
        ) : !tableColumns ? (
          <div className="muted exp-loading"><span className="spinner" />{t("exp.loadingFields")}</div>
        ) : (
          tableColumns.map((name) => {
            const list = fields[fieldTable];
            const checked = allFields || (Array.isArray(list) && list.includes(name));
            return (
              <label key={name} className="check-row">
                <input
                  type="checkbox"
                  checked={checked}
                  disabled={allFields}
                  onChange={(e) => {
                    const base = Array.isArray(list) ? list : tableColumns;
                    setTableFields(tableColumns.filter((c) => (c === name ? e.target.checked : base.includes(c))));
                  }}
                />
                <span>{name}</span>
              </label>
            );
          })
        )}
      </div>
      <div className="exp-fields-actions">
        <button type="button" className="btn" disabled={allFields || !tableColumns} onClick={() => setTableFields(tableColumns)}>{t("exp.selectAll")}</button>
        <button type="button" className="btn" disabled={allFields || !tableColumns} onClick={() => setTableFields([])}>{t("exp.unselectAll")}</button>
        <label className="check-row">
          <input type="checkbox" checked={allFields} disabled={forceAll} onChange={(e) => setTableFields(e.target.checked ? null : tableColumns ?? [])} />
          <span>{t("exp.allFields")}</span>
        </label>
      </div>
      {forceAll && <div className="muted exp-note">{t("exp.dupColumns")}</div>}
    </>
  );

  const summaryStep = (
    <>
      <dl className="exp-summary">
        <dt>{t("exp.sourceTable")}</dt><dd>{current?.table ?? "—"}{overall.total > 1 ? ` (${overall.done + (running ? 1 : 0)}/${overall.total})` : ""}</dd>
        <dt>{t("exp.total")}</dt><dd>{total === null ? "—" : total.toLocaleString()}</dd>
        <dt>{t("exp.processed")}</dt><dd>{current ? current.processed.toLocaleString() : "—"}</dd>
        <dt>{t("exp.time")}</dt><dd>{formatClock(elapsed)}</dd>
      </dl>
      <div className="exp-log" ref={logRef} role="log" aria-live="polite">
        {log.map((line, i) => <div key={i} className={line.error ? "is-error" : undefined}>{line.text}</div>)}
      </div>
      <div className="exp-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(fraction * 100)}>
        <div className="exp-progress-bar" style={{ width: `${Math.round(fraction * 100)}%` }} />
      </div>
    </>
  );

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget && !running && step !== LAST) onClose(); }}>
      <div className="modal exp-wizard" role="dialog" aria-label={t("exp.title")}>
        <div className="modal-header">{running ? `${Math.round(fraction * 100)}% - ` : ""}{t("exp.title")}</div>
        <div className="exp-heading">{t(HEADINGS[step])}</div>
        <div className="exp-body">
          {step === 0 && formatStep}
          {step === 1 && filesStep}
          {step === 2 && fieldsStep}
          {step === 3 && summaryStep}
          {message && <div className="msg-error exp-message" role="alert">{message}</div>}
        </div>
        <div className="exp-footer">
          <div className="exp-menu-wrap split-button">
            <button type="button" className="btn" disabled={step !== LAST || running || selected.length === 0} onClick={() => setNamePrompt(true)}>{t("exp.saveProfile")}</button>
            <button type="button" className="btn" disabled={step !== LAST || running} onClick={() => setMenu((m) => (m === "profiles" ? null : "profiles"))}>
              {t("exp.openProfile")}
            </button>
            {menu === "profiles" && <PopupMenu items={profileMenu} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%" }} />}
          </div>
          <span className="spacer" />
          <button type="button" className="btn" disabled={step === 0 || running} onClick={() => goTo(0)}>{t("exp.first")}</button>
          <button type="button" className="btn" disabled={step === 0 || running} onClick={() => goTo(step - 1)}>{t("exp.back")}</button>
          {step === LAST ? (
            running
              ? <button type="button" className="btn btn-primary" onClick={stop}>{t("exp.stop")}</button>
              : <button type="button" className="btn btn-primary" onClick={() => void start()}>{t("exp.start")}</button>
          ) : (
            <button type="button" className="btn btn-primary" disabled={dir === null} onClick={() => goTo(step + 1)}>{t("exp.next")}</button>
          )}
          <button type="button" className="btn" disabled={step === LAST || running} onClick={() => goTo(LAST)}>{t("exp.last")}</button>
          <button type="button" className="btn" disabled={running} onClick={onClose}>{t(step === LAST ? "common.close" : "common.cancel")}</button>
        </div>
      </div>

      {advancedOpen && <ExportOptionsDialog isQuery={!!target.query} kind={kind} options={opts} onChange={setOpts} onClose={() => setAdvancedOpen(false)} />}
      {namePrompt && (
        <MessageDialog
          title={t("exp.saveProfile")}
          prompt={{ label: t("exp.profileName"), initial: "" }}
          okLabel={t("exp.saveProfile")}
          onOk={(name) => { if (name) saveProfile(name); }}
          onCancel={() => setNamePrompt(false)}
        />
      )}
    </div>
  );
}
