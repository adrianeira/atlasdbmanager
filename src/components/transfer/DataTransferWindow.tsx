import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MenuEntry } from "../../commands";
import {
  errorText, local, onExportProgress, onTransferLog, onTransferProgress, pg, toAtlasError,
  type DataTransferParams, type TransferCounts,
} from "../../backend";
import { idPrefix } from "../../providerRoute";
import { useT } from "../../i18n";
import { SmallIcon } from "../../icons";
import { MessageDialog } from "../MessageDialog";
import { PopupMenu } from "../PopupMenu";
import { ToolWindow } from "../ToolWindow";
import { useEndpoints } from "./endpoints";
import {
  CATS, PROFILES_KEY, buildSpec, defaultAdv, formatClock, itemsOf, previewSql, selKey,
  type Cat, type Item, type TableAdv, type TransferOptions, type TransferProfile,
} from "./transferModel";

type Params = DataTransferParams & { title: string };

const STEP_COUNT = 4;
const CAT_LABEL = { table: "dt.cat.table", view: "dt.cat.view", function: "dt.cat.function", sequence: "dt.cat.sequence", type: "dt.cat.type" } as const;
const ALL_LABEL = { table: "dt.all.table", view: "dt.all.view", function: "dt.all.function", sequence: "dt.all.sequence", type: "dt.all.type" } as const;

/** Janela do Data Transfer: origem e destino → objetos → modo de cada tabela → resumo → andamento. */
export function DataTransferWindow() {
  const params = (window as unknown as { __ATLAS_DT__: Params }).__ATLAS_DT__;
  return (
    <ToolWindow title={params.title}>
      <Wizard params={params} />
    </ToolWindow>
  );
}

