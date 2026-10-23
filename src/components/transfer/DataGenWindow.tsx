import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useRef, useState } from "react";
import type { MenuEntry } from "../../commands";
import {
  errorText, local, onGenLog, onGenProgress, pg, toAtlasError,
  type DataTransferParams, type GenColumnMeta, type GenOptions, type GenPreview, type GenStats, type GenTableMeta, type Generator, type GeneratorKind, type TextStyle,
} from "../../backend";
import { useT } from "../../i18n";
import { SmallIcon } from "../../icons";
import { Dropdown } from "../Dropdown";
import { MessageDialog } from "../MessageDialog";
import { PopupMenu } from "../PopupMenu";
import { ToolWindow } from "../ToolWindow";
import { useEndpoints, type Endpoint } from "./endpoints";
import { formatClock } from "./transferModel";

type Params = DataTransferParams & { title: string };

interface ColCfg {
  checked: boolean;
  generator: Generator;
  nullPercent: number;
}
interface TableCfg {
  checked: boolean;
  rows: number;
  cols: Record<string, ColCfg>;
}
type Config = Record<string, TableCfg>;

interface GenProfile {
  name: string;
  target: Endpoint;
  config: Config;
  order: string[];
  options: GenOptions;
}

const PROFILES_KEY = "datagen.profiles";
const DEFAULT_ROWS = 1000;
const PREVIEW_ROWS = 100;
const KINDS: GeneratorKind[] = ["sequence", "integer", "decimal", "text", "dateTime", "date", "boolean", "uuid", "list", "fixed", "null", "reference"];
const STYLES: TextStyle[] = ["sentence", "words", "letters"];
const DEFAULT_OPTIONS: GenOptions = { continueOnError: false, emptyFirst: false, transaction: true, multiInsert: true };

/** Gerador inicial de um tipo, para quando o usuário troca de tipo na coluna. */
function defaultFor(kind: GeneratorKind, col: GenColumnMeta | undefined, tables: GenTableMeta[]): Generator {
  const today = new Date().toISOString().slice(0, 10);
  const max = Math.min(col?.charLen ?? 100, 100);
  switch (kind) {
    case "sequence": return { kind, start: null, step: 1 };
    case "integer": return { kind, min: 0, max: 1_000_000 };
    case "decimal": return { kind, min: 0, max: 10_000, scale: 2 };
    case "text": return { kind, minLen: Math.min(10, max), maxLen: max, style: max >= 20 ? "sentence" : "words" };
    case "dateTime": return { kind, from: "2000-01-01 00:00:00", to: `${today} 23:59:59` };
    case "date": return { kind, from: "2000-01-01", to: today };
    case "boolean": return { kind, truePercent: 50 };
    case "uuid": return { kind };
    case "list": return { kind, values: ["A", "B", "C"] };
    case "fixed": return { kind, value: "" };
    case "null": return { kind };
    case "reference": {
      const t = tables.find((x) => x.columns.some((c) => c.isPk)) ?? tables[0];
      return { kind, table: t?.name ?? "", column: t?.columns.find((c) => c.isPk)?.name ?? t?.columns[0]?.name ?? "" };
    }
  }
}

const buildConfig = (meta: GenTableMeta[], previous?: Config): Config =>
  Object.fromEntries(meta.map((t, i) => {
    const old = previous?.[t.name];
    return [t.name, {
      checked: old ? old.checked : i === 0,
      rows: old?.rows ?? DEFAULT_ROWS,
      cols: Object.fromEntries(t.columns.map((c) => {
        const o = old?.cols[c.name];
        return [c.name, { checked: o ? o.checked : !c.generated, generator: o?.generator ?? c.suggested, nullPercent: o?.nullPercent ?? 0 }];
      })),
    }];
  }));

export function DataGenWindow() {
  const params = (window as unknown as { __ATLAS_DG__: Params }).__ATLAS_DG__;
  return (
    <ToolWindow title={params.title}>
      <Wizard params={params} />
    </ToolWindow>
  );
}

