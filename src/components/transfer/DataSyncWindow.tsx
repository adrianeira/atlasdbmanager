import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MenuEntry } from "../../commands";
import {
  errorText, local, onSyncDeployProgress, onSyncLog, onSyncProgress, pg, toAtlasError,
  type DataTransferParams, type SyncRow, type SyncSelection, type SyncTableSummary, type TransferFieldMap,
} from "../../backend";
import { useT } from "../../i18n";
import { SmallIcon } from "../../icons";
import { Dropdown } from "../Dropdown";
import { MessageDialog } from "../MessageDialog";
import { PopupMenu } from "../PopupMenu";
import { SqlEditor } from "../SqlEditor";
import { ToolWindow } from "../ToolWindow";
import { useEndpoints } from "./endpoints";
import { formatClock } from "./transferModel";

type Params = DataTransferParams & { title: string };
type Pair = TransferFieldMap;

/** Uma linha da grade de tabelas (etapa 2). `key`/`fields` ausentes = "Default". */
interface TableRow {
  source: string;
  target: string;
  checked: boolean;
  key?: Pair[];
  fields?: Pair[];
}

/** Resultado da comparação de uma tabela e o que o usuário marcou dele. */
interface ResultTable extends SyncTableSummary {
  checked: boolean;
  insertOn: boolean;
  updateOn: boolean;
  deleteOn: boolean;
  excluded: Set<number>;
}

interface Options {
  insert: boolean;
  update: boolean;
  delete: boolean;
}

interface SyncProfile {
  name: string;
  source: { connId: string; database: string; schema: string };
  target: { connId: string; database: string; schema: string };
  rows: TableRow[];
  mode: "preview" | "deploy";
  options: Options;
}

const PROFILES_KEY = "datasync.profiles";
const PAGE = 500;
/** Linhas por tela de detalhe e nomes dos filtros. */
const FILTERS = ["difference", "insert", "update", "delete"] as const;
type Filter = (typeof FILTERS)[number];

export function DataSyncWindow() {
  const params = (window as unknown as { __ATLAS_DS__: Params }).__ATLAS_DS__;
  return (
    <ToolWindow title={params.title}>
      <Wizard params={params} />
    </ToolWindow>
  );
}

