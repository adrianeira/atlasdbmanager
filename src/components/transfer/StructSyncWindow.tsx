import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useRef, useState } from "react";
import type { MenuEntry } from "../../commands";
import {
  errorText, local, onSyncDeployProgress, onSyncLog, onSyncProgress, pg, toAtlasError,
  type DataTransferParams, type OpenScriptRequest, type StructKind, type StructObject, type StructOp, type StructPart, type StructSelection,
} from "../../backend";
import { useT } from "../../i18n";
import { SmallIcon, type SmallIconName } from "../../icons";
import { diffLines } from "../../sql/linediff";
import { Dropdown } from "../Dropdown";
import { MessageDialog } from "../MessageDialog";
import { PopupMenu } from "../PopupMenu";
import { SqlEditor } from "../SqlEditor";
import { ToolWindow } from "../ToolWindow";
import { useEndpoints } from "./endpoints";
import { formatClock } from "./transferModel";

type Params = DataTransferParams & { title: string };

/** O que está marcado de um objeto: ele mesmo e, nas tabelas, as partes (colunas, restrições...) que mudam. */
interface Pick {
  checked: boolean;
  parts: Set<number>;
}

interface DeployOptions {
  continueOnError: boolean;
  includeQuery: boolean;
  /** Opções da comparação: donos (`OWNER TO`) e privilégios (`GRANT`/`REVOKE`). */
  compareOwners: boolean;
  comparePrivileges: boolean;
}

interface Statement {
  sql: string;
  on: boolean;
}

interface StructProfile {
  name: string;
  source: { connId: string; database: string; schema: string };
  target: { connId: string; database: string; schema: string };
  options: DeployOptions;
}

const PROFILES_KEY = "structsync.profiles";
const KINDS: StructKind[] = ["table", "ftable", "view", "matview", "sequence", "function", "type"];
const OPS: Exclude<StructOp, "same">[] = ["add", "modify", "delete"];

const keyOf = (o: { kind: string; name: string }) => `${o.kind}:${o.name}`;
const OP_GLYPH: Record<StructOp, string> = { add: "+", delete: "−", modify: "→", same: "=" };

const KIND_ICON: Record<StructKind, SmallIconName> = { table: "table", ftable: "table", view: "view", matview: "matview", sequence: "sequence", function: "function", type: "type" };
const PART_ICON: Record<string, SmallIconName> = {
  privileges: "privilege", column: "column", primary: "columnPk", foreign: "columnFk", unique: "key", check: "key", constraint: "key", index: "columns", trigger: "function", options: "tableDesign",
};

export function StructSyncWindow() {
  const params = (window as unknown as { __ATLAS_SS__: Params }).__ATLAS_SS__;
  return (
    <ToolWindow title={params.title}>
      <Wizard params={params} />
    </ToolWindow>
  );
}

const isModifiedTable = (o: StructObject) => (o.kind === "table" || o.kind === "ftable") && o.op === "modify";
const changedParts = (o: StructObject) => o.parts.flatMap((p, i) => (p.op === "same" ? [] : [i]));

