import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type FunctionDetail, type QueryOutcome } from "../backend";
import { ComboInput } from "../components/ComboInput";
import { DataGrid, typeKind, type GridColumn } from "../components/DataGrid";
import { EditTable, type EditColumn } from "../components/EditTable";
import { MessageDialog } from "../components/MessageDialog";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import { FullscreenButton } from "../components/FullscreenButton";
import type { CellValue } from "../model";
import {
  alterFuncStatements, callSql, createFuncStatements, DEFAULT_PLPGSQL_BODY, emptyFunc, fromDetail, identityList, PARAM_MODES,
  type FuncParam, type FuncState,
} from "../sql/func";

/** Acesso ao banco real; ausente = modo de demonstração. */
export interface FunctionBackend {
  loadDetail: (name: string, identity: string) => Promise<FunctionDetail>;
  execute: (sql: string, maxRows: number) => Promise<QueryOutcome>;
  /** Executa o DDL numa transação. */
  apply: (statements: string[]) => Promise<void>;
}

interface Props {
  schema: string;
  /** Ausentes = função nova. */
  fnName?: string;
  identity?: string;
  owners: string[];
  serverVersionNum: number;
  active: boolean;
  production: boolean;
  completionSchema?: Record<string, string[]>;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  /** `identity` é o texto do servidor (`pg_get_function_identity_arguments`) da função recém-gravada. */
  onSaved: (name: string, identity: string) => void;
  backend?: FunctionBackend;
}