function Illustration() {
  const t = useT();
  return (
    <svg className="dg-illustration" viewBox="0 0 330 360" aria-hidden="true">
      <g fill="none" stroke="#c9c9c9" strokeWidth="2"><rect x="40" y="10" width="60" height="44" /><rect x="110" y="0" width="52" height="40" /></g>
      <g fill="none" stroke="var(--c-accent)" strokeWidth="3"><rect x="62" y="30" width="62" height="52" /><path d="M62 47h62M62 64h62M83 30v52M104 30v52" /></g>
      <path d="M95 100v36" stroke="var(--c-accent)" strokeWidth="2" strokeDasharray="4 4" />
      <path d="M90 130l5 8 5-8" fill="none" stroke="var(--c-accent)" strokeWidth="2" />
      <g fill="none" stroke="var(--c-accent)" strokeWidth="3"><circle cx="100" cy="196" r="22" /><circle cx="100" cy="196" r="8" /></g>
      <g fill="none" stroke="#c9c9c9" strokeWidth="3"><circle cx="40" cy="180" r="22" /><circle cx="140" cy="176" r="16" /></g>
      <path d="M95 240v36" stroke="var(--c-accent)" strokeWidth="2" strokeDasharray="4 4" />
      <path d="M90 270l5 8 5-8" fill="none" stroke="var(--c-accent)" strokeWidth="2" />
      <g fill="none" stroke="var(--c-accent)" strokeWidth="3"><ellipse cx="95" cy="296" rx="32" ry="10" /><path d="M63 296v32c0 6 14 12 32 12s32-6 32-12v-32M63 312c0 6 14 12 32 12s32-6 32-12" /></g>
      <foreignObject x="150" y="52" width="170" height="60"><div className="dg-hint">{t("dg.hint1")}</div></foreignObject>
      <foreignObject x="150" y="180" width="170" height="70"><div className="dg-hint">{t("dg.hint2")}</div></foreignObject>
    </svg>
  );
}