function Wizard({ params }: { params: Params }) {
  const t = useT();
  const [step, setStep] = useState(0);
  const [rows, setRows] = useState<TableRow[] | null>(null);
  const [rowsError, setRowsError] = useState<string | null>(null);
  const [targetTables, setTargetTables] = useState<string[]>([]);
  const [mode, setMode] = useState<"preview" | "deploy">("preview");
  const [options, setOptions] = useState<Options>({ insert: true, update: true, delete: true });
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [menu, setMenu] = useState<"save" | "load" | "deploy" | null>(null);
  const [profiles, setProfiles] = useState<SyncProfile[]>([]);
  const [namePrompt, setNamePrompt] = useState(false);
  const [mapEdit, setMapEdit] = useState<{ table: string; kind: "key" | "fields"; pairs: Pair[]; sourceCols: string[]; targetCols: string[] } | null>(null);

  const [comparing, setComparing] = useState<{ text: string; fraction: number } | null>(null);
  const [result, setResult] = useState<ResultTable[] | null>(null);
  const [showSame, setShowSame] = useState(true);
  const [focus, setFocus] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("difference");
  const [page, setPage] = useState<{ rows: SyncRow[]; total: number } | null>(null);
  const [cell, setCell] = useState<{ left: string; right: string }>({ left: "", right: "" });
  const compareId = useRef("");

  const [tab, setTab] = useState<"script" | "log">("script");
  const [script, setScript] = useState<{ text: string; statements: number; truncated: boolean } | null>(null);
  const [deployOpts, setDeployOpts] = useState({ transaction: true, continueOnError: false });
  const [running, setRunning] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [deployResult, setDeployResult] = useState<{ errors: number; rolledBack: boolean } | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const deployId = useRef("");
  const startedAt = useRef(0);
  const logRef = useRef<HTMLDivElement>(null);

  const {
    source, target, sourceConn, targetConn, message, setMessage, header, connectStep, passwordDialog, endpointProblem, setSource, setTarget,
  } = useEndpoints(params, { allowFile: false, onSourceChange: () => { setRows(null); setResult(null); } });

  /* ---------- Tabelas do schema de origem e do destino ---------- */
  useEffect(() => {
    if (step < 1 || !sourceConn?.open || !targetConn?.open || !source.database || !target.database || !source.schema || !target.schema) return;
    let alive = true;
    setRowsError(null);
    Promise.all([pg.listObjects(source.connId, source.database, source.schema), pg.listObjects(target.connId, target.database, target.schema)])
      .then(([s, d]) => {
        if (!alive) return;
        const dst = d.tables.map((x) => x.name);
        setTargetTables(dst);
        setRows((old) => s.tables.map((x) => old?.find((o) => o.source === x.name) ?? { source: x.name, target: dst.includes(x.name) ? x.name : "", checked: dst.includes(x.name) }));
      })
      .catch((e) => alive && setRowsError(errorText(toAtlasError(e))));
    return () => { alive = false; };
    // Relê ao voltar à etapa das tabelas ou ao mudar a origem/destino.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step === 1, source.connId, source.database, source.schema, target.connId, target.database, target.schema]);

  useEffect(() => { void local.getPref<SyncProfile[]>(PROFILES_KEY).then((p) => setProfiles(Array.isArray(p) ? p : [])).catch(() => undefined); }, []);

  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => setElapsed(performance.now() - startedAt.current), 100);
    return () => window.clearInterval(id);
  }, [running]);

  useEffect(() => { if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight; }, [log]);

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => { if (!(e.target as Element | null)?.closest?.(".dt-menu-wrap")) setMenu(null); };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  // A comparação guardada no app é descartada ao fechar a janela.
  useEffect(() => () => { if (compareId.current) void pg.syncForget(compareId.current).catch(() => undefined); }, []);

  /* ---------- Mapeamento de chave e de campos ---------- */
  const patchRow = (source: string, p: Partial<TableRow>) => setRows((r) => r?.map((x) => (x.source === source ? { ...x, ...p } : x)) ?? r);

  const openMapping = async (row: TableRow, kind: "key" | "fields") => {
    if (!row.target) return;
    try {
      const [sd, td] = await Promise.all([
        pg.tableDetail(source.connId, source.database, source.schema, row.source),
        pg.tableDetail(target.connId, target.database, target.schema, row.target),
      ]);
      const sourceCols = sd.columns.map((c) => c.name);
      const targetCols = td.columns.map((c) => c.name);
      const current = kind === "key" ? row.key : row.fields;
      const defaults: Pair[] = kind === "key"
        ? sd.columns.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0)).map((c) => ({ source: c.name, target: c.name }))
        : sd.columns.filter((c) => c.generated !== "s" && targetCols.includes(c.name)).map((c) => ({ source: c.name, target: c.name }));
      setMapEdit({ table: row.source, kind, pairs: current ?? defaults, sourceCols, targetCols });
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  };

  /* ---------- Comparação ---------- */
  const compare = async (thenDeploy = false) => {
    const chosen = (rows ?? []).filter((r) => r.checked && r.target);
    if (chosen.length === 0) { setMessage(t("ds.needTables")); return; }
    setMessage(null);
    if (compareId.current) void pg.syncForget(compareId.current).catch(() => undefined);
    const id = crypto.randomUUID();
    compareId.current = id;
    setComparing({ text: "", fraction: 0 });
    const unlisten = await onSyncProgress(id, (text, fraction) => setComparing({ text, fraction }));
    try {
      const summary = await pg.syncCompare({
        compareId: id, sourceConn: source.connId, sourceDatabase: source.database, targetConn: target.connId, targetDatabase: target.database,
        spec: { sourceSchema: source.schema, targetSchema: target.schema, tables: chosen.map((r) => ({ source: r.source, target: r.target, key: r.key, fields: r.fields })) },
      });
      const next: ResultTable[] = summary.tables.map((x) => ({
        ...x, checked: !x.message && x.insert + x.update + x.delete > 0,
        insertOn: options.insert && x.insert > 0, updateOn: options.update && x.update > 0, deleteOn: options.delete && x.delete > 0, excluded: new Set<number>(),
      }));
      setResult(next);
      setFocus(next.find((x) => x.checked)?.source ?? null);
      setFilter("difference");
      setScript(null);
      setStep(2);
      if (thenDeploy) await prepareScript(next, true);
    } catch (e) {
      const err = toAtlasError(e);
      if (err.kind !== "canceled") setMessage(errorText(err));
    } finally {
      unlisten();
      setComparing(null);
    }
  };

  const stopCompare = () => void pg.exportCancel(compareId.current).catch(() => undefined);

  const selectionOf = (list: ResultTable[]): SyncSelection[] =>
    list.filter((x) => x.checked && !x.message).map((x) => ({ table: x.source, insert: x.insertOn, update: x.updateOn, delete: x.deleteOn, excluded: [...x.excluded] }));

  /* ---------- Detalhe das diferenças de uma tabela ---------- */
  const focusTable = result?.find((x) => x.source === focus) ?? null;
  const loadRows = useCallback(async (offset: number) => {
    if (!focus || !compareId.current) return;
    try {
      const p = await pg.syncRows(compareId.current, focus, filter === "difference" ? null : filter, offset, PAGE);
      setPage((old) => (offset === 0 || !old ? p : { total: p.total, rows: [...old.rows, ...p.rows] }));
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  }, [focus, filter, setMessage]);
  useEffect(() => { setPage(null); setCell({ left: "", right: "" }); if (step === 2 && focus) void loadRows(0); }, [step, focus, filter, result, loadRows]);

  const patchResult = (source: string, p: (x: ResultTable) => Partial<ResultTable>) =>
    setResult((r) => r?.map((x) => (x.source === source ? { ...x, ...p(x) } : x)) ?? r);

  const toggleRow = (id: number) => {
    if (!focusTable) return;
    patchResult(focusTable.source, (x) => { const e = new Set(x.excluded); if (e.has(id)) e.delete(id); else e.add(id); return { excluded: e }; });
  };

  /* ---------- Script e implantação ---------- */
  const prepareScript = async (list: ResultTable[], deployNow = false) => {
    const sel = selectionOf(list);
    try {
      const s = await pg.syncScript(compareId.current, sel);
      if (s.statements === 0) { setMessage(t("ds.nothing")); return; }
      setScript({ text: s.text, statements: s.statements, truncated: s.truncated });
      setTab(deployNow ? "log" : "script");
      setStep(3);
      if (deployNow) await deploy(list);
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  };

  const deploy = async (list: ResultTable[] = result ?? []) => {
    if (running) return;
    const id = crypto.randomUUID();
    deployId.current = id;
    setTab("log");
    setLog([]);
    setProgress({ done: 0, total: script?.statements ?? 0 });
    setDeployResult(null);
    setElapsed(0);
    startedAt.current = performance.now();
    setRunning(true);
    const unlistenLog = await onSyncLog(id, (line) => setLog((l) => [...l, line]));
    const unlistenProgress = await onSyncDeployProgress(id, (done, total) => setProgress({ done, total }));
    try {
      const r = await pg.syncDeploy({
        deployId: id, compareId: compareId.current, targetConn: target.connId, targetDatabase: target.database,
        selection: selectionOf(list), transaction: deployOpts.transaction, continueOnError: deployOpts.continueOnError,
      });
      setProgress({ done: r.done + r.errors, total: r.total });
      setDeployResult({ errors: r.errors, rolledBack: r.rolledBack });
    } catch (e) {
      const err = toAtlasError(e);
      setLog((l) => [...l, err.kind === "canceled" ? `--${t("dt.log.canceled")}--` : `[ERR] ${errorText(err)}`]);
      setDeployResult({ errors: err.kind === "canceled" ? 0 : 1, rolledBack: true });
    } finally {
      unlistenLog();
      unlistenProgress();
    }
    setElapsed(performance.now() - startedAt.current);
    setRunning(false);
  };

  const copyScript = async () => {
    try {
      const full = await pg.syncScriptFull(compareId.current, selectionOf(result ?? []));
      await navigator.clipboard.writeText(full);
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  };

  /* ---------- Navegação ---------- */
  const goNext = async () => {
    setMenu(null);
    if (step === 0) {
      const p = endpointProblem();
      if (p) { setMessage(p); return; }
      setMessage(null);
      setStep(1);
    } else if (step === 2) {
      await prepareScript(result ?? []);
    }
  };

  const pct = progress.total ? (progress.done / progress.total) * 100 : running ? 0 : 100;
  useEffect(() => {
    void getCurrentWindow().setTitle(step === 3 && tab === "log" ? `${Math.round(pct)}% - ${t("ds.title")}` : `* ${t("ds.title")}`).catch(() => undefined);
  }, [step, tab, pct, t]);

  /* ---------- Perfis ---------- */
  const persist = (next: SyncProfile[]) => { setProfiles(next); void local.setPref(PROFILES_KEY, next).catch(() => undefined); };
  const saveProfile = (name: string) => {
    const p: SyncProfile = { name, source: { ...source }, target: { ...target }, rows: rows ?? [], mode, options };
    persist([...profiles.filter((x) => x.name !== name), p].sort((a, b) => a.name.localeCompare(b.name)));
    setNamePrompt(false);
  };
  const loadProfile = (p: SyncProfile) => {
    setSource(p.source);
    setTarget(p.target);
    setRows(p.rows);
    setMode(p.mode);
    setOptions(p.options);
    setResult(null);
    setStep(0);
  };

  /* ---------- Peças ---------- */
  const saveItems: MenuEntry[] = [
    { kind: "item", text: t("dt.saveAs"), run: () => setNamePrompt(true) },
    ...(profiles.length ? [{ kind: "sep" } as MenuEntry, ...profiles.map((p): MenuEntry => ({ kind: "item", text: t("exp.profileDelete", { name: p.name }), run: () => persist(profiles.filter((x) => x.name !== p.name)) }))] : []),
  ];
  const loadItems: MenuEntry[] = profiles.length ? profiles.map((p): MenuEntry => ({ kind: "item", text: p.name, run: () => loadProfile(p) })) : [{ kind: "item", text: t("exp.profileNone") }];
  const deployItems: MenuEntry[] = [
    { kind: "item", text: t("ds.opt.transaction"), checked: deployOpts.transaction, run: () => setDeployOpts((o) => ({ ...o, transaction: !o.transaction })) },
    { kind: "item", text: t("ds.opt.continue"), checked: deployOpts.continueOnError, run: () => setDeployOpts((o) => ({ ...o, continueOnError: !o.continueOnError })) },
  ];

  const tablesStep = (
    <div className="ds-tables">
      <div className="ds-grid-wrap">
        {rowsError && <div className="msg-error">{rowsError}</div>}
        {rows === null && !rowsError && <div className="muted dt-loading"><span className="spinner" />{t("dt.reading")}</div>}
        {rows && (
          <table className="ds-grid">
            <colgroup><col style={{ width: 220 }} /><col style={{ width: 70 }} /><col style={{ width: 220 }} /><col style={{ width: 165 }} /><col style={{ width: 165 }} /><col /></colgroup>
            <thead><tr><th>{t("ds.srcTable")}</th><th /><th>{t("ds.tgtTable")}</th><th>{t("ds.keyMapping")}</th><th>{t("ds.fieldMapping")}</th><th /></tr></thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.source}>
                  <td className="ds-name" title={r.source}>{r.source}</td>
                  <td className="ds-check"><input type="checkbox" checked={r.checked} disabled={!r.target} aria-label={r.source} onChange={(e) => patchRow(r.source, { checked: e.target.checked })} /></td>
                  <td>
                    <select className="field ds-cell" value={r.target} onChange={(e) => patchRow(r.source, { target: e.target.value, checked: !!e.target.value, key: undefined, fields: undefined })}>
                      <option value="">{t("ds.noTarget")}</option>
                      {targetTables.map((n) => <option key={n} value={n}>{n}</option>)}
                    </select>
                  </td>
                  <td><span className="ds-map"><span>{r.key ? t("ds.custom") : t("ds.default")}</span><button type="button" className="ds-dots" disabled={!r.target} aria-label={t("ds.keyMapping")} onClick={() => void openMapping(r, "key")}>…</button></span></td>
                  <td><span className="ds-map"><span>{r.fields ? t("ds.custom") : t("ds.default")}</span><button type="button" className="ds-dots" disabled={!r.target} aria-label={t("ds.fieldMapping")} onClick={() => void openMapping(r, "fields")}>…</button></span></td>
                  <td />
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      <div className="ds-modes">
        <label><input type="radio" name="ds-mode" checked={mode === "preview"} onChange={() => setMode("preview")} />{t("ds.modePreview")}</label>
        <label><input type="radio" name="ds-mode" checked={mode === "deploy"} onChange={() => setMode("deploy")} />{t("ds.modeDeploy")}</label>
      </div>
    </div>
  );

  const shown = (result ?? []).filter((x) => showSame || x.message || x.insert + x.update + x.delete > 0);
  const ordered = [...shown].sort((a, b) => Number(b.checked || !!(b.insert + b.update + b.delete)) - Number(a.checked || !!(a.insert + a.update + a.delete)));
  const catCell = (x: ResultTable, count: number, on: boolean, key: "insertOn" | "updateOn" | "deleteOn") => (
    <td className="ds-count"><label><input type="checkbox" disabled={count === 0 || !!x.message} checked={on} onChange={(e) => patchResult(x.source, () => ({ [key]: e.target.checked }))} />{count.toLocaleString()}</label></td>
  );

  const cellValue = (v: string | null | undefined) => (v === null ? "(null)" : v ?? "");
  const resultStep = (
    <div className="ds-result">
      <label className="check-row ds-show"><input type="checkbox" checked={showSame} onChange={(e) => setShowSame(e.target.checked)} /><span>{t("ds.showSame")}</span></label>
      <div className="ds-grid-wrap ds-summary">
        <table className="ds-grid">
          <thead><tr><th style={{ width: 30 }} /><th>{t("ds.srcTable")}</th><th>{t("ds.tgtTable")}</th><th>{t("ds.colInsert")}</th><th>{t("ds.colUpdate")}</th><th>{t("ds.colDelete")}</th><th>{t("ds.colSame")}</th><th>{t("ds.colMessage")}</th></tr></thead>
          <tbody>
            {ordered.map((x) => {
              const diff = x.insert + x.update + x.delete > 0 && !x.message;
              return (
                <tr key={x.source} className={`${focus === x.source ? "is-focus" : ""}${diff ? "" : " is-muted"}`} onClick={() => setFocus(x.source)}>
                  <td className="ds-check"><input type="checkbox" disabled={!diff} checked={x.checked && diff} aria-label={x.source} onChange={(e) => patchResult(x.source, () => ({ checked: e.target.checked }))} onClick={(e) => e.stopPropagation()} /></td>
                  <td className="ds-name">{x.source}</td>
                  <td className="ds-name">{x.target}</td>
                  {catCell(x, x.insert, x.insertOn, "insertOn")}
                  {catCell(x, x.update, x.updateOn, "updateOn")}
                  {catCell(x, x.delete, x.deleteOn, "deleteOn")}
                  <td className="ds-count"><label><input type="checkbox" disabled checked={false} readOnly />{x.same.toLocaleString()}</label></td>
                  <td className="ds-msg">{x.message ?? ""}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {focusTable && !focusTable.message && focusTable.insert + focusTable.update + focusTable.delete > 0 && (
        <div className="ds-detail">
          <Dropdown className="ds-filter" value={filter} items={FILTERS.map((f) => ({ value: f, label: t(`ds.filter.${f}`) }))} onChange={(v) => setFilter(v as Filter)} />
          <div className="ds-panes">
            {(["source", "target"] as const).map((side) => (
              <div key={side} className="ds-pane">
                <div className="ds-pane-title"><SmallIcon name="table" /> {side === "source" ? focusTable.source : focusTable.target}</div>
                <div className="ds-rows">
                  <table className="ds-grid ds-data">
                    <thead><tr>{side === "source" && <th style={{ width: 26 }} />}{(side === "source" ? focusTable.sourceFields : focusTable.targetFields).map((f) => <th key={f}>{f}</th>)}</tr></thead>
                    <tbody>
                      {(page?.rows ?? []).map((r) => {
                        const values = side === "source" ? r.source : r.target;
                        return (
                          <tr key={r.id} className={`ds-${r.kind}`}>
                            {side === "source" && <td className="ds-check"><input type="checkbox" checked={!focusTable.excluded.has(r.id)} onChange={() => toggleRow(r.id)} aria-label={String(r.id)} /></td>}
                            {(values ?? (side === "source" ? focusTable.sourceFields : focusTable.targetFields).map(() => undefined)).map((v, i) => (
                              <td key={i} className={`${r.changed[i] ? "is-changed" : ""}${v === null ? " is-null" : ""}`} title={cellValue(v)}
                                onClick={() => setCell({ left: cellValue(r.source?.[i]), right: cellValue(r.target?.[i]) })}>{cellValue(v)}</td>
                            ))}
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                  {page && page.rows.length < page.total && side === "source" && (
                    <div className="muted ds-more">{t("ds.moreRows", { n: page.rows.length, total: page.total })} <button type="button" className="btn" onClick={() => void loadRows(page.rows.length)}>{t("ds.loadMore")}</button></div>
                  )}
                </div>
                <textarea className="ds-cell-view" readOnly value={side === "source" ? cell.left : cell.right} spellCheck={false} />
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );

  const targetName = targetConn?.name ?? "";
  const scriptTab = (
    <div className="ds-script">
      <div className="ds-script-head">
        <SmallIcon name="database" size={26} />
        <div className="ds-script-target"><strong>{targetName}</strong><span>{target.database}.{target.schema}</span></div>
        <span className="muted">{t("ds.runOn")}</span>
        <span className="spacer" />
        <button type="button" className="ds-link" onClick={() => void copyScript()}>{t("ds.copyScript")}</button>
      </div>
      <div className="ds-script-body">
        <SqlEditor value={script?.text ?? ""} readOnly />
      </div>
      {script?.truncated && <div className="muted ds-note">{t("ds.truncated", { n: 5000 })}</div>}
    </div>
  );

  const logTab = (
    <div className="dt-progress">
      <dl className="dt-stats">
        <dt>{t("ds.stat.progress")}</dt><dd>{progress.done.toLocaleString()}/{progress.total.toLocaleString()} ({pct.toFixed(1).replace(".", ",")}%)</dd>
        <dt>{t("ds.stat.success")}</dt><dd>{deployResult ? (deployResult.errors === 0 && !deployResult.rolledBack ? 1 : 0) : 0}</dd>
        <dt>{t("ds.stat.errors")}</dt><dd>{deployResult?.errors ?? 0}</dd>
        <dt>{t("exp.time").replace(":", "")}:</dt><dd>{formatClock(elapsed)}</dd>
      </dl>
      <div className="exp-log dt-log" ref={logRef} role="log" aria-live="polite">
        {log.map((l, i) => <div key={i} className={l.startsWith("[ERR]") ? "is-error" : undefined}>{l}</div>)}
      </div>
    </div>
  );

  const footerCommon = (
    <>
      <div className="dt-menu-wrap split-button">
        <button type="button" className="btn dt-wide" disabled={running} onClick={() => setNamePrompt(true)}>{t("exp.saveProfile")}</button>
        <button type="button" className="btn dt-arrow-btn" aria-label={t("exp.saveProfile")} disabled={running} onClick={() => setMenu((m) => (m === "save" ? null : "save"))}><SmallIcon name="chevronDown" size={10} /></button>
        {menu === "save" && <PopupMenu items={saveItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%" }} />}
      </div>
      <div className="dt-menu-wrap">
        <button type="button" className="btn dt-wide" disabled={running} onClick={() => setMenu((m) => (m === "load" ? null : "load"))}>{t("dt.loadProfile")}</button>
        {menu === "load" && <PopupMenu items={loadItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%", left: 0 }} />}
      </div>
    </>
  );

  return (
    <div className="dt-window">
      {step === 3 && (
        <div className="ds-tabs" role="tablist">
          <button type="button" role="tab" aria-selected={tab === "script"} className={`ds-tab${tab === "script" ? " is-active" : ""}`} onClick={() => setTab("script")}>{t("ds.tab.script")}</button>
          <button type="button" role="tab" aria-selected={tab === "log"} className={`ds-tab${tab === "log" ? " is-active" : ""}`} onClick={() => setTab("log")}>{t("ds.tab.log")}</button>
        </div>
      )}
      {header}
      <div className="dt-body">
        {step === 0 && connectStep}
        {step === 1 && tablesStep}
        {step === 2 && resultStep}
        {step === 3 && (tab === "script" ? scriptTab : logTab)}
        {message && <div className="msg-error dt-message" role="alert">{message}</div>}
      </div>
      <div className="dt-footer">
        {footerCommon}
        {step <= 1 && <button type="button" className="btn dt-wide" onClick={() => setOptionsOpen(true)}>{t("dt.options")}</button>}
        {step === 3 && tab === "script" && (
          <div className="dt-menu-wrap split-button">
            <button type="button" className="btn dt-wide" onClick={() => setMenu((m) => (m === "deploy" ? null : "deploy"))}>{t("ds.deployOptions")}</button>
            <button type="button" className="btn dt-arrow-btn" aria-label={t("ds.deployOptions")} onClick={() => setMenu((m) => (m === "deploy" ? null : "deploy"))}><SmallIcon name="chevronDown" size={10} /></button>
            {menu === "deploy" && <PopupMenu items={deployItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%", left: 0 }} />}
          </div>
        )}
        <span className="spacer" />
        {step > 0 && <button type="button" className="btn dt-wide" disabled={running} onClick={() => { setMessage(null); setStep(step === 3 ? 2 : step - 1); }}>{t("exp.back").replace("< ", "")}</button>}
        {step >= 2 && <button type="button" className="btn dt-wide" disabled={running || !!comparing} onClick={() => void compare()}>{t("ds.recompare")}</button>}
        {step === 0 && <button type="button" className="btn btn-primary dt-wide" onClick={() => void goNext()}>{t("exp.next").replace(" >", "")}</button>}
        {step === 1 && <button type="button" className="btn btn-primary dt-wide" disabled={!rows || !!comparing} onClick={() => void compare(mode === "deploy")}>{t(mode === "deploy" ? "ds.btnDeploy" : "ds.btnPreview")}</button>}
        {step === 2 && <button type="button" className="btn btn-primary dt-wide" onClick={() => void goNext()}>{t("exp.next").replace(" >", "")}</button>}
        {step === 3 && tab === "script" && <button type="button" className="btn btn-primary dt-wide" disabled={running} onClick={() => void deploy()}>{t("exp.start")}</button>}
        {step === 3 && tab === "log" && (running
          ? <button type="button" className="btn btn-primary dt-wide" onClick={() => void pg.exportCancel(deployId.current).catch(() => undefined)}>{t("exp.stop")}</button>
          : <button type="button" className="btn btn-primary dt-wide" onClick={() => void getCurrentWindow().close()}>{t("common.close")}</button>)}
      </div>

      {comparing && (
        <div className="modal-overlay">
          <div className="ds-comparing" role="dialog" aria-label={t("ds.comparing")}>
            <div className="ds-comparing-title"><SmallIcon name="database" size={30} /><strong>{t("ds.comparing")}</strong></div>
            <div className="exp-progress"><div className="exp-progress-bar" style={{ width: `${Math.round(comparing.fraction * 100)}%` }} /></div>
            <div className="ds-comparing-text">{comparing.text}</div>
            <div className="ds-comparing-actions"><button type="button" className="btn" onClick={stopCompare}>{t("common.cancel")}</button></div>
          </div>
        </div>
      )}
      {mapEdit && (
        <MappingEditor
          edit={mapEdit}
          onClose={() => setMapEdit(null)}
          onOk={(pairs) => { patchRow(mapEdit.table, mapEdit.kind === "key" ? { key: pairs } : { fields: pairs }); setMapEdit(null); }}
        />
      )}
      {optionsOpen && (
        <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) setOptionsOpen(false); }}>
          <div className="modal exp-options-dialog" role="dialog" aria-label={t("dt.options")}>
            <div className="modal-header"><span className="modal-title">{t("dt.options")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={() => setOptionsOpen(false)}><SmallIcon name="close" /></button></div>
            <div className="modal-body exp-options">
              <label className="check-row"><input type="checkbox" checked={options.insert} onChange={(e) => setOptions({ ...options, insert: e.target.checked })} /><span>{t("ds.opt.insert")}</span></label>
              <label className="check-row"><input type="checkbox" checked={options.update} onChange={(e) => setOptions({ ...options, update: e.target.checked })} /><span>{t("ds.opt.update")}</span></label>
              <label className="check-row"><input type="checkbox" checked={options.delete} onChange={(e) => setOptions({ ...options, delete: e.target.checked })} /><span>{t("ds.opt.delete")}</span></label>
              <div className="muted exp-note">{t("ds.opt.note")}</div>
            </div>
            <div className="modal-footer"><span className="spacer" /><button type="button" className="btn btn-primary" onClick={() => setOptionsOpen(false)}>OK</button></div>
          </div>
        </div>
      )}
      {passwordDialog}
      {namePrompt && <MessageDialog title={t("exp.saveProfile")} prompt={{ label: t("exp.profileName"), initial: "" }} okLabel={t("exp.saveProfile")} onOk={(n) => { if (n) saveProfile(n); }} onCancel={() => setNamePrompt(false)} />}
    </div>
  );
}

/** Editor do mapeamento de chave ou de campos de uma tabela (origem → destino), com ordem. */
function MappingEditor({ edit, onClose, onOk }: { edit: { kind: "key" | "fields"; table: string; pairs: Pair[]; sourceCols: string[]; targetCols: string[] }; onClose: () => void; onOk: (pairs: Pair[]) => void }) {
  const t = useT();
  const [pairs, setPairs] = useState<Pair[]>(edit.pairs);
  const [sel, setSel] = useState(0);
  const move = (dir: -1 | 1) => {
    const to = sel + dir;
    if (to < 0 || to >= pairs.length) return;
    const next = [...pairs];
    [next[sel], next[to]] = [next[to], next[sel]];
    setPairs(next);
    setSel(to);
  };
  const choose = (i: number, p: Partial<Pair>) => setPairs((all) => all.map((x, k) => (k === i ? { ...x, ...p } : x)));
  const valid = useMemo(() => pairs.filter((p) => p.source && p.target), [pairs]);
  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal ds-map-dialog" role="dialog" aria-label={t(edit.kind === "key" ? "ds.keyMapping" : "ds.fieldMapping")}>
        <div className="modal-header"><span className="modal-title">{t(edit.kind === "key" ? "ds.keyMapping" : "ds.fieldMapping")} — {edit.table}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="modal-body">
          <div className="dt-grid ds-map-grid">
            <div className="dt-grid-head"><span>{t("ds.mapSource")}</span><span>{t("ds.mapTarget")}</span></div>
            {pairs.map((p, i) => (
              <div key={i} className={`dt-grid-row${sel === i ? " is-sel" : ""}`} onClick={() => setSel(i)}>
                <select className="field" value={p.source} onChange={(e) => choose(i, { source: e.target.value, target: edit.targetCols.includes(e.target.value) ? e.target.value : p.target })}>
                  <option value="" />
                  {edit.sourceCols.map((c) => <option key={c}>{c}</option>)}
                </select>
                <select className="field" value={p.target} onChange={(e) => choose(i, { target: e.target.value })}>
                  <option value="" />
                  {edit.targetCols.map((c) => <option key={c}>{c}</option>)}
                </select>
              </div>
            ))}
          </div>
          <div className="dt-grid-tools">
            <button type="button" className="icon-button" aria-label="↑" onClick={() => move(-1)}>↑</button>
            <button type="button" className="icon-button" aria-label="↓" onClick={() => move(1)}>↓</button>
            <button type="button" className="icon-button" aria-label="+" onClick={() => setPairs([...pairs, { source: "", target: "" }])}>+</button>
            <button type="button" className="icon-button" aria-label="−" disabled={pairs.length === 0} onClick={() => { setPairs(pairs.filter((_, k) => k !== sel)); setSel(Math.max(0, sel - 1)); }}>−</button>
          </div>
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" disabled={edit.kind === "key" && valid.length === 0} onClick={() => onOk(valid)}>OK</button>
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
        </div>
      </div>
    </div>
  );
}