function Wizard({ params }: { params: Params }) {
  const t = useT();
  const [step, setStep] = useState(0);
  const [objects, setObjects] = useState<StructObject[] | null>(null);
  const [picks, setPicks] = useState<Record<string, Pick>>({});
  const [group, setGroup] = useState<"operation" | "type">("operation");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set(["op:same", "type:same"]));
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<string | null>(null);
  const [focusPart, setFocusPart] = useState<number | null>(null);
  const [bottom, setBottom] = useState<"ddl" | "script">("ddl");
  const [preview, setPreview] = useState<string[]>([]);
  const [comparing, setComparing] = useState<{ text: string; fraction: number } | null>(null);
  const compareId = useRef("");

  const [stmts, setStmts] = useState<Statement[]>([]);
  const [tab, setTab] = useState<"script" | "log">("script");
  const [deployOpts, setDeployOpts] = useState<DeployOptions>({ continueOnError: false, includeQuery: false, compareOwners: false, comparePrivileges: false });
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [reorder, setReorder] = useState<Statement[] | null>(null);
  const [menu, setMenu] = useState<"save" | "load" | "deploy" | null>(null);
  const [profiles, setProfiles] = useState<StructProfile[]>([]);
  const [namePrompt, setNamePrompt] = useState(false);
  const [running, setRunning] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [deployResult, setDeployResult] = useState<{ errors: number; rolledBack: boolean } | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const deployId = useRef("");
  const startedAt = useRef(0);
  const logRef = useRef<HTMLDivElement>(null);

  const {
    source, target, targetConn, message, setMessage, header, connectStep, passwordDialog, endpointProblem, setSource, setTarget,
  } = useEndpoints(params, { allowFile: false, onSourceChange: () => { setObjects(null); } });

  useEffect(() => { void local.getPref<StructProfile[]>(PROFILES_KEY).then((p) => setProfiles(Array.isArray(p) ? p : [])).catch(() => undefined); }, []);
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
  useEffect(() => () => { if (compareId.current) void pg.structForget(compareId.current).catch(() => undefined); }, []);

  /* ---------- Seleção ---------- */
  const selectionOf = (list: StructObject[] | null = objects, all = picks): StructSelection[] =>
    (list ?? []).filter((o) => o.op !== "same" && all[keyOf(o)]?.checked).map((o) => ({
      kind: o.kind, name: o.name, parts: isModifiedTable(o) ? [...all[keyOf(o)].parts] : null,
    }));

  const initialPicks = (list: StructObject[]): Record<string, Pick> =>
    Object.fromEntries(list.filter((o) => o.op !== "same").map((o) => [keyOf(o), { checked: true, parts: new Set(isModifiedTable(o) ? changedParts(o) : []) }]));

  const patchPick = (o: StructObject, p: (x: Pick) => Pick) => setPicks((all) => ({ ...all, [keyOf(o)]: p(all[keyOf(o)] ?? { checked: false, parts: new Set() }) }));

  const toggleObject = (o: StructObject, on: boolean) => patchPick(o, (x) => ({ checked: on, parts: on && isModifiedTable(o) && x.parts.size === 0 ? new Set(changedParts(o)) : x.parts }));
  const togglePart = (o: StructObject, i: number) => patchPick(o, (x) => {
    const parts = new Set(x.parts);
    if (parts.has(i)) parts.delete(i); else parts.add(i);
    return { checked: parts.size > 0 || x.checked, parts };
  });

  /* ---------- Comparação ---------- */
  const compare = async () => {
    setMessage(null);
    if (compareId.current) void pg.structForget(compareId.current).catch(() => undefined);
    const id = crypto.randomUUID();
    compareId.current = id;
    setComparing({ text: "", fraction: 0 });
    const unlisten = await onSyncProgress(id, (text, fraction) => setComparing({ text, fraction }));
    try {
      const list = await pg.structCompare({
        compareId: id, sourceConn: source.connId, sourceDatabase: source.database, targetConn: target.connId, targetDatabase: target.database,
        spec: { sourceSchema: source.schema, targetSchema: target.schema, compareOwners: deployOpts.compareOwners, comparePrivileges: deployOpts.comparePrivileges },
      });
      setObjects(list);
      setPicks(initialPicks(list));
      setFocus(null);
      setFocusPart(null);
      setExpanded(new Set());
      setStmts([]);
      setStep(1);
    } catch (e) {
      const err = toAtlasError(e);
      if (err.kind !== "canceled") setMessage(errorText(err));
    } finally {
      unlisten();
      setComparing(null);
    }
  };

  const stopCompare = () => void pg.exportCancel(compareId.current).catch(() => undefined);

  /* ---------- Script ---------- */
  useEffect(() => {
    if (step !== 1 || bottom !== "script" || !objects || !compareId.current) return;
    let alive = true;
    pg.structScript(compareId.current, selectionOf()).then((s) => alive && setPreview(s)).catch(() => alive && setPreview([]));
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, bottom, objects, picks]);

  const prepareScript = async () => {
    try {
      const list = await pg.structScript(compareId.current, selectionOf());
      if (list.length === 0) {
        setMessage(t((objects ?? []).every((o) => o.op === "same") ? "ss.nothingSame" : "ss.nothing"));
        return;
      }
      setMessage(null);
      setStmts(list.map((sql) => ({ sql, on: true })));
      setTab("script");
      setStep(2);
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  };

  const scriptText = stmts.filter((s) => s.on).map((s) => s.sql).join("\n\n");

  const deploy = async () => {
    if (running) return;
    const statements = stmts.filter((s) => s.on).map((s) => s.sql);
    const id = crypto.randomUUID();
    deployId.current = id;
    setTab("log");
    setLog([]);
    setProgress({ done: 0, total: statements.length });
    setDeployResult(null);
    setElapsed(0);
    startedAt.current = performance.now();
    setRunning(true);
    const unlistenLog = await onSyncLog(id, (line) => setLog((l) => [...l, line]));
    const unlistenProgress = await onSyncDeployProgress(id, (done, total) => setProgress({ done, total }));
    try {
      const r = await pg.structDeploy({
        deployId: id, targetConn: target.connId, targetDatabase: target.database, statements,
        continueOnError: deployOpts.continueOnError, includeQuery: deployOpts.includeQuery,
      });
      setProgress({ done: r.done + r.errors, total: r.total });
      setDeployResult({ errors: r.errors, rolledBack: r.rolledBack });
    } catch (e) {
      const err = toAtlasError(e);
      setLog((l) => [...l, err.kind === "canceled" ? `--${t("dt.log.canceled")}--` : `[ERR] ${errorText(err)}`]);
      setDeployResult({ errors: err.kind === "canceled" ? 0 : 1, rolledBack: false });
    } finally {
      unlistenLog();
      unlistenProgress();
    }
    setElapsed(performance.now() - startedAt.current);
    setRunning(false);
  };

  const copyScript = () => void navigator.clipboard.writeText(scriptText).catch((e) => setMessage(errorText(toAtlasError(e))));
  const openInEditor = () => {
    const req: OpenScriptRequest = { connId: target.connId, database: target.database, schema: target.schema, sql: scriptText };
    void emit("tool-open-query", req).catch((e) => setMessage(errorText(toAtlasError(e))));
  };

  /* ---------- Navegação ---------- */
  const goNext = async () => {
    setMenu(null);
    if (step === 0) {
      const p = endpointProblem();
      if (p) { setMessage(p); return; }
      await compare();
    } else if (step === 1) {
      await prepareScript();
    }
  };

  const pct = progress.total ? (progress.done / progress.total) * 100 : running ? 0 : 100;
  useEffect(() => {
    void getCurrentWindow().setTitle(step === 2 && tab === "log" ? `${Math.round(pct)}% - ${t("ss.title")}` : `* ${t("ss.title")}`).catch(() => undefined);
  }, [step, tab, pct, t]);

  /* ---------- Perfis ---------- */
  const persist = (next: StructProfile[]) => { setProfiles(next); void local.setPref(PROFILES_KEY, next).catch(() => undefined); };
  const saveProfile = (name: string) => {
    persist([...profiles.filter((x) => x.name !== name), { name, source: { ...source }, target: { ...target }, options: deployOpts }].sort((a, b) => a.name.localeCompare(b.name)));
    setNamePrompt(false);
  };
  const loadProfile = (p: StructProfile) => {
    setSource(p.source);
    setTarget(p.target);
    setDeployOpts({ ...p.options, compareOwners: p.options.compareOwners ?? false, comparePrivileges: p.options.comparePrivileges ?? false });
    setObjects(null);
    setStep(0);
  };

  /* ---------- Árvore de objetos ---------- */
  const focusObject = objects?.find((o) => keyOf(o) === focus) ?? null;
  const diff = useMemo(() => diffLines(focusObject?.sourceDdl ?? "", focusObject?.targetDdl ?? ""), [focusObject]);
  const marksLeft = useMemo(() => ({ add: diff.left }), [diff]);
  const marksRight = useMemo(() => ({ remove: diff.right }), [diff]);

  const groups = useMemo(() => {
    const list = objects ?? [];
    if (group === "operation") {
      return [...OPS, "same" as const].map((op) => ({ id: `op:${op}`, label: t(`ss.group.${op}`), op, items: list.filter((o) => o.op === op) }));
    }
    return KINDS.map((k) => ({ id: `type:${k}`, label: t(`ss.type.${k}`), op: undefined, items: list.filter((o) => o.kind === k) }));
  }, [objects, group, t]);

  const toggleSet = (set: Set<string>, setter: (s: Set<string>) => void, key: string) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key); else next.add(key);
    setter(next);
  };

  const partName = (p: StructPart) => (p.kind === "options" ? t("ss.tableOptions") : p.kind === "privileges" ? t("ss.privileges") : p.name);
  const opCell = (op: StructOp) => <td className="ss-op-cell"><span className={`ss-op ss-op-${op}`}>{OP_GLYPH[op]}</span></td>;

  const renderObject = (o: StructObject) => {
    const k = keyOf(o);
    const pick = picks[k];
    const changed = o.op !== "same";
    const open = expanded.has(k) && o.parts.length > 0;
    return [
      <tr key={k} className={`ss-row${focus === k && focusPart === null ? " is-focus" : ""}${changed ? "" : " is-muted"}`} onClick={() => { setFocus(k); setFocusPart(null); }}>
        <td><div className="ss-line">
          <span className="ss-indent" style={{ width: 22 }} />
          {o.parts.length > 0
            ? <button type="button" className="ss-chevron" aria-label={k} onClick={(e) => { e.stopPropagation(); toggleSet(expanded, setExpanded, k); }}><SmallIcon name={open ? "chevronDown" : "chevronRight"} size={10} /></button>
            : <span className="ss-chevron" />}
          <input type="checkbox" disabled={!changed} checked={changed && !!pick?.checked} aria-label={o.name} onClick={(e) => e.stopPropagation()} onChange={(e) => toggleObject(o, e.target.checked)} />
          <SmallIcon name={KIND_ICON[o.kind]} />
          <span className="ss-text">{o.op === "delete" ? "" : o.name}</span>
        </div></td>
        {opCell(o.op)}
        <td><div className="ss-line">{o.op === "add" ? "" : <><SmallIcon name={KIND_ICON[o.kind]} /><span className="ss-text">{o.name}</span></>}</div></td>
      </tr>,
      ...(open
        ? o.parts.map((p, i) => {
            const isSame = p.op === "same";
            return (
              <tr key={`${k}#${i}`} className={`ss-row ss-part${focus === k && focusPart === i ? " is-focus" : ""}${isSame ? " is-muted" : ""}`} onClick={() => { setFocus(k); setFocusPart(i); }}>
                <td><div className="ss-line">
                  <span className="ss-indent" style={{ width: 54 }} />
                  <input type="checkbox" disabled={isSame || !pick?.checked} checked={!isSame && !!pick?.parts.has(i)} aria-label={p.name} onClick={(e) => e.stopPropagation()} onChange={() => togglePart(o, i)} />
                  <SmallIcon name={PART_ICON[p.kind] ?? "column"} />
                  <span className="ss-text">{p.op === "delete" ? "" : partName(p)}</span>
                </div></td>
                {opCell(p.op)}
                <td><div className="ss-line"><span className="ss-indent" style={{ width: 32 }} />{p.op === "add" ? "" : <><SmallIcon name={PART_ICON[p.kind] ?? "column"} /><span className="ss-text">{partName(p)}</span></>}</div></td>
              </tr>
            );
          })
        : []),
    ];
  };

  const renderGroup = (g: (typeof groups)[number]) => {
    if (g.items.length === 0) return null;
    const closed = collapsed.has(g.id);
    const selectable = g.items.filter((o) => o.op !== "same");
    const on = selectable.filter((o) => picks[keyOf(o)]?.checked).length;
    return [
      <tr key={g.id} className="ss-group-row">
        <td colSpan={3}><div className="ss-line">
          <button type="button" className="ss-chevron" aria-label={g.label} onClick={() => toggleSet(collapsed, setCollapsed, g.id)}><SmallIcon name={closed ? "chevronRight" : "chevronDown"} size={10} /></button>
          {selectable.length > 0 && (
            <input
              type="checkbox" aria-label={g.label} checked={on === selectable.length}
              ref={(el) => { if (el) el.indeterminate = on > 0 && on < selectable.length; }}
              onChange={(e) => selectable.forEach((o) => toggleObject(o, e.target.checked))}
            />
          )}
          {g.op && <span className={`ss-op ss-op-${g.op}`}>{OP_GLYPH[g.op]}</span>}
          <strong>{g.label}</strong>
          <span className="muted"> {selectable.length > 0 ? `(${t("ss.selected", { n: on, total: selectable.length })})` : `(${g.items.length})`}</span>
        </div></td>
      </tr>,
      ...(closed ? [] : g.items.flatMap(renderObject)),
    ];
  };

  const resultStep = (
    <div className="ss-result">
      <div className="ss-tree-wrap">
        <table className="ss-tree">
          <colgroup><col /><col style={{ width: 100 }} /><col /></colgroup>
          <thead><tr><th>{t("ss.colSource")}</th><th>{t("ss.colOperation")}</th><th>{t("ss.colTarget")}</th></tr></thead>
          <tbody>{groups.map(renderGroup)}</tbody>
        </table>
      </div>
      <div className="ss-bottom">
        <div className="ss-subtabs" role="tablist">
          <button type="button" role="tab" aria-selected={bottom === "ddl"} className={`ds-tab${bottom === "ddl" ? " is-active" : ""}`} onClick={() => setBottom("ddl")}>{t("ss.tab.ddl")}</button>
          <button type="button" role="tab" aria-selected={bottom === "script"} className={`ds-tab${bottom === "script" ? " is-active" : ""}`} onClick={() => setBottom("script")}>{t("ds.tab.script")}</button>
        </div>
        {bottom === "ddl" ? (
          <div className="ss-panes">
            {(["source", "target"] as const).map((side) => (
              <div key={side} className="ss-pane">
                <div className="ds-pane-title">{focusObject && (side === "source" ? focusObject.sourceDdl : focusObject.targetDdl) ? <><SmallIcon name={KIND_ICON[focusObject.kind]} /> {focusObject.name}</> : " "}</div>
                <div className="ss-editor">
                  <SqlEditor value={focusObject ? (side === "source" ? focusObject.sourceDdl : focusObject.targetDdl) : ""} readOnly lineMarks={side === "source" ? marksLeft : marksRight} />
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="ss-editor ss-preview"><SqlEditor value={preview.join("\n\n")} readOnly /></div>
        )}
      </div>
    </div>
  );

  /* ---------- Etapa do script e do registro ---------- */
  const scriptTab = (
    <div className="ds-script">
      <div className="ds-script-head">
        <SmallIcon name="database" size={26} />
        <div className="ds-script-target"><strong>{targetConn?.name ?? ""}</strong><span>{target.database}.{target.schema}</span></div>
        <span className="muted">{t("ds.runOn")}</span>
        <span className="spacer" />
        <button type="button" className="ds-link" onClick={copyScript}>{t("ds.copyScript")}</button>
      </div>
      <div className="ds-script-body"><SqlEditor value={scriptText} readOnly /></div>
    </div>
  );

  const logTab = (
    <div className="dt-progress">
      <dl className="dt-stats">
        <dt>{t("ds.stat.progress")}</dt><dd>{progress.done.toLocaleString()}/{progress.total.toLocaleString()} ({pct.toFixed(1).replace(".", ",")}%)</dd>
        <dt>{t("ds.stat.success")}</dt><dd>{Math.max(0, progress.done - (deployResult?.errors ?? 0))}</dd>
        <dt>{t("ds.stat.errors")}</dt><dd>{deployResult?.errors ?? 0}</dd>
        <dt>{t("exp.time").replace(":", "")}:</dt><dd>{formatClock(elapsed)}</dd>
      </dl>
      <div className="exp-log dt-log" ref={logRef} role="log" aria-live="polite">
        {log.map((l, i) => <div key={i} className={l.startsWith("[ERR]") ? "is-error" : undefined}>{l}</div>)}
      </div>
    </div>
  );

  /* ---------- Rodapé ---------- */
  const saveItems: MenuEntry[] = [
    { kind: "item", text: t("dt.saveAs"), run: () => setNamePrompt(true) },
    ...(profiles.length ? [{ kind: "sep" } as MenuEntry, ...profiles.map((p): MenuEntry => ({ kind: "item", text: t("exp.profileDelete", { name: p.name }), run: () => persist(profiles.filter((x) => x.name !== p.name)) }))] : []),
  ];
  const loadItems: MenuEntry[] = profiles.length ? profiles.map((p): MenuEntry => ({ kind: "item", text: p.name, run: () => loadProfile(p) })) : [{ kind: "item", text: t("exp.profileNone") }];
  const deployItems: MenuEntry[] = [
    { kind: "item", text: `${t("ss.optTitle")}...`, run: () => setOptionsOpen(true) },
    { kind: "item", text: t("ss.reorder"), run: () => setReorder(stmts.map((s) => ({ ...s }))) },
    { kind: "item", text: t("ss.openEditor"), run: openInEditor },
  ];

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
      {step === 2 && (
        <div className="ds-tabs" role="tablist">
          <button type="button" role="tab" aria-selected={tab === "script"} className={`ds-tab${tab === "script" ? " is-active" : ""}`} onClick={() => setTab("script")}>{t("ds.tab.script")}</button>
          <button type="button" role="tab" aria-selected={tab === "log"} className={`ds-tab${tab === "log" ? " is-active" : ""}`} onClick={() => setTab("log")}>{t("ds.tab.log")}</button>
        </div>
      )}
      {step === 1 ? (
        <div className="ss-head">
          <Dropdown className="ss-group" value={group} items={[{ value: "operation", label: t("ss.groupOp") }, { value: "type", label: t("ss.groupType") }]} onChange={(v) => setGroup(v as "operation" | "type")} />
          {header}
        </div>
      ) : header}
      <div className="dt-body">
        {step === 0 && connectStep}
        {step === 1 && resultStep}
        {step === 2 && (tab === "script" ? scriptTab : logTab)}
        {message && <div className="msg-error dt-message" role="alert">{message}</div>}
      </div>
      <div className="dt-footer">
        {footerCommon}
        {step === 0 && <button type="button" className="btn dt-wide" onClick={() => setOptionsOpen(true)}>{t("dt.options")}</button>}
        {step === 2 && tab === "script" && (
          <div className="dt-menu-wrap split-button">
            <button type="button" className="btn dt-wide" onClick={() => setMenu((m) => (m === "deploy" ? null : "deploy"))}>{t("ds.deployOptions")}</button>
            <button type="button" className="btn dt-arrow-btn" aria-label={t("ds.deployOptions")} onClick={() => setMenu((m) => (m === "deploy" ? null : "deploy"))}><SmallIcon name="chevronDown" size={10} /></button>
            {menu === "deploy" && <PopupMenu items={deployItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%", left: 0 }} />}
          </div>
        )}
        <span className="spacer" />
        {step > 0 && <button type="button" className="btn dt-wide" disabled={running} onClick={() => { setMessage(null); setStep(step - 1); }}>{t("exp.back").replace("< ", "")}</button>}
        {step >= 1 && <button type="button" className="btn dt-wide" disabled={running || !!comparing} onClick={() => void compare()}>{t("ds.recompare")}</button>}
        {step <= 1 && <button type="button" className="btn btn-primary dt-wide" disabled={!!comparing} onClick={() => void goNext()}>{t(step === 0 ? "ss.compare" : "exp.next").replace(" >", "")}</button>}
        {step === 2 && (running
          ? <button type="button" className="btn btn-primary dt-wide" onClick={() => void pg.exportCancel(deployId.current).catch(() => undefined)}>{t("exp.stop")}</button>
          : <button type="button" className="btn btn-primary dt-wide" onClick={() => void deploy()}>{t("exp.start")}</button>)}
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
      {optionsOpen && (
        <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) setOptionsOpen(false); }}>
          <div className="modal exp-options-dialog" role="dialog" aria-label={t("dt.options")}>
            <div className="modal-header"><span className="modal-title">{t("dt.options")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={() => setOptionsOpen(false)}><SmallIcon name="close" /></button></div>
            <div className="modal-body exp-options">
              <div className="ss-opt-title">{t("ss.optCompare")}</div>
              <label className="check-row"><input type="checkbox" checked={deployOpts.compareOwners} onChange={(e) => setDeployOpts({ ...deployOpts, compareOwners: e.target.checked })} /><span>{t("ss.opt.owners")}</span></label>
              <label className="check-row"><input type="checkbox" checked={deployOpts.comparePrivileges} onChange={(e) => setDeployOpts({ ...deployOpts, comparePrivileges: e.target.checked })} /><span>{t("ss.opt.privileges")}</span></label>
              <div className="ss-opt-title">{t("ss.optTitle")}</div>
              <label className="check-row"><input type="checkbox" checked={deployOpts.continueOnError} onChange={(e) => setDeployOpts({ ...deployOpts, continueOnError: e.target.checked })} /><span>{t("ds.opt.continue")}</span></label>
              <label className="check-row"><input type="checkbox" checked={deployOpts.includeQuery} onChange={(e) => setDeployOpts({ ...deployOpts, includeQuery: e.target.checked })} /><span>{t("ss.opt.query")}</span></label>
            </div>
            <div className="modal-footer"><span className="spacer" /><button type="button" className="btn btn-primary" onClick={() => setOptionsOpen(false)}>OK</button><button type="button" className="btn" onClick={() => setOptionsOpen(false)}>{t("common.cancel")}</button></div>
          </div>
        </div>
      )}
      {reorder && <ReorderDialog items={reorder} onClose={() => setReorder(null)} onOk={(items) => { setStmts(items); setReorder(null); }} />}
      {passwordDialog}
      {namePrompt && <MessageDialog title={t("exp.saveProfile")} prompt={{ label: t("exp.profileName"), initial: "" }} okLabel={t("exp.saveProfile")} onOk={(n) => { if (n) saveProfile(n); }} onCancel={() => setNamePrompt(false)} />}
    </div>
  );
}

/** "Reorder Script": cada comando pode ser desligado e a ordem muda com as setas. */
function ReorderDialog({ items, onClose, onOk }: { items: Statement[]; onClose: () => void; onOk: (items: Statement[]) => void }) {
  const t = useT();
  const [list, setList] = useState<Statement[]>(items);
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
      <div className="modal ss-reorder" role="dialog" aria-label={t("ss.reorderTitle")}>
        <div className="modal-header"><span className="modal-title">{t("ss.reorderTitle")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="modal-body ss-reorder-body">
          <div className="ss-reorder-list">
            {list.map((s, i) => (
              <div key={i} className={`ss-reorder-row${sel === i ? " is-sel" : ""}`} onClick={() => setSel(i)}>
                <input type="checkbox" checked={s.on} aria-label={String(i + 1)} onChange={(e) => setList(list.map((x, k) => (k === i ? { ...x, on: e.target.checked } : x)))} />
                <span title={s.sql}>{s.sql.replace(/\s+/g, " ")}</span>
              </div>
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
