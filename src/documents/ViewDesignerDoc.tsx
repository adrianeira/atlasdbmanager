import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type QueryOutcome, type ViewDetail } from "../backend";
import { DataGrid, typeKind, type GridColumn } from "../components/DataGrid";
import { MessageDialog } from "../components/MessageDialog";
import { SqlEditor, type SqlEditorHandle } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import type { CellValue } from "../model";
import { beautifySql } from "../sql/format";
import {
  alterMatviewStatements, alterViewStatements, cleanDefinition, createMatviewStatements, createViewStatements, emptyView, refreshMatviewSql, type ViewState,
} from "../sql/view";

/** Acesso ao banco real; ausente = modo de demonstração. */
export interface ViewBackend {
  loadDetail: () => Promise<ViewDetail>;
  execute: (sql: string, maxRows: number) => Promise<QueryOutcome>;
  /** Executa o DDL numa transação. */
  apply: (statements: string[]) => Promise<void>;
}

interface Props {
  schema: string;
  /** Ausente = nova view. */
  viewName?: string;
  /** View materializada (PostgreSQL 9.3+): outro DDL, sem regras/check option e com "Atualizar dados". */
  materialized?: boolean;
  /** Definição de demonstração (sem banco). */
  mock?: Partial<ViewState>;
  owners: string[];
  serverVersionNum: number;
  active: boolean;
  production: boolean;
  completionSchema?: Record<string, string[]>;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string, state: ViewState) => void;
  backend?: ViewBackend;
}