function Wizard({ params }: { params: Params }) {
  const t = useT();
  const [step, setStep] = useState(0);
  const [items, setItems] = useState<Record<Cat, Item[]> | null>(null);
  const [itemsError, setItemsError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [collapsed, setCollapsed] = useState<Set<Cat>>(new Set());
  const [focus, setFocus] = useState<string | null>(null);
  const [adv, setAdv] = useState<Record<string, TableAdv>>({});
  const [columns, setColumns] = useState<Record<string, string[]>>({});

  const [dropBefore, setDropBefore] = useState(true);
  const [continueOnError, setContinueOnError] = useState(false);
  const [options, setOptions] = useState<TransferOptions>({ structure: true, data: true });
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [confirmDrop, setConfirmDrop] = useState(false);

  const [profiles, setProfiles] = useState<TransferProfile[]>([]);
  const [menu, setMenu] = useState<"save" | "load" | null>(null);
  const [namePrompt, setNamePrompt] = useState(false);

  const [running, setRunning] = useState(false);
  const [log, setLog] = useState<{ text: string; error?: boolean }[]>([]);
  const [counts, setCounts] = useState<TransferCounts>({ objects: 0, processed: 0, transferred: 0, errors: 0, elapsedMs: 0 });
  const [doneTables, setDoneTables] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [finished, setFinished] = useState(false);
  const transferId = useRef("");
  const startedAt = useRef(0);
  const logRef = useRef<HTMLDivElement>(null);
  const [generator, setGenerator] = useState<string | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);

  const {
    source, setSource, target, setTarget, targetKind, setTargetKind, filePath, setFilePath, sourceConn, message, setMessage,
    header, connectStep, passwordDialog, endpointProblem,
  } = useEndpoints(params, { allowFile: true, onSourceChange: () => { setSelected(new Set()); setItems(null); } });

  useEffect(() => { void local.getPref<TransferProfile[]>(PROFILES_KEY).then((p) => setProfiles(Array.isArray(p) ? p : [])).catch(() => undefined); }, []);

  /* ---------- Objetos do schema de origem ---------- */
  useEffect(() => {
    if (step < 1 || !sourceConn?.open || !source.database || !source.schema) return;
    let alive = true;
    setItems(null);
    setItemsError(null);
    pg.listObjects(source.connId, source.database, source.schema)
      .then((o) => alive && setItems(itemsOf(o)))
      .catch((e) => alive && setItemsError(errorText(toAtlasError(e))));
    return () => { alive = false; };
    // Só relê ao mudar a origem ou ao voltar à etapa dos objetos.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step === 1, source.connId, source.database, source.schema]);

  const loadColumns = useCallback(async (table: string) => {
    if (columns[table]) return;
    try {
      const d = await pg.tableDetail(source.connId, source.database, source.schema, table);
      setColumns((c) => ({ ...c, [table]: d.columns.map((x) => x.name) }));
    } catch {
      setColumns((c) => ({ ...c, [table]: [] }));
    }
  }, [columns, source.connId, source.database, source.schema]);

  useEffect(() => {
    if (focus?.startsWith("table:")) void loadColumns(focus.slice(6));
  }, [focus, loadColumns]);

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

  /* ---------- Marcação dos objetos ---------- */
  const countOf = (cat: Cat) => (items ? items[cat].filter((i) => selected.has(selKey(i))).length : 0);
  const toggleItem = (i: Item) => setSelected((s) => { const n = new Set(s); const k = selKey(i); if (n.has(k)) n.delete(k); else n.add(k); return n; });
  const toggleCat = (cat: Cat) => {
    if (!items) return;
    const every = items[cat].length > 0 && items[cat].every((i) => selected.has(selKey(i)));
    setSelected((s) => { const n = new Set(s); items[cat].forEach((i) => (every ? n.delete(selKey(i)) : n.add(selKey(i)))); return n; });
  };
  const patchAdv = (table: string, p: Partial<TableAdv>) => setAdv((a) => ({ ...a, [table]: { ...(a[table] ?? defaultAdv(table)), ...p } }));
  const advOf = (table: string) => adv[table] ?? defaultAdv(table);

  /* ---------- Navegação ---------- */
  const problemFor = (to: number): string | null => {
    if (to >= 1) {
      const p = endpointProblem();
      if (p) return p;
    }
    if (to >= 2 && selected.size === 0) return t("dt.needObjects");
    if (to >= 2 && targetKind === "file" && !filePath) return t("dt.needFile");
    return null;
  };

  const goTo = (to: number) => {
    if (to > step) {
      const p = problemFor(to);
      if (p) { setMessage(p); return; }
    }
    setMessage(null);
    setMenu(null);
    setStep(to);
  };

  /* ---------- Perfis ---------- */
  const persist = (next: TransferProfile[]) => { setProfiles(next); void local.setPref(PROFILES_KEY, next).catch(() => undefined); };
  const saveProfile = (name: string) => {
    const p: TransferProfile = {
      name, sourceConn: source.connId, sourceDb: source.database, sourceSchema: source.schema,
      target: { kind: targetKind, conn: target.connId, db: target.database, schema: target.schema, path: filePath },
      selected: [...selected], adv, dropBefore, continueOnError, options,
    };
    persist([...profiles.filter((x) => x.name !== name), p].sort((a, b) => a.name.localeCompare(b.name)));
    setNamePrompt(false);
    setLog((l) => [...l, { text: t("exp.profileSaved", { name }) }]);
  };
  const loadProfile = (p: TransferProfile) => {
    setSource({ connId: p.sourceConn, database: p.sourceDb, schema: p.sourceSchema });
    setTarget({ connId: p.target.conn, database: p.target.db, schema: p.target.schema });
    setTargetKind(p.target.kind);
    setFilePath(p.target.path);
    setSelected(new Set(p.selected));
    setAdv(p.adv);
    setDropBefore(p.dropBefore);
    setContinueOnError(p.continueOnError);
    setOptions(p.options);
    setStep(0);
    setLog((l) => [...l, { text: t("exp.profileLoaded", { name: p.name }) }]);
  };

  /* ---------- Execução ---------- */
  const requestStart = () => {
    const p = problemFor(STEP_COUNT);
    if (p) { setMessage(p); return; }
    if (dropBefore && options.structure) setConfirmDrop(true);
    else void start();
  };

  const start = async () => {
    if (running || !items) return;
    setConfirmDrop(false);
    setStep(3);
    setFinished(false);
    setLog([{ text: `[DTF] ${t("dt.log.start")}` }]);
    setCounts({ objects: 0, processed: 0, transferred: 0, errors: 0, elapsedMs: 0 });
    setDoneTables(0);
    setElapsed(0);
    startedAt.current = performance.now();
    setRunning(true);
    // O prefixo da conexão leva o botão Parar (exportCancel) ao banco certo.
    const id = `${idPrefix(source.connId)}${crypto.randomUUID()}`;
    transferId.current = id;
    const spec = buildSpec({ sourceSchema: source.schema, targetSchema: target.schema, items, selected, adv, dropBefore, continueOnError, options });
    const add = (text: string, error = false) => setLog((l) => [...l, { text, error }]);
    const unlistenProgress = await onTransferProgress(id, setCounts);
    const unlistenLog = await onTransferLog(id, (line) => {
      add(line, line.startsWith("[ERR]"));
      if (/record\(s\) transferred/.test(line)) setDoneTables((n) => n + 1);
    });
    try {
      if (targetKind === "file") {
        const unlistenRows = await onExportProgress(id, (n) => setCounts((c) => ({ ...c, processed: n, transferred: n })));
        try {
          const r = await pg.dumpSql({
            dumpId: id, sessionId: source.connId, database: source.database,
            options: {
              scope: { kind: "selection", schema: spec.sourceSchema, tables: spec.tables, views: spec.views, functions: spec.functions, sequences: spec.sequences, types: spec.types },
              structure: spec.structure, data: spec.data, clean: spec.dropBeforeCreate, includeOwner: false,
            },
            dataFormat: "insert", rowsPerInsert: 100, path: filePath,
          });
          setCounts((c) => ({ ...c, objects: r.objects, processed: r.rows, transferred: r.rows }));
          add(`[DTF] ${t("dt.log.file", { path: r.path })}`);
        } finally {
          unlistenRows();
        }
      } else {
        const c = await pg.transfer({
          transferId: id, sourceConn: source.connId, sourceDatabase: source.database,
          targetConn: target.connId, targetDatabase: target.database, spec,
        });
        setCounts(c);
      }
      add(`[DTF] ${t("dt.log.end")}`);
    } catch (e) {
      const err = toAtlasError(e);
      add(err.kind === "canceled" ? `[DTF] ${t("dt.log.canceled")}` : `[ERR] ${errorText(err)}`, err.kind !== "canceled");
    } finally {
      unlistenProgress();
      unlistenLog();
    }
    setElapsed(performance.now() - startedAt.current);
    setRunning(false);
    setFinished(true);
  };

  const stop = () => void pg.exportCancel(transferId.current).catch(() => undefined);

  const totalTables = items ? countOf("table") : 0;
  const percent = finished ? 100 : running ? Math.min(99, totalTables ? Math.round((100 * doneTables) / totalTables) : 0) : 0;
  useEffect(() => {
    void getCurrentWindow().setTitle(running || finished ? `${percent}% - ${t("dt.title")}` : `* ${t("dt.title")}`).catch(() => undefined);
  }, [running, finished, percent, t]);

  /* ---------- Peças das etapas ---------- */
  const catIcon = { table: "table", view: "view", function: "function", sequence: "sequence", type: "type" } as const;
  const tree = (
    <div className="dt-tree" role="tree">
      {items === null && !itemsError && <div className="muted dt-loading"><span className="spinner" />{t("dt.reading")}</div>}
      {itemsError && <div className="msg-error">{itemsError}</div>}
      {items && CATS.map((cat) => {
        const list = items[cat];
        const n = countOf(cat);
        const open = !collapsed.has(cat);
        return (
          <div key={cat}>
            <div className="dt-tree-row dt-cat">
              <button type="button" className="dt-twisty" aria-label="toggle" onClick={() => setCollapsed((c) => { const x = new Set(c); if (x.has(cat)) x.delete(cat); else x.add(cat); return x; })}>
                {list.length > 0 && <SmallIcon name={open ? "chevronDown" : "chevronRight"} size={10} />}
              </button>
              <input type="checkbox" checked={list.length > 0 && n === list.length} ref={(el) => { if (el) el.indeterminate = n > 0 && n < list.length; }} onChange={() => toggleCat(cat)} aria-label={t(CAT_LABEL[cat])} />
              <SmallIcon name={catIcon[cat]} size={18} />
              <span>{t(CAT_LABEL[cat])} {n === list.length && n > 0 ? "( * )" : `(${n}/${list.length})`}</span>
            </div>
            {open && list.map((i) => (
              <div key={selKey(i)} className={`dt-tree-row dt-item${focus === selKey(i) ? " is-focus" : ""}`} onClick={() => setFocus(selKey(i))}>
                <input type="checkbox" checked={selected.has(selKey(i))} onChange={() => toggleItem(i)} onClick={(e) => e.stopPropagation()} aria-label={i.label} />
                <SmallIcon name={catIcon[cat]} size={18} />
                <span title={i.label}>{i.label}</span>
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );

  const focusTable = focus?.startsWith("table:") ? focus.slice(6) : null;
  const focusAdv = focusTable ? advOf(focusTable) : null;
  const focusCols = focusTable ? columns[focusTable] ?? [] : [];

  const moveRow = (list: string[] | { source: string; target: string }[], from: number, dir: -1 | 1) => {
    const to = from + dir;
    if (from < 0 || to < 0 || to >= list.length) return list;
    const next = [...list];
    [next[from], next[to]] = [next[to], next[from]];
    return next;
  };
  const [fieldSel, setFieldSel] = useState(0);
  const [setSel, setSetSel] = useState(0);

  const advancedPanel = focusTable && focusAdv && (
    <div className="dt-adv">
      <div className="dt-modes">
        <span>{t("dt.mode")}</span>
        <label><input type="radio" name="dt-mode" checked={focusAdv.mode === "auto"} onChange={() => patchAdv(focusTable, { mode: "auto" })} />{t("dt.mode.auto")}</label>
        <label><input type="radio" name="dt-mode" checked={focusAdv.mode === "advanced"} onChange={() => patchAdv(focusTable, { mode: "advanced" })} />{t("dt.mode.advanced")}</label>
      </div>
      {focusAdv.mode === "advanced" && (
        <fieldset className="dt-advanced">
          <div className="dt-row"><label>{t("dt.targetName")}</label><input className="field" value={focusAdv.targetName} spellCheck={false} onChange={(e) => patchAdv(focusTable, { targetName: e.target.value })} /></div>
          <label className="dt-radio"><input type="radio" name="dt-fields" checked={!focusAdv.customFields} onChange={() => patchAdv(focusTable, { customFields: false })} />{t("dt.allFields")}</label>
          <label className="dt-radio"><input type="radio" name="dt-fields" checked={focusAdv.customFields} onChange={() => patchAdv(focusTable, { customFields: true, fields: focusAdv.fields.length ? focusAdv.fields : focusCols.map((c) => ({ source: c, target: c })) })} />{t("dt.customFields")}</label>
          <div className={`dt-grid${focusAdv.customFields ? "" : " is-disabled"}`}>
            <div className="dt-grid-head"><span>{t("dt.sourceField")}</span><span>{t("dt.targetField")}</span></div>
            {focusAdv.fields.map((f, i) => (
              <div key={i} className={`dt-grid-row${fieldSel === i ? " is-sel" : ""}`} onClick={() => setFieldSel(i)}>
                <select className="field" disabled={!focusAdv.customFields} value={f.source} onChange={(e) => patchAdv(focusTable, { fields: focusAdv.fields.map((x, k) => (k === i ? { source: e.target.value, target: x.target || e.target.value } : x)) })}>
                  <option value="" />
                  {focusCols.map((c) => <option key={c}>{c}</option>)}
                </select>
                <input className="field" disabled={!focusAdv.customFields} value={f.target} spellCheck={false} onChange={(e) => patchAdv(focusTable, { fields: focusAdv.fields.map((x, k) => (k === i ? { ...x, target: e.target.value } : x)) })} />
              </div>
            ))}
          </div>
          <div className="dt-grid-tools">
            <button type="button" className="icon-button" disabled={!focusAdv.customFields} aria-label="+" onClick={() => patchAdv(focusTable, { fields: [...focusAdv.fields, { source: "", target: "" }] })}>+</button>
            <button type="button" className="icon-button" disabled={!focusAdv.customFields || !focusAdv.fields.length} aria-label="−" onClick={() => patchAdv(focusTable, { fields: focusAdv.fields.filter((_, k) => k !== fieldSel) })}>−</button>
            <button type="button" className="icon-button" disabled={!focusAdv.customFields} aria-label="↑" onClick={() => { patchAdv(focusTable, { fields: moveRow(focusAdv.fields, fieldSel, -1) as typeof focusAdv.fields }); setFieldSel(Math.max(0, fieldSel - 1)); }}>↑</button>
            <button type="button" className="icon-button" disabled={!focusAdv.customFields} aria-label="↓" onClick={() => { patchAdv(focusTable, { fields: moveRow(focusAdv.fields, fieldSel, 1) as typeof focusAdv.fields }); setFieldSel(Math.min(focusAdv.fields.length - 1, fieldSel + 1)); }}>↓</button>
          </div>

          <label className="dt-radio"><input type="radio" name="dt-rows" checked={!focusAdv.customSets} onChange={() => patchAdv(focusTable, { customSets: false })} />{t("dt.allRows")}</label>
          <div className="dt-batch">
            <label className="check-row"><input type="checkbox" checked={focusAdv.batchOn} disabled={focusAdv.customSets} onChange={(e) => patchAdv(focusTable, { batchOn: e.target.checked })} /><span>{t("dt.batch")}</span></label>
            <input className="field dt-num" type="number" min={1} value={focusAdv.batch} disabled={!focusAdv.batchOn || focusAdv.customSets} onChange={(e) => patchAdv(focusTable, { batch: Math.max(1, Math.floor(Number(e.target.value) || 1)) })} />
          </div>
          <label className="dt-radio"><input type="radio" name="dt-rows" checked={focusAdv.customSets} onChange={() => patchAdv(focusTable, { customSets: true })} />{t("dt.customSets")}</label>
          <div className={`dt-grid${focusAdv.customSets ? "" : " is-disabled"}`}>
            <div className="dt-grid-head"><span>{t("dt.recordset")}</span></div>
            {focusAdv.sets.map((s, i) => (
              <div key={i} className={`dt-grid-row${setSel === i ? " is-sel" : ""}`} onClick={() => setSetSel(i)}>
                <input className="field" disabled={!focusAdv.customSets} value={s} placeholder='e.g. "id" >= 1 AND "id" <= 100' spellCheck={false} onChange={(e) => patchAdv(focusTable, { sets: focusAdv.sets.map((x, k) => (k === i ? e.target.value : x)) })} />
              </div>
            ))}
          </div>
          <div className="dt-grid-tools">
            <button type="button" className="icon-button" disabled={!focusAdv.customSets} aria-label="+" onClick={() => patchAdv(focusTable, { sets: [...focusAdv.sets, ""] })}>+</button>
            <button type="button" className="icon-button" disabled={!focusAdv.customSets || focusAdv.sets.length < 2} aria-label="−" onClick={() => patchAdv(focusTable, { sets: focusAdv.sets.filter((_, k) => k !== setSel) })}>−</button>
            <button type="button" className="icon-button" disabled={!focusAdv.customSets} aria-label="↑" onClick={() => { patchAdv(focusTable, { sets: moveRow(focusAdv.sets, setSel, -1) as string[] }); setSetSel(Math.max(0, setSel - 1)); }}>↑</button>
            <button type="button" className="icon-button" disabled={!focusAdv.customSets} aria-label="↓" onClick={() => { patchAdv(focusTable, { sets: moveRow(focusAdv.sets, setSel, 1) as string[] }); setSetSel(Math.min(focusAdv.sets.length - 1, setSel + 1)); }}>↓</button>
            <button type="button" className="btn" disabled={!focusAdv.customSets} onClick={() => setGenerator(focusCols[0] ?? "")}>{t("dt.generator")}</button>
          </div>
          <button type="button" className="btn dt-preview-btn" onClick={() => setPreviewOpen(true)}>{t("dt.sqlPreview")}</button>
          <label className="check-row"><input type="checkbox" checked disabled readOnly /><span>{t("dt.useTx")}</span></label>
        </fieldset>
      )}
    </div>
  );

  const objectsStep = (
    <div className="dt-objects">
      <h2 className="dt-h">{t("dt.objects")}</h2>
      <div className="dt-split">
        {tree}
        <div className="dt-right">{focusTable && advancedPanel && targetKind === "connection" ? advancedPanel : <p className="dt-hint">{focusTable && targetKind === "file" ? t("dt.fileNoAdvanced") : t("dt.objectsHint")}</p>}</div>
      </div>
    </div>
  );

  const summaryRows: { key: string; icon: "table" | "view" | "function" | "sequence" | "type"; source: string; target: string; mode: string }[] = [];
  if (items) {
    for (const cat of CATS) {
      const list = items[cat];
      const chosen = list.filter((i) => selected.has(selKey(i)));
      if (chosen.length === 0) continue;
      if (chosen.length === list.length) summaryRows.push({ key: cat, icon: catIcon[cat], source: `${t(ALL_LABEL[cat])} ( * )`, target: "", mode: "" });
      else {
        for (const i of chosen) {
          const a = cat === "table" ? adv[i.name] : undefined;
          summaryRows.push({ key: selKey(i), icon: catIcon[cat], source: i.label, target: a?.mode === "advanced" && a.targetName ? a.targetName : i.name, mode: a?.mode === "advanced" ? t("dt.mode.advanced") : t("dt.mode.auto") });
        }
      }
    }
  }
  const summaryText = items ? CATS.filter((c) => countOf(c) > 0).map((c) => (countOf(c) === items[c].length ? t(ALL_LABEL[c]) : `${countOf(c)} ${t(CAT_LABEL[c]).toLowerCase()}`)).join(", ") : "";

  const summaryStep = (
    <div className="dt-summary">
      <h2 className="dt-h">{t("dt.summary")}</h2>
      <p>{summaryText}</p>
      <div className="dt-sum-grid">
        <div className="dt-sum-head"><span>{t("dt.srcObject")}</span><span>{t("dt.dstObject")}</span><span>{t("dt.modeCol")}</span></div>
        {summaryRows.map((r) => (
          <div key={r.key} className="dt-sum-row"><span><SmallIcon name={r.icon} size={16} />{r.source}</span><span>{r.target}</span><span>{r.mode}</span></div>
        ))}
      </div>
      <label className="check-row"><input type="checkbox" checked={continueOnError} onChange={(e) => setContinueOnError(e.target.checked)} /><span>{t("dt.continueOnError")}</span></label>
      <label className="check-row"><input type="checkbox" checked={dropBefore} onChange={(e) => setDropBefore(e.target.checked)} /><span>{t("dt.dropBefore")}</span></label>
    </div>
  );

  const progressStep = (
    <div className="dt-progress">
      <dl className="dt-stats">
        <dt>{t("dt.stat.objects")}</dt><dd>{counts.objects.toLocaleString()}</dd>
        <dt>{t("dt.stat.processed")}</dt><dd>{counts.processed.toLocaleString()}</dd>
        <dt>{t("dt.stat.transferred")}</dt><dd>{counts.transferred.toLocaleString()}</dd>
        <dt>{t("dt.stat.errors")}</dt><dd>{counts.errors.toLocaleString()}</dd>
        <dt>{t("exp.time").replace(":", "")}:</dt><dd>{formatClock(elapsed)}</dd>
      </dl>
      <div className="exp-log dt-log" ref={logRef} role="log" aria-live="polite">
        {log.map((l, i) => <div key={i} className={l.error ? "is-error" : undefined}>{l.text}</div>)}
      </div>
    </div>
  );

  const saveItems: MenuEntry[] = [
    { kind: "item", text: t("dt.saveAs"), run: () => setNamePrompt(true) },
    ...(profiles.length ? [{ kind: "sep" } as MenuEntry, ...profiles.map((p): MenuEntry => ({ kind: "item", text: t("exp.profileDelete", { name: p.name }), run: () => persist(profiles.filter((x) => x.name !== p.name)) }))] : []),
  ];
  const loadItems: MenuEntry[] = profiles.length ? profiles.map((p): MenuEntry => ({ kind: "item", text: p.name, run: () => loadProfile(p) })) : [{ kind: "item", text: t("exp.profileNone") }];

  const lastStep = step === 2;
  return (
    <div className="dt-window">
      {header}
      <div className="dt-body">
        {step === 0 && connectStep}
        {step === 1 && objectsStep}
        {step === 2 && summaryStep}
        {step === 3 && progressStep}
        {message && <div className="msg-error dt-message" role="alert">{message}</div>}
      </div>
      <div className="dt-footer">
        <div className="dt-menu-wrap split-button">
          <button type="button" className="btn dt-wide" disabled={running} onClick={() => setNamePrompt(true)}>{t("exp.saveProfile")}</button>
          <button type="button" className="btn dt-arrow-btn" aria-label={t("exp.saveProfile")} disabled={running} onClick={() => setMenu((m) => (m === "save" ? null : "save"))}><SmallIcon name="chevronDown" size={10} /></button>
          {menu === "save" && <PopupMenu items={saveItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%" }} />}
        </div>
        <div className="dt-menu-wrap">
          <button type="button" className="btn dt-wide" disabled={running} onClick={() => setMenu((m) => (m === "load" ? null : "load"))}>{t("dt.loadProfile")}</button>
          {menu === "load" && <PopupMenu items={loadItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%", left: 0 }} />}
        </div>
        {step !== 3 && <button type="button" className="btn dt-wide" onClick={() => setOptionsOpen(true)}>{t("dt.options")}</button>}
        <span className="spacer" />
        {step > 0 && <button type="button" className="btn dt-wide" disabled={running} onClick={() => goTo(step - 1)}>{t("exp.back").replace("< ", "")}</button>}
        {step === 3
          ? (running
            ? <button type="button" className="btn btn-primary dt-wide" onClick={stop}>{t("exp.stop")}</button>
            : <button type="button" className="btn btn-primary dt-wide" onClick={() => void getCurrentWindow().close()}>{t("common.close")}</button>)
          : lastStep
            ? <button type="button" className="btn btn-primary dt-wide" onClick={requestStart}>{t("exp.start")}</button>
            : <button type="button" className="btn btn-primary dt-wide" disabled={step === 1 && selected.size === 0} onClick={() => goTo(step + 1)}>{t("exp.next").replace(" >", "")}</button>}
      </div>

      {optionsOpen && (
        <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) setOptionsOpen(false); }}>
          <div className="modal exp-options-dialog" role="dialog" aria-label={t("dt.options")}>
            <div className="modal-header"><span className="modal-title">{t("dt.options")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={() => setOptionsOpen(false)}><SmallIcon name="close" /></button></div>
            <div className="modal-body exp-options">
              <label className="check-row"><input type="checkbox" checked={options.structure} onChange={(e) => setOptions({ ...options, structure: e.target.checked, data: e.target.checked ? options.data : true })} /><span>{t("dt.opt.structure")}</span></label>
              <label className="check-row"><input type="checkbox" checked={options.data} onChange={(e) => setOptions({ ...options, data: e.target.checked, structure: e.target.checked ? options.structure : true })} /><span>{t("dt.opt.data")}</span></label>
            </div>
            <div className="modal-footer"><span className="spacer" /><button type="button" className="btn btn-primary" onClick={() => setOptionsOpen(false)}>OK</button></div>
          </div>
        </div>
      )}
      {passwordDialog}
      {confirmDrop && <MessageDialog title={t("dt.title")} message={t("dt.confirmDrop")} onOk={() => void start()} onCancel={() => setConfirmDrop(false)} />}
      {namePrompt && <MessageDialog title={t("exp.saveProfile")} prompt={{ label: t("exp.profileName"), initial: "" }} okLabel={t("exp.saveProfile")} onOk={(n) => { if (n) saveProfile(n); }} onCancel={() => setNamePrompt(false)} />}
      {previewOpen && focusTable && focusAdv && (
        <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) setPreviewOpen(false); }}>
          <div className="modal exp-options-dialog" role="dialog" aria-label={t("dt.sqlPreview")}>
            <div className="modal-header"><span className="modal-title">{t("dt.sqlPreview")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={() => setPreviewOpen(false)}><SmallIcon name="close" /></button></div>
            <div className="modal-body"><pre className="dt-pre">{previewSql(source.schema, focusTable, focusAdv, focusCols)}</pre></div>
            <div className="modal-footer"><span className="spacer" /><button type="button" className="btn btn-primary" onClick={() => setPreviewOpen(false)}>OK</button></div>
          </div>
        </div>
      )}
      {generator !== null && focusTable && focusAdv && (
        <Generator
          columns={focusCols}
          column={generator}
          onClose={() => setGenerator(null)}
          onMake={(sets) => { patchAdv(focusTable, { customSets: true, sets }); setGenerator(null); }}
        />
      )}
    </div>
  );
}

/** "Recordset Generator": faixas de uma coluna numérica, uma por filtro (`"id" >= 1 AND "id" < 1001`). */
function Generator({ columns, column, onClose, onMake }: { columns: string[]; column: string; onClose: () => void; onMake: (sets: string[]) => void }) {
  const t = useT();
  const [col, setCol] = useState(column);
  const [from, setFrom] = useState(1);
  const [to, setTo] = useState(10000);
  const [step, setStep] = useState(1000);
  const sets = useMemo(() => {
    const out: string[] = [];
    const q = `"${col.replace(/"/g, '""')}"`;
    for (let a = from; a <= to && out.length < 500; a += Math.max(1, step)) out.push(`${q} >= ${a} AND ${q} < ${Math.min(a + Math.max(1, step), to + 1)}`);
    return out;
  }, [col, from, to, step]);
  const num = (v: number, set: (n: number) => void) => <input className="field dt-num" type="number" value={v} onChange={(e) => set(Math.floor(Number(e.target.value) || 0))} />;
  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal exp-options-dialog" role="dialog" aria-label={t("dt.generator")}>
        <div className="modal-header"><span className="modal-title">{t("dt.generator")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="modal-body exp-options">
          <div className="exp-row"><label>{t("dt.gen.field")}</label><select className="field" value={col} onChange={(e) => setCol(e.target.value)}>{columns.map((c) => <option key={c}>{c}</option>)}</select></div>
          <div className="exp-row"><label>{t("dt.gen.from")}</label>{num(from, setFrom)}</div>
          <div className="exp-row"><label>{t("dt.gen.to")}</label>{num(to, setTo)}</div>
          <div className="exp-row"><label>{t("dt.gen.step")}</label>{num(step, setStep)}</div>
          <div className="muted exp-note">{t("dt.gen.count", { n: sets.length })}</div>
        </div>
        <div className="modal-footer"><span className="spacer" /><button type="button" className="btn btn-primary" disabled={!col || sets.length === 0} onClick={() => onMake(sets)}>OK</button><button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button></div>
      </div>
    </div>
  );
}