function Wizard({ params }: { params: Params }) {
  const t = useT();
  const [step, setStep] = useState(0);
  const [meta, setMeta] = useState<GenTableMeta[] | null>(null);
  const [metaError, setMetaError] = useState<string | null>(null);
  const [config, setConfig] = useState<Config>({});
  const [order, setOrder] = useState<string[]>([]);
  const [options, setOptions] = useState<GenOptions>(DEFAULT_OPTIONS);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [orderDialog, setOrderDialog] = useState<string[] | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set(["#tables"]));
  const [focus, setFocus] = useState<{ table: string; column?: string } | null>(null);
  const [menu, setMenu] = useState<"save" | "load" | null>(null);
  const [profiles, setProfiles] = useState<GenProfile[]>([]);
  const [namePrompt, setNamePrompt] = useState(false);
  const pendingProfile = useRef<Config | null>(null);

  const [previewTable, setPreviewTable] = useState("");
  const [preview, setPreview] = useState<GenPreview | null>(null);
  const [previewBusy, setPreviewBusy] = useState(false);

  const [running, setRunning] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [stats, setStats] = useState<GenStats>({ objects: 0, generated: 0, inserted: 0, errors: 0 });
  const [elapsed, setElapsed] = useState(0);
  const runId = useRef("");
  const startedAt = useRef(0);
  const logRef = useRef<HTMLDivElement>(null);
  const totalRows = useRef(0);

  const {
    source: target, message, setMessage, header, connectStep, passwordDialog, endpointProblem, setSource: setTarget,
  } = useEndpoints(params, { allowFile: false, single: true, aside: <Illustration />, onSourceChange: () => { setMeta(null); } });

  useEffect(() => { void local.getPref<GenProfile[]>(PROFILES_KEY).then((p) => setProfiles(Array.isArray(p) ? p : [])).catch(() => undefined); }, []);
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

  /* ---------- Tabelas do schema ---------- */
  const describe = async () => {
    setMetaError(null);
    try {
      const list = await pg.genDescribe(target.connId, target.database, target.schema);
      setMeta(list);
      const base = pendingProfile.current ?? undefined;
      pendingProfile.current = null;
      setConfig((old) => buildConfig(list, base ?? (Object.keys(old).length ? old : undefined)));
      setOrder((o) => [...o.filter((n) => list.some((x) => x.name === n)), ...list.map((x) => x.name).filter((n) => !o.includes(n))]);
      setFocus(list[0] ? { table: list[0].name } : null);
      setExpanded(new Set(["#tables", ...(list[0] ? [list[0].name] : [])]));
    } catch (e) {
      setMetaError(errorText(toAtlasError(e)));
    }
  };

  const goNext = async () => {
    setMenu(null);
    if (step === 0) {
      const p = endpointProblem();
      if (p) { setMessage(p); return; }
      setMessage(null);
      setStep(1);
      if (!meta) await describe();
    } else if (step === 1) {
      if (!selected.length) { setMessage(t("dg.needTable")); return; }
      setMessage(null);
      setPreviewTable(selected[0]);
      setStep(2);
    }
  };

  /* ---------- Seleção e propriedades ---------- */
  const tableOf = (name: string) => meta?.find((x) => x.name === name);
  const selected = useMemo(() => order.filter((n) => config[n]?.checked && Object.values(config[n].cols).some((c) => c.checked)), [order, config]);
  const patchTable = (name: string, p: Partial<TableCfg>) => setConfig((c) => ({ ...c, [name]: { ...c[name], ...p } }));
  const patchCol = (table: string, col: string, p: Partial<ColCfg>) => setConfig((c) => ({ ...c, [table]: { ...c[table], cols: { ...c[table].cols, [col]: { ...c[table].cols[col], ...p } } } }));
  const resetTable = (name: string) => {
    const m = tableOf(name);
    if (m) setConfig((c) => ({ ...c, [name]: { ...buildConfig([m])[name], checked: c[name].checked } }));
  };
  const resetColumn = (table: string, col: string) => {
    const c = tableOf(table)?.columns.find((x) => x.name === col);
    if (c) patchCol(table, col, { generator: c.suggested, nullPercent: 0 });
  };
  const toggleTable = (name: string, on: boolean) => {
    patchTable(name, { checked: on });
    if (on) setFocus({ table: name });
  };
  const toggle = (set: Set<string>, key: string) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key); else next.add(key);
    setExpanded(next);
  };

  const specOf = (name: string) => {
    const c = config[name];
    return { name, rows: c.rows, columns: (tableOf(name)?.columns ?? []).filter((m) => c.cols[m.name]?.checked).map((m) => ({ name: m.name, generator: c.cols[m.name].generator, nullPercent: m.notNull ? 0 : c.cols[m.name].nullPercent })) };
  };

  /* ---------- Prévia ---------- */
  const loadPreview = async (table: string) => {
    if (!table || !config[table]) return;
    setPreviewBusy(true);
    setMessage(null);
    try {
      setPreview(await pg.genPreview(target.connId, target.database, target.schema, specOf(table), PREVIEW_ROWS));
    } catch (e) {
      setPreview(null);
      setMessage(errorText(toAtlasError(e)));
    } finally {
      setPreviewBusy(false);
    }
  };
  useEffect(() => { if (step === 2) void loadPreview(previewTable); /* eslint-disable-next-line react-hooks/exhaustive-deps */ }, [step, previewTable]);

  /* ---------- Execução ---------- */
  const start = async () => {
    if (running) return;
    const id = crypto.randomUUID();
    runId.current = id;
    totalRows.current = selected.reduce((n, name) => n + config[name].rows, 0);
    setLog([]);
    setStats({ objects: 0, generated: 0, inserted: 0, errors: 0 });
    setElapsed(0);
    setMessage(null);
    startedAt.current = performance.now();
    setStep(3);
    setRunning(true);
    const unlistenLog = await onGenLog(id, (line) => setLog((l) => [...l, line]));
    const unlistenProgress = await onGenProgress(id, setStats);
    try {
      const r = await pg.genRun({ runId: id, conn: target.connId, database: target.database, spec: { schema: target.schema, tables: selected.map(specOf), options } });
      setStats({ objects: r.objects, generated: r.generated, inserted: r.inserted, errors: r.errors });
    } catch (e) {
      const err = toAtlasError(e);
      setLog((l) => [...l, err.kind === "canceled" ? `[DGR] ${t("dt.log.canceled")}` : `[DGR] ERROR: ${errorText(err)}`]);
    } finally {
      unlistenLog();
      unlistenProgress();
    }
    setElapsed(performance.now() - startedAt.current);
    setRunning(false);
  };

  const pct = totalRows.current ? Math.min(100, (stats.generated / totalRows.current) * 100) : running ? 0 : 100;
  useEffect(() => {
    void getCurrentWindow().setTitle(step === 3 ? `${Math.round(pct)}% - ${t("dg.title")}` : step === 0 ? t("dg.title") : `* ${t("dg.title")}`).catch(() => undefined);
  }, [step, pct, t]);

  /* ---------- Perfis ---------- */
  const persist = (next: GenProfile[]) => { setProfiles(next); void local.setPref(PROFILES_KEY, next).catch(() => undefined); };
  const saveProfile = (name: string) => {
    persist([...profiles.filter((x) => x.name !== name), { name, target: { ...target }, config, order, options }].sort((a, b) => a.name.localeCompare(b.name)));
    setNamePrompt(false);
  };
  const loadProfile = (p: GenProfile) => {
    setTarget(p.target);
    setOrder(p.order);
    setOptions(p.options);
    pendingProfile.current = p.config;
    setConfig(p.config);
    setMeta(null);
    setStep(0);
  };

  /* ---------- Árvore e propriedades ---------- */
  const kindLabel = (g: Generator) => t(`dg.g.${g.kind}`);
  const tree = (
    <div className="dg-tree" role="tree">
      <div className="dg-node" style={{ paddingLeft: 4 }}>
        <button type="button" className="ss-chevron" aria-label={t("dg.tables")} onClick={() => toggle(expanded, "#tables")}><SmallIcon name={expanded.has("#tables") ? "chevronDown" : "chevronRight"} size={10} /></button>
        <SmallIcon name="table" />
        <span>{t("dg.tables")} ({selected.length}/{meta?.length ?? 0})</span>
      </div>
      {expanded.has("#tables") && order.map((name) => {
        const m = tableOf(name);
        const c = config[name];
        if (!m || !c) return null;
        const open = expanded.has(name);
        const isFocus = focus?.table === name && !focus.column;
        return (
          <div key={name}>
            <div className={`dg-node${isFocus ? " is-focus" : ""}`} style={{ paddingLeft: 24 }} onClick={() => setFocus({ table: name })}>
              <button type="button" className="ss-chevron" aria-label={name} onClick={(e) => { e.stopPropagation(); toggle(expanded, name); }}><SmallIcon name={open ? "chevronDown" : "chevronRight"} size={10} /></button>
              <input type="checkbox" checked={c.checked} aria-label={name} onClick={(e) => e.stopPropagation()} onChange={(e) => toggleTable(name, e.target.checked)} />
              <SmallIcon name="table" />
              <span>{name}</span>
              {c.checked && <span className="dg-dim">({c.rows})</span>}
            </div>
            {open && m.columns.map((col) => {
              const cc = c.cols[col.name];
              const colFocus = focus?.table === name && focus.column === col.name;
              return (
                <div key={col.name} className={`dg-node${colFocus ? " is-focus" : ""}${col.generated ? " is-muted" : ""}`} style={{ paddingLeft: 66 }} onClick={() => setFocus({ table: name, column: col.name })}>
                  <input type="checkbox" checked={!!cc?.checked && c.checked} disabled={col.generated || !c.checked} aria-label={col.name} onClick={(e) => e.stopPropagation()} onChange={(e) => patchCol(name, col.name, { checked: e.target.checked })} />
                  <SmallIcon name={col.isPk ? "columnPk" : "column"} />
                  <span>{col.name}</span>
                  {cc && <span className="dg-dim">({kindLabel(cc.generator)})</span>}
                </div>
              );
            })}
          </div>
        );
      })}
    </div>
  );

  const num = (v: string, fallback = 0) => (v.trim() === "" || Number.isNaN(Number(v)) ? fallback : Number(v));
  const field = (label: string, value: string | number, onChange: (v: string) => void, placeholder?: string) => (
    <label className="dg-field"><span>{label}</span><input className="field" value={value} placeholder={placeholder} spellCheck={false} onChange={(e) => onChange(e.target.value)} /></label>
  );

  const generatorEditor = (table: string, column: string) => {
    const meta1 = tableOf(table)?.columns.find((x) => x.name === column);
    const cc = config[table]?.cols[column];
    if (!meta1 || !cc) return null;
    const g = cc.generator;
    const set = (p: Partial<Generator>) => patchCol(table, column, { generator: { ...g, ...p } as Generator });
    const refMeta = g.kind === "reference" ? meta?.find((x) => x.name === g.table) : undefined;
    return (
      <div className="dg-props">
        <label className="dg-field"><span>{t("dg.p.type")}</span>
          <Dropdown className="dg-dd" value={g.kind} items={KINDS.map((k) => ({ value: k, label: t(`dg.g.${k}`) }))} onChange={(v) => patchCol(table, column, { generator: defaultFor(v as GeneratorKind, meta1, meta ?? []) })} />
        </label>
        {g.kind === "sequence" && <>
          {field(t("dg.p.start"), g.start ?? "", (v) => set({ start: v.trim() === "" ? null : Math.trunc(num(v)) }))}
          {field(t("dg.p.step"), g.step, (v) => set({ step: Math.trunc(num(v, 1)) }))}
        </>}
        {g.kind === "integer" && <>{field(t("dg.p.min"), g.min, (v) => set({ min: Math.trunc(num(v)) }))}{field(t("dg.p.max"), g.max, (v) => set({ max: Math.trunc(num(v)) }))}</>}
        {g.kind === "decimal" && <>
          {field(t("dg.p.min"), g.min, (v) => set({ min: num(v) }))}{field(t("dg.p.max"), g.max, (v) => set({ max: num(v) }))}
          {field(t("dg.p.scale"), g.scale, (v) => set({ scale: Math.max(0, Math.trunc(num(v))) }))}
        </>}
        {g.kind === "text" && <>
          <label className="dg-field"><span>{t("dg.p.style")}</span>
            <Dropdown className="dg-dd" value={g.style} items={STYLES.map((s) => ({ value: s, label: t(`dg.style.${s}`) }))} onChange={(v) => set({ style: v as TextStyle })} />
          </label>
          {field(t("dg.p.minLen"), g.minLen, (v) => set({ minLen: Math.max(0, Math.trunc(num(v))) }))}
          {field(t("dg.p.maxLen"), g.maxLen, (v) => set({ maxLen: Math.max(1, Math.trunc(num(v, 1))) }))}
        </>}
        {(g.kind === "dateTime" || g.kind === "date") && <>
          {field(t("dg.p.from"), g.from, (v) => set({ from: v }), g.kind === "date" ? "YYYY-MM-DD" : "YYYY-MM-DD hh:mm:ss")}
          {field(t("dg.p.to"), g.to, (v) => set({ to: v }), g.kind === "date" ? "YYYY-MM-DD" : "YYYY-MM-DD hh:mm:ss")}
        </>}
        {g.kind === "boolean" && field(t("dg.p.truePct"), g.truePercent, (v) => set({ truePercent: Math.min(100, Math.max(0, Math.trunc(num(v)))) }))}
        {g.kind === "list" && (
          <label className="dg-field is-top"><span>{t("dg.p.values")}</span>
            <textarea className="field dg-values" value={g.values.join("\n")} spellCheck={false} onChange={(e) => set({ values: e.target.value.split("\n").filter((x) => x !== "") })} />
          </label>
        )}
        {g.kind === "fixed" && field(t("dg.p.value"), g.value, (v) => set({ value: v }))}
        {g.kind === "reference" && <>
          <label className="dg-field"><span>{t("dg.p.refTable")}</span>
            <Dropdown className="dg-dd" value={g.table} items={(meta ?? []).map((x) => ({ value: x.name, label: x.name }))}
              onChange={(v) => { const m = meta?.find((x) => x.name === v); set({ table: v, column: m?.columns.find((c) => c.isPk)?.name ?? m?.columns[0]?.name ?? "" }); }} />
          </label>
          <label className="dg-field"><span>{t("dg.p.refColumn")}</span>
            <Dropdown className="dg-dd" value={g.column} items={(refMeta?.columns ?? []).map((x) => ({ value: x.name, label: x.name }))} onChange={(v) => set({ column: v })} />
          </label>
        </>}
        {!meta1.notNull && g.kind !== "null" && field(t("dg.p.nullPct"), cc.nullPercent, (v) => patchCol(table, column, { nullPercent: Math.min(100, Math.max(0, Math.trunc(num(v)))) }))}
        <div><button type="button" className="btn" onClick={() => resetColumn(table, column)}>{t("dg.reset")}</button></div>
      </div>
    );
  };

  const propsPane = !focus ? null : (
    <div className="dg-pane-right">
      <h2 className="dt-h">{focus.column ? `${focus.table}.${focus.column}` : focus.table}</h2>
      {focus.column ? generatorEditor(focus.table, focus.column) : (
        <div className="dg-props">
          <label className="dg-field"><span>{t("dg.rows")}</span>
            <input className="field dg-rows" value={config[focus.table]?.rows ?? DEFAULT_ROWS} spellCheck={false}
              onChange={(e) => patchTable(focus.table, { rows: Math.max(1, Math.trunc(num(e.target.value, 1))) })} />
          </label>
          <div><button type="button" className="btn" onClick={() => resetTable(focus.table)}>{t("dg.reset")}</button></div>
        </div>
      )}
    </div>
  );

  const objectsStep = (
    <div className="dg-objects">
      <div className="dg-pane-left">
        <h2 className="dt-h">{t("dg.objects")}</h2>
        {metaError && <div className="msg-error">{metaError}</div>}
        {!meta && !metaError && <div className="muted dt-loading"><span className="spinner" />{t("dt.reading")}</div>}
        {meta && meta.length === 0 && <div className="muted">{t("dg.noTables")}</div>}
        {meta && meta.length > 0 && tree}
      </div>
      {propsPane}
    </div>
  );

  const previewStep = (
    <div className="dg-preview">
      <h2 className="dt-h">{t("dg.preview")}</h2>
      <div className="dg-preview-bar">
        <label>{t("dg.table")}</label>
        <Dropdown className="dg-dd" value={previewTable} items={selected.map((n) => ({ value: n, label: n }))} onChange={setPreviewTable} />
        <button type="button" className="btn" disabled={previewBusy} onClick={() => void loadPreview(previewTable)}>{t("dg.regenerate")}</button>
      </div>
      <div className="dg-grid-wrap">
        {preview && (
          <table className="ds-grid dg-grid">
            <thead><tr><th className="dg-rownum" />{preview.columns.map((c) => <th key={c}>{c}</th>)}</tr></thead>
            <tbody>
              {preview.rows.map((r, i) => (
                <tr key={i}>
                  <td className="dg-rownum">{i + 1}</td>
                  {r.map((v, k) => <td key={k} className={v === null ? "is-null" : undefined} title={v ?? ""}>{v === null ? "(null)" : v}</td>)}
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {previewBusy && !preview && <div className="muted dt-loading"><span className="spinner" />{t("dt.reading")}</div>}
      </div>
    </div>
  );

  const runStep = (
    <div className="dt-progress">
      <dl className="dt-stats">
        <dt>{t("dg.stat.objects")}</dt><dd>{stats.objects}</dd>
        <dt>{t("dg.stat.generated")}</dt><dd>{stats.generated.toLocaleString()}</dd>
        <dt>{t("dg.stat.inserted")}</dt><dd>{stats.inserted.toLocaleString()}</dd>
        <dt>{t("ds.stat.errors")}</dt><dd>{stats.errors}</dd>
        <dt>{t("exp.time").replace(":", "")}:</dt><dd>{formatClock(elapsed)}</dd>
      </dl>
      <div className="exp-log dt-log" ref={logRef} role="log" aria-live="polite">
        {log.map((l, i) => <div key={i} className={l.includes("ERROR") ? "is-error" : undefined}>{l}</div>)}
      </div>
    </div>
  );

  /* ---------- Rodapé ---------- */
  const saveItems: MenuEntry[] = [
    { kind: "item", text: t("dt.saveAs"), run: () => setNamePrompt(true) },
    ...(profiles.length ? [{ kind: "sep" } as MenuEntry, ...profiles.map((p): MenuEntry => ({ kind: "item", text: t("exp.profileDelete", { name: p.name }), run: () => persist(profiles.filter((x) => x.name !== p.name)) }))] : []),
  ];
  const loadItems: MenuEntry[] = profiles.length ? profiles.map((p): MenuEntry => ({ kind: "item", text: p.name, run: () => loadProfile(p) })) : [{ kind: "item", text: t("exp.profileNone") }];

  return (
    <div className="dt-window">
      {header}
      <div className="dt-body">
        {step === 0 && connectStep}
        {step === 1 && objectsStep}
        {step === 2 && previewStep}
        {step === 3 && runStep}
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
        {step <= 2 && <button type="button" className="btn dt-wide" onClick={() => setOptionsOpen(true)}>{t("dt.options")}</button>}
        {step === 1 && <button type="button" className="btn dt-wide" disabled={selected.length < 2} onClick={() => setOrderDialog(selected)}>{t("dg.order")}</button>}
        <span className="spacer" />
        {step > 0 && <button type="button" className="btn dt-wide" disabled={running} onClick={() => { setMessage(null); setStep(step - 1); }}>{t("exp.back").replace("< ", "")}</button>}
        {step <= 1 && <button type="button" className="btn btn-primary dt-wide" onClick={() => void goNext()}>{t("exp.next").replace(" >", "")}</button>}
        {step === 2 && <button type="button" className="btn btn-primary dt-wide" onClick={() => void start()}>{t("exp.start")}</button>}
        {step === 3 && (running
          ? <button type="button" className="btn btn-primary dt-wide" onClick={() => void pg.exportCancel(runId.current).catch(() => undefined)}>{t("exp.stop")}</button>
          : <button type="button" className="btn btn-primary dt-wide" onClick={() => void getCurrentWindow().close()}>{t("common.close")}</button>)}
      </div>

      {optionsOpen && (
        <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) setOptionsOpen(false); }}>
          <div className="modal exp-options-dialog" role="dialog" aria-label={t("dt.options")}>
            <div className="modal-header"><span className="modal-title">{t("dt.options")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={() => setOptionsOpen(false)}><SmallIcon name="close" /></button></div>
            <div className="modal-body exp-options">
              <div className="ss-opt-title">{t("dt.options")}</div>
              <label className="check-row"><input type="checkbox" checked={options.continueOnError} onChange={(e) => setOptions({ ...options, continueOnError: e.target.checked })} /><span>{t("ds.opt.continue")}</span></label>
              <label className="check-row"><input type="checkbox" checked={options.emptyFirst} onChange={(e) => setOptions({ ...options, emptyFirst: e.target.checked })} /><span>{t("dg.opt.empty")}</span></label>
              <label className="check-row"><input type="checkbox" checked={options.transaction} onChange={(e) => setOptions({ ...options, transaction: e.target.checked })} /><span>{t("dg.opt.transaction")}</span></label>
              <label className="check-row"><input type="checkbox" checked={options.multiInsert} onChange={(e) => setOptions({ ...options, multiInsert: e.target.checked })} /><span>{t("dg.opt.multi")}</span></label>
            </div>
            <div className="modal-footer"><span className="spacer" /><button type="button" className="btn btn-primary" onClick={() => setOptionsOpen(false)}>OK</button><button type="button" className="btn" onClick={() => setOptionsOpen(false)}>{t("common.cancel")}</button></div>
          </div>
        </div>
      )}
      {orderDialog && <OrderDialog items={orderDialog} onClose={() => setOrderDialog(null)} onOk={(items) => {
        // As tabelas marcadas trocam de lugar entre si; as demais ficam onde estão.
        let k = 0;
        setOrder((o) => o.map((n) => (items.includes(n) ? items[k++] : n)));
        setOrderDialog(null);
      }} />}
      {passwordDialog}
      {namePrompt && <MessageDialog title={t("exp.saveProfile")} prompt={{ label: t("exp.profileName"), initial: "" }} okLabel={t("exp.saveProfile")} onOk={(n) => { if (n) saveProfile(n); }} onCancel={() => setNamePrompt(false)} />}
    </div>
  );
}

/** "Table Generation Order": a ordem em que as tabelas marcadas são geradas. */
function OrderDialog({ items, onClose, onOk }: { items: string[]; onClose: () => void; onOk: (items: string[]) => void }) {
  const t = useT();
  const [list, setList] = useState(items);
  const [sel, setSel] = useState(0);
  const move = (dir: -1 | 1) => {
    const to = sel + dir;
    if (to < 0 || to >= list.length) return;
    const next = [...list];
    [next[sel], next[to]] = [next[to], next[sel]];
    setList(next);
    setSel(to);
  };
  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal dg-order" role="dialog" aria-label={t("dg.order")}>
        <div className="modal-header"><span className="modal-title">{t("dg.order")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="modal-body ss-reorder-body">
          <div className="ss-reorder-list">
            {list.map((n, i) => (
              <div key={n} className={`ss-reorder-row${sel === i ? " is-sel" : ""}`} onClick={() => setSel(i)}><SmallIcon name="table" /><span>{n}</span></div>
            ))}
          </div>
        </div>
        <div className="modal-footer">
          <button type="button" className="icon-button" aria-label="↑" onClick={() => move(-1)}>↑</button>
          <button type="button" className="icon-button" aria-label="↓" onClick={() => move(1)}>↓</button>
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={() => onOk(list)}>OK</button>
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
        </div>
      </div>
    </div>
  );
}