type SubTab = "definition" | "rules" | "advanced" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "definition", label: "vw.definition" },
  { id: "rules", label: "des.rules" },
  { id: "advanced", label: "vw.advanced" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

/** Linhas máximas no "Pré-visualizar" (como no editor de consultas). */
const PREVIEW_MAX_ROWS = 1000;

interface Outcome {
  sql: string;
  ms: number;
  result?: { columns: GridColumn[]; rows: CellValue[][] };
  message: string;
  error?: string;
}

function fromDetail(v: ViewDetail): ViewState {
  return {
    definition: v.definition,
    owner: v.owner ?? "",
    comment: v.comment ?? "",
    checkOption: v.checkOption === "local" ? "LOCAL" : v.checkOption === "cascaded" ? "CASCADED" : "",
    securityBarrier: v.securityBarrier,
    withData: v.populated,
  };
}

export function ViewDesignerDoc({
  schema, viewName, materialized, mock, owners, serverVersionNum, active, production, completionSchema, onStatus, onDirty, onSaved, backend,
}: Props) {
  const t = useT();
  const editor = useRef<SqlEditorHandle | null>(null);
  const [name, setName] = useState(viewName);
  const [original, setOriginal] = useState<ViewState | null>(viewName && !backend ? { ...emptyView(), ...mock } : null);
  const [d, setD] = useState<ViewState>(() => ({ ...emptyView(), ...(viewName ? mock : undefined) }));
  const [rules, setRules] = useState<ViewDetail["rules"]>([]);
  const [loading, setLoading] = useState(!!backend && !!viewName);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("definition");
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(viewName ? "save" : "saveAs");
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [resultTab, setResultTab] = useState<0 | 1>(0);
  const [activeRow, setActiveRow] = useState(0);
  const [running, setRunning] = useState(false);
  const [dialog, setDialog] = useState<null | { kind: "name" } | { kind: "review"; statements: string[]; newName?: string } | { kind: "empty" }>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!backend || !viewName) return;
    let alive = true;
    backend
      .loadDetail()
      .then((v) => {
        if (!alive) return;
        const s = fromDetail(v);
        setOriginal(s);
        setD(s);
        setRules(v.rules);
      })
      .catch((e) => alive && setLoadError(errorText(toAtlasError(e))))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const patch = (p: Partial<ViewState>) => setD((x) => ({ ...x, ...p }));
  const isNew = !name;
  const alterStatements = useMemo(
    () => (name && original
      ? materialized ? alterMatviewStatements(schema, name, original, d) : alterViewStatements(schema, name, original, d, serverVersionNum)
      : []),
    [name, original, d, schema, serverVersionNum, materialized],
  );
  const dirty = isNew ? cleanDefinition(d.definition) !== "" : alterStatements.length > 0;

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

  /** Pré-visualizar / Explain: roda o SELECT (ou o EXPLAIN dele) sem criar nada. */
  const run = useCallback((explain: boolean) => {
    const def = cleanDefinition(d.definition);
    if (!def || running) return;
    const sqlText = explain ? `EXPLAIN ${def}` : def;
    setTab("definition");
    if (!backend) {
      setOutcome({ sql: sqlText, ms: 0, message: t("vw.mockPreview") });
      setResultTab(0);
      return;
    }
    setRunning(true);
    void (async () => {
      try {
        const out = await backend.execute(sqlText, PREVIEW_MAX_ROWS);
        const rs = out.statements.find((s) => s.result)?.result;
        const lines = [sqlText, `> OK${rs ? ` — ${t("query.rowsReturned", { n: rs.rows.length })}${rs.truncated ? ` (${t("query.truncated", { n: PREVIEW_MAX_ROWS })})` : ""}` : ""}`];
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
  }, [d.definition, running, backend, t]);

  const beautify = () => {
    const text = editor.current?.getText() ?? "";
    if (text.trim()) editor.current?.replaceAll(beautifySql(text));
  };

  const createStatements = (n: string) => (materialized ? createMatviewStatements(schema, n, d) : createViewStatements(schema, n, d, serverVersionNum));

  /** REFRESH MATERIALIZED VIEW; o resultado aparece na área de mensagens. */
  const refreshData = (concurrently: boolean) => {
    if (!name || !backend || running) return;
    const sqlText = refreshMatviewSql(schema, name, concurrently);
    setTab("definition");
    setRunning(true);
    void (async () => {
      try {
        const out = await backend.execute(sqlText, 1);
        setOutcome({ sql: sqlText, ms: out.elapsedMs, message: `${sqlText}\n> OK — ${t("vw.refreshed")}` });
        setOriginal((o) => (o ? { ...o, withData: true } : o));
        setD((x) => ({ ...x, withData: true }));
      } catch (e) {
        setOutcome({ sql: sqlText, ms: 0, message: "", error: errorText(toAtlasError(e)) });
      } finally {
        setResultTab(0);
        setRunning(false);
      }
    })();
  };

  const save = () => {
    if (!cleanDefinition(d.definition)) return setDialog({ kind: "empty" });
    if (isNew) return setDialog({ kind: "name" });
    setDialog({ kind: "review", statements: alterStatements });
  };

  const commit = async (statements: string[], newName?: string) => {
    const finalName = newName ?? name;
    if (!finalName) return;
    setSaving(true);
    setSaveError(null);
    try {
      if (backend) await backend.apply(statements);
    } catch (e) {
      setSaveError(errorText(toAtlasError(e)));
      setSaving(false);
      return;
    }
    setSaving(false);
    setDialog(null);
    // Guarda a definição limpa como base das próximas comparações.
    const saved = { ...d, definition: cleanDefinition(d.definition) };
    setOriginal(saved);
    setName(finalName);
    setPreviewMode("save");
    onSaved(finalName, saved);
  };

  const tool = (icon: SmallIconName, label: MessageKey, onClick?: () => void) => (
    <button type="button" className="ot-button" disabled={!onClick} onClick={onClick} title={onClick ? t(label) : t("notImplemented")}>
      <SmallIcon name={icon} />
      <span>{t(label)}</span>
    </button>
  );

  const prop = (label: MessageKey, control: ReactNode, disabledNote?: string) => (
    <div className={`prop-row${disabledNote ? " is-disabled" : ""}`} title={disabledNote}>
      <label>{t(label)}:</label>
      {control}
    </div>
  );

  if (loading) return <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>;

  const hasDef = !!cleanDefinition(d.definition);
  const needs = (v: number, label: string) => (serverVersionNum && serverVersionNum < v ? t("vw.needsVersion", { v: label }) : undefined);
  const checkNote = needs(90400, "9.4");
  const barrierNote = needs(90200, "9.2");

  const body = () => {
    if (tab === "definition") {
      return (
        <div className="query-body">
          <div className="query-editor" style={{ flexBasis: outcome ? "55%" : "100%" }}>
            <SqlEditor
              value={d.definition}
              schema={completionSchema}
              defaultSchema={schema}
              placeholder={t(materialized ? "vw.matPlaceholder" : "vw.placeholder")}
              onChange={(v) => patch({ definition: v })}
              onRun={() => run(false)}
              handleRef={(h) => { editor.current = h; }}
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
    if (tab === "rules" && !materialized) {
      const text = rules.length ? rules.map((r) => r.definition).join("\n\n") : t("vw.noRules");
      return <SqlEditor value={text} readOnly />;
    }
    if (tab === "advanced") {
      return (
        <div className="options-form">
          {prop("des.owner", (
            <select className="field prop-field" value={d.owner} onChange={(e) => patch({ owner: e.target.value })}>
              <option value="" />
              {[...new Set([...(d.owner ? [d.owner] : []), ...owners])].map((o) => <option key={o}>{o}</option>)}
            </select>
          ))}
          {materialized ? (
            <label className={`check-row${isNew ? "" : " is-disabled"}`} title={isNew ? undefined : t("vw.withDataExisting")}>
              <input type="checkbox" checked={d.withData} disabled={!isNew} onChange={(e) => patch({ withData: e.target.checked })} />
              <span>{t("vw.withData")}</span>
            </label>
          ) : (<>
          {prop("vw.checkOption", (
            <select className="field prop-field" value={d.checkOption} disabled={!!checkNote} onChange={(e) => patch({ checkOption: e.target.value as ViewState["checkOption"] })}>
              <option value="" />
              <option>LOCAL</option>
              <option>CASCADED</option>
            </select>
          ), checkNote)}
          <label className={`check-row${barrierNote ? " is-disabled" : ""}`} title={barrierNote}>
            <input type="checkbox" checked={d.securityBarrier} disabled={!!barrierNote} onChange={(e) => patch({ securityBarrier: e.target.checked })} />
            <span>{t("vw.securityBarrier")}</span>
          </label>
          </>)}
        </div>
      );
    }
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || !name
      ? createStatements(name ?? "Untitled")
      : alterStatements;
    const text = hasDef && statements.length ? statements.join("\n\n") : t("des.noChanges");
    return <SqlEditor value={text} readOnly />;
  };

  return (
    <div className="doc">
      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      <div className="object-toolbar" role="toolbar">
        {tool("save", "des.save", dirty && !loadError ? save : undefined)}
        <span className="ot-sep" />
        {tool("preview", "vw.preview", hasDef && !running ? () => run(false) : undefined)}
        {tool("explain", "query.explain", hasDef && !running ? () => run(true) : undefined)}
        <span className="ot-sep" />
        {materialized && !isNew && (<>
          {tool("refresh", "act.refreshMatview", backend && !running ? () => refreshData(false) : undefined)}
          {tool("refresh", "act.refreshMatviewConcurrently", backend && !running ? () => refreshData(true) : undefined)}
          <span className="ot-sep" />
        </>)}
        {tool("queryBuilder", "vw.builder")}
        {tool("beautify", "query.beautify", tab === "definition" && hasDef ? beautify : undefined)}
        {running && <span className="spinner" />}
        <div className="ot-spacer" />
        <button type="button" className="icon-button" title={t("notImplemented")} disabled><SmallIcon name="fullscreen" /></button>
      </div>

      <div className="dlg-tabs des-tabs" role="tablist">
        {SUBTABS.filter((x) => !(materialized && x.id === "rules")).map((x) => (
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

      {dialog?.kind === "empty" && (
        <MessageDialog title={t(materialized ? "vw.matSaveTitle" : "vw.saveTitle")} message={t("vw.empty")} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />
      )}
      {dialog?.kind === "name" && (
        <MessageDialog
          title={t(materialized ? "vw.matSaveTitle" : "vw.saveTitle")}
          prompt={{ label: t(materialized ? "vw.matViewName" : "vw.viewName"), initial: "" }}
          onOk={(value) => {
            const n = value?.trim();
            if (!n) return;
            setDialog({ kind: "review", statements: createStatements(n), newName: n });
          }}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "review" && (
        <MessageDialog
          title={t("des.reviewTitle")}
          message={t(materialized && !dialog.newName && dialog.statements.some((x) => x.startsWith("DROP ")) ? "vw.matRecreate" : "des.reviewMessage")}
          sql={dialog.statements.join("\n\n")}
          confirmText={production && !dialog.newName ? name : undefined}
          okLabel={t("des.save")}
          busy={saving}
          error={saveError ?? undefined}
          onOk={() => void commit(dialog.statements, dialog.newName)}
          onCancel={() => { setSaveError(null); setDialog(null); }}
        />
      )}
    </div>
  );
}