type SubTab = "definition" | "body" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "definition", label: "fn.definition" },
  { id: "body", label: "fn.body" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

const LANGUAGES = ["plpgsql", "sql", "plpython3u", "plperl", "pltcl", "c", "internal"];
const RESULT_MAX_ROWS = 1000;

interface Outcome {
  sql: string;
  ms: number;
  result?: { columns: GridColumn[]; rows: CellValue[][] };
  message: string;
  error?: string;
}

const sqlLiteral = (s: string) => `'${s.replace(/'/g, "''")}'`;
/** Compara listas de argumentos ignorando espaços e maiúsculas. */
const normalizeArgs = (s: string) => s.replace(/\s+/g, "").toLowerCase();

export function FunctionDesignerDoc({
  schema, fnName, identity: identityProp, owners, serverVersionNum, active, production, completionSchema, onStatus, onDirty, onSaved, backend,
}: Props) {
  const t = useT();
  const [name, setName] = useState(fnName);
  const [identity, setIdentity] = useState(identityProp ?? "");
  const isNew = !name;
  const [original, setOriginal] = useState<FuncState | null>(null);
  const [d, setD] = useState<FuncState>(() => ({ ...emptyFunc(), body: DEFAULT_PLPGSQL_BODY }));
  const [loading, setLoading] = useState(!!backend && !!fnName);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("definition");
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(fnName ? "save" : "saveAs");
  const [selectedParam, setSelectedParam] = useState(0);
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [resultTab, setResultTab] = useState<0 | 1>(0);
  const [activeRow, setActiveRow] = useState(0);
  const [running, setRunning] = useState(false);
  const [runValues, setRunValues] = useState<string[] | null>(null);
  const [dialog, setDialog] = useState<null | { kind: "review"; statements: string[] } | { kind: "invalid"; message: MessageKey }>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!backend || !fnName) return;
    let alive = true;
    backend
      .loadDetail(fnName, identityProp ?? "")
      .then((detail) => {
        if (!alive) return;
        const s = fromDetail(detail);
        setOriginal(s);
        setD(s);
        setIdentity(detail.identity);
      })
      .catch((e) => alive && setLoadError(errorText(toAtlasError(e))))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const patch = (p: Partial<FuncState>) => setD((x) => ({ ...x, ...p }));
  const patchParam = (i: number, p: Partial<FuncParam>) => setD((x) => ({ ...x, params: x.params.map((q, j) => (j === i ? { ...q, ...p } : q)) }));

  const alterStatements = useMemo(
    () => (name && original ? alterFuncStatements(schema, identity, original, d) : []),
    [name, original, d, schema, identity],
  );
  const dirty = isNew ? d.name.trim() !== "" : alterStatements.length > 0;
  useEffect(() => onDirty(dirty), [dirty, onDirty]);

  useEffect(() => {
    if (!active) return;
    const cells: StatusCell[] = [];
    if (outcome) {
      cells.push({ text: outcome.sql.replace(/\s+/g, " ").slice(0, 160), grow: true });
      cells.push({ text: t("query.time", { s: (outcome.ms / 1000).toFixed(3) }), width: 150 });
      if (outcome.result) cells.push({ text: t("data.record", { n: outcome.result.rows.length ? activeRow + 1 : 0, total: outcome.result.rows.length, page: 1 }), width: 200 });
    } else {
      cells.push({ text: dirty ? t("des.unsaved") : "", grow: true });
    }
    onStatus(cells);
  }, [active, outcome, activeRow, dirty, t, onStatus]);

  const inputParams = d.params.filter((p) => p.mode !== "OUT");

  /** Executa a função salva (a versão do servidor) com os valores informados. */
  const run = useCallback((values: string[]) => {
    if (!backend || running) return;
    const sqlText = callSql(schema, d, values);
    setTab("body");
    setRunning(true);
    void (async () => {
      try {
        const out = await backend.execute(sqlText, RESULT_MAX_ROWS);
        const rs = out.statements.find((s) => s.result)?.result;
        const lines = [sqlText, `> OK${rs ? ` — ${t("query.rowsReturned", { n: rs.rows.length })}` : ""}`];
        setOutcome({
          sql: sqlText,
          ms: out.elapsedMs,
          message: lines.join("\n"),
          result: rs
            ? { columns: rs.columns.map((c) => ({ name: c.name, type: c.typeName ?? "", kind: typeKind(c.typeName ?? "") })), rows: rs.rows }
            : undefined,
        });
        setResultTab(rs ? 1 : 0);
      } catch (e) {
        const err = toAtlasError(e);
        const pos = err.position ? `\n> ${t("query.errorPosition", { n: err.position })}` : "";
        setOutcome({ sql: sqlText, ms: 0, message: "", error: `${errorText(err)}${pos}` });
        setResultTab(0);
      } finally {
        setActiveRow(0);
        setRunning(false);
      }
    })();
  }, [backend, running, schema, d, t]);

  const requestRun = () => {
    if (inputParams.length === 0) run([]);
    else setRunValues(inputParams.map(() => ""));
  };

  const save = () => {
    if (!d.name.trim()) return setDialog({ kind: "invalid", message: "fn.needName" });
    if (d.params.some((p) => !p.type.trim())) return setDialog({ kind: "invalid", message: "fn.needParamType" });
    if (!d.procedure && !d.returns.trim() && !d.params.some((p) => p.mode === "OUT" || p.mode === "INOUT")) {
      return setDialog({ kind: "invalid", message: "fn.needReturns" });
    }
    setDialog({ kind: "review", statements: isNew ? createFuncStatements(schema, d) : alterStatements });
  };

  /** Depois do DDL: pergunta ao servidor a identidade da função (o texto dele é a chave dela na árvore). */
  const resolveIdentity = async (fn: string, params: FuncParam[]): Promise<string> => {
    const guess = identityList(params);
    if (!backend) return guess;
    try {
      const out = await backend.execute(
        `SELECT pg_get_function_identity_arguments(p.oid) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace ` +
        `WHERE n.nspname = ${sqlLiteral(schema)} AND p.proname = ${sqlLiteral(fn)}`, 100,
      );
      const rows = out.statements.find((s) => s.result)?.result?.rows.map((r) => String(r[0] ?? "")) ?? [];
      return rows.find((r) => normalizeArgs(r) === normalizeArgs(guess)) ?? (rows.length === 1 ? rows[0] : guess);
    } catch {
      return guess;
    }
  };

  const commit = async (statements: string[]) => {
    setSaving(true);
    setSaveError(null);
    try {
      if (backend) await backend.apply(statements);
    } catch (e) {
      setSaveError(errorText(toAtlasError(e)));
      setSaving(false);
      return;
    }
    const finalName = d.name.trim();
    const finalIdentity = await resolveIdentity(finalName, d.params);
    setSaving(false);
    setDialog(null);
    setOriginal({ ...d, name: finalName });
    setName(finalName);
    setIdentity(finalIdentity);
    setPreviewMode("save");
    onSaved(finalName, finalIdentity);
  };

  const tool = (icon: SmallIconName, label: MessageKey, onClick?: () => void) => (
    <button type="button" className="ot-button" disabled={!onClick} onClick={onClick} title={onClick ? t(label) : t("notImplemented")}>
      <SmallIcon name={icon} />
      <span>{t(label)}</span>
    </button>
  );

  const prop = (label: MessageKey, control: ReactNode, note?: string) => (
    <div className={`prop-row${note ? " is-disabled" : ""}`} title={note}>
      <label>{t(label)}:</label>
      {control}
    </div>
  );

  if (loading) return <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>;

  const canProcedure = serverVersionNum === 0 || serverVersionNum >= 110000;
  const paramColumns: EditColumn<FuncParam>[] = [
    { key: "mode", label: t("fn.paramMode"), width: 90, kind: "select", options: PARAM_MODES },
    { key: "name", label: t("fn.paramName"), width: 180, kind: "text" },
    { key: "type", label: t("fn.paramType"), width: 200, kind: "text" },
    { key: "default", label: t("fn.paramDefault"), width: 200, kind: "text" },
  ];

  const body = () => {
    if (tab === "definition") {
      return (
        <div className="options-form">
          {prop("fn.name", <input className="field prop-field" value={d.name} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />)}
          {prop("fn.kind", (
            <select className="field prop-field" value={d.procedure ? "procedure" : "function"} disabled={!isNew || !canProcedure}
              onChange={(e) => patch({ procedure: e.target.value === "procedure", returns: e.target.value === "procedure" ? "" : d.returns || "void" })}>
              <option value="function">{t("fn.function")}</option>
              <option value="procedure">{t("fn.procedure")}</option>
            </select>
          ), !isNew ? t("fn.kindLocked") : !canProcedure ? t("vw.needsVersion", { v: "11" }) : undefined)}
          {!d.procedure && prop("fn.returns", <input className="field prop-field" value={d.returns} spellCheck={false} onChange={(e) => patch({ returns: e.target.value })} />)}
          {prop("col.language", <ComboInput className="prop-field" value={d.language} options={LANGUAGES} onChange={(v) => patch({ language: v })} spellCheck={false} />)}
          {!d.procedure && prop("fn.volatility", (
            <select className="field prop-field" value={d.volatility} onChange={(e) => patch({ volatility: e.target.value as FuncState["volatility"] })}>
              <option>VOLATILE</option>
              <option>STABLE</option>
              <option>IMMUTABLE</option>
            </select>
          ))}
          {prop("des.owner", (
            <select className="field prop-field" value={d.owner} onChange={(e) => patch({ owner: e.target.value })}>
              <option value="" />
              {[...new Set([...(d.owner ? [d.owner] : []), ...owners])].map((o) => <option key={o}>{o}</option>)}
            </select>
          ))}
          {!d.procedure && (
            <label className="check-row">
              <input type="checkbox" checked={d.strict} onChange={(e) => patch({ strict: e.target.checked })} />
              <span>{t("fn.strict")}</span>
            </label>
          )}
          <label className="check-row">
            <input type="checkbox" checked={d.securityDefiner} onChange={(e) => patch({ securityDefiner: e.target.checked })} />
            <span>{t("fn.securityDefiner")}</span>
          </label>
          <div className="fn-params">
            <div className="fn-params-bar">
              <span>{t("fn.params")}</span>
              <button type="button" className="ot-button" onClick={() => { patch({ params: [...d.params, { mode: "IN", name: "", type: "", default: "" }] }); setSelectedParam(d.params.length); }}>
                <SmallIcon name="addRow" /><span>{t("fn.addParam")}</span>
              </button>
              <button type="button" className="ot-button" disabled={!d.params[selectedParam]} onClick={() => { patch({ params: d.params.filter((_, i) => i !== selectedParam) }); setSelectedParam(Math.max(0, selectedParam - 1)); }}>
                <SmallIcon name="removeRow" /><span>{t("fn.deleteParam")}</span>
              </button>
            </div>
            <EditTable<FuncParam>
              columns={paramColumns}
              rows={d.params}
              selected={selectedParam}
              onSelect={setSelectedParam}
              onChange={(i, key, value) => patchParam(i, { [key]: value } as Partial<FuncParam>)}
            />
          </div>
        </div>
      );
    }
    if (tab === "body") {
      return (
        <div className="query-body">
          <div className="query-editor" style={{ flexBasis: outcome ? "55%" : "100%" }}>
            <SqlEditor
              value={d.body}
              schema={completionSchema}
              defaultSchema={schema}
              placeholder={t("fn.bodyPlaceholder")}
              onChange={(v) => patch({ body: v })}
            />
          </div>
          {outcome && (
            <>
              <div className="hsplitter" role="separator" aria-orientation="horizontal" />
              <div className="query-results">
                <div className="result-tabs" role="tablist">
                  <button type="button" role="tab" aria-selected={resultTab === 0} className={`result-tab${resultTab === 0 ? " is-active" : ""}`} onClick={() => setResultTab(0)}>{t("query.message")}</button>
                  {outcome.result && (
                    <button type="button" role="tab" aria-selected={resultTab === 1} className={`result-tab${resultTab === 1 ? " is-active" : ""}`} onClick={() => setResultTab(1)}>{t("query.result", { n: 1 })}</button>
                  )}
                </div>
                <div className="result-body">
                  {resultTab === 0 || !outcome.result ? (
                    <pre className={`result-message${outcome.error ? " is-error" : ""}`}>
                      {outcome.error ? `${outcome.sql}\n> ${outcome.error}` : outcome.message}
                      {"\n> "}{t("query.time", { s: (outcome.ms / 1000).toFixed(3) })}
                    </pre>
                  ) : (
                    <DataGrid columns={outcome.result.columns} rows={outcome.result.rows} readOnly activeRow={activeRow} onActiveRow={setActiveRow} />
                  )}
                </div>
              </div>
            </>
          )}
        </div>
      );
    }
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || isNew ? createFuncStatements(schema, d) : alterStatements;
    const text = d.name.trim() && statements.length ? statements.join("\n\n") : t("des.noChanges");
    return <SqlEditor value={text} readOnly />;
  };

  const runBlockedNote = isNew ? t("fn.runSaveFirst") : dirty ? t("fn.runSaveFirst") : undefined;

  return (
    <div className="doc">
      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      <div className="object-toolbar" role="toolbar">
        {tool("save", "des.save", dirty && !loadError ? save : undefined)}
        <span className="ot-sep" />
        <button type="button" className="ot-button" disabled={!backend || isNew || dirty || running} onClick={requestRun} title={runBlockedNote ?? t("fn.execute")}>
          <SmallIcon name="run" />
          <span>{t("fn.execute")}</span>
        </button>
        {running && <span className="spinner" />}
        <div className="ot-spacer" />
        <FullscreenButton />
      </div>

      <div className="dlg-tabs des-tabs" role="tablist">
        {SUBTABS.map((x) => (
          <button key={x.id} type="button" role="tab" aria-selected={tab === x.id} className={`dlg-tab${tab === x.id ? " is-active" : ""}`} onClick={() => setTab(x.id)}>
            {t(x.label)}
          </button>
        ))}
      </div>

      <div className="des-body">
        <div className="des-main">{body()}</div>
        {tab === "sql" && (
          <div className="des-sql-footer">
            <select className="field" value={previewMode} onChange={(e) => setPreviewMode(e.target.value as "save" | "saveAs")}>
              {!isNew && <option value="save">{t("des.previewSave")}</option>}
              <option value="saveAs">{t("des.previewSaveAs")}</option>
            </select>
          </div>
        )}
      </div>

      {dialog?.kind === "invalid" && (
        <MessageDialog title={t("fn.saveTitle")} message={t(dialog.message)} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />
      )}
      {dialog?.kind === "review" && (
        <MessageDialog
          title={t("des.reviewTitle")}
          message={t(dialog.statements.some((x) => x.startsWith("DROP ")) ? "fn.recreate" : "des.reviewMessage")}
          confirmText={production && !isNew ? name : undefined}
          okLabel={t("des.save")}
          busy={saving}
          error={saveError ?? undefined}
          onOk={() => void commit(dialog.statements)}
          onCancel={() => { setSaveError(null); setDialog(null); }}
        />
      )}
      {runValues && (
        <MessageDialog
          title={t("fn.execute")}
          message={(
            <div className="fn-run-form">
              <div>{t("fn.runHint")}</div>
              {inputParams.map((p, i) => (
                <label key={i} className="prop-row">
                  <span>{p.name || `$${i + 1}`} <small>{p.type}</small></span>
                  <input
                    className="field prop-field"
                    value={runValues[i] ?? ""}
                    placeholder={p.default ? `DEFAULT ${p.default}` : "NULL"}
                    spellCheck={false}
                    onChange={(e) => setRunValues(runValues.map((v, j) => (j === i ? e.target.value : v)))}
                  />
                </label>
              ))}
            </div>
          )}
          okLabel={t("fn.execute")}
          onOk={() => { const v = runValues; setRunValues(null); run(v); }}
          onCancel={() => setRunValues(null)}
        />
      )}
    </div>
  );
}
