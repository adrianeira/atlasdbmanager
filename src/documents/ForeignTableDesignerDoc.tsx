import { useEffect, useMemo, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type ForeignTableDetail, type TableDetail } from "../backend";
import { MessageDialog } from "../components/MessageDialog";
import { OptionsGrid } from "../components/OptionsGrid";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import { FullscreenButton } from "../components/FullscreenButton";
import {
  alterForeignTableStatements, createForeignTableStatements, emptyForeignTable, fromDetail, newColumn, validateForeignTable,
  type ForeignTableProblem, type ForeignTableState,
} from "../sql/foreignTable";

/** Acesso ao banco real; ausente = modo de demonstração. */
export interface ForeignTableBackend {
  loadDetail: (name: string) => Promise<{ detail: ForeignTableDetail; table: TableDetail }>;
  /** Executa o DDL numa transação. */
  apply: (statements: string[]) => Promise<void>;
}

interface Props {
  /** Ausente = tabela nova. */
  tableName?: string;
  schema: string;
  /** Servidores externos do banco. */
  servers: string[];
  roles: string[];
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string) => void;
  backend?: ForeignTableBackend;
}

type SubTab = "columns" | "general" | "options" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "general", label: "role.general" },
  { id: "columns", label: "des.fields" },
  { id: "options", label: "fsv.options" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

type Dialog = null | { kind: "invalid"; problem: ForeignTableProblem } | { kind: "review"; statements: string[] };

export function ForeignTableDesignerDoc({ tableName, schema, servers, roles, active, production, onStatus, onDirty, onSaved, backend }: Props) {
  const t = useT();
  const [name, setName] = useState(tableName);
  const isNew = !name;
  const [original, setOriginal] = useState<ForeignTableState | null>(null);
  const [d, setD] = useState<ForeignTableState>(() => emptyForeignTable());
  const [loading, setLoading] = useState(!!backend && !!tableName);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("general");
  const [colSel, setColSel] = useState(0);
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(tableName ? "save" : "saveAs");
  const [dialog, setDialog] = useState<Dialog>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const load = async (n: string) => {
    if (!backend) return;
    const { detail, table } = await backend.loadDetail(n);
    const st = fromDetail(detail, table);
    setOriginal(st);
    setD(st);
  };

  useEffect(() => {
    if (!backend || !tableName) return;
    let alive = true;
    backend.loadDetail(tableName)
      .then(({ detail, table }) => {
        if (!alive) return;
        const st = fromDetail(detail, table);
        setOriginal(st);
        setD(st);
      })
      .catch((e) => alive && setLoadError(errorText(toAtlasError(e))))
      .finally(() => alive && setLoading(false));
    return () => { alive = false; };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const patch = (p: Partial<ForeignTableState>) => setD((x) => ({ ...x, ...p }));
  const createStatements = useMemo(() => (d.name.trim() && d.server.trim() && d.columns.length ? createForeignTableStatements(schema, d) : []), [d, schema]);
  const alterStatements = useMemo(() => (original && !isNew ? alterForeignTableStatements(schema, original, d) : []), [original, d, isNew, schema]);
  const dirty = isNew ? d.name.trim() !== "" || d.columns.length > 0 : alterStatements.length > 0;
  useEffect(() => onDirty(dirty), [dirty, onDirty]);
  useEffect(() => {
    if (active) onStatus([{ text: dirty ? t("des.unsaved") : "", grow: true }]);
  }, [active, dirty, t, onStatus]);

  const save = () => {
    const problem = validateForeignTable(d);
    if (problem) return setDialog({ kind: "invalid", problem });
    setDialog({ kind: "review", statements: isNew ? createStatements : alterStatements });
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
    try {
      await load(finalName);
    } catch {
      // A gravação deu certo; só a releitura falhou (a aba continua utilizável).
    }
    setSaving(false);
    setDialog(null);
    setName(finalName);
    setPreviewMode("save");
    onSaved(finalName);
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

  const setCol = (i: number, p: Partial<(typeof d.columns)[number]>) => patch({ columns: d.columns.map((c, k) => (k === i ? { ...c, ...p } : c)) });

  const body = () => {
    if (tab === "general") {
      return (
        <div className="options-form">
          {prop("fn.name", <input className="field prop-field" value={d.name} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />)}
          {prop("ftb.server", isNew ? (
            <select className="field prop-field" value={d.server} onChange={(e) => patch({ server: e.target.value })}>
              <option value="" />
              {servers.map((s) => <option key={s} value={s}>{s}</option>)}
            </select>
          ) : <input className="field prop-field" value={d.server} readOnly spellCheck={false} />, !isNew ? t("ftb.serverFixed") : undefined)}
          {isNew && !servers.length && <div className="prop-row"><span /><div className="muted ext-note">{t("ftb.noServer")}</div></div>}
          {prop("des.owner", <input className="field prop-field" value={d.owner} list="ft-roles" spellCheck={false} onChange={(e) => patch({ owner: e.target.value })} />)}
          <datalist id="ft-roles">{roles.map((r) => <option key={r} value={r} />)}</datalist>
        </div>
      );
    }
    if (tab === "columns") {
      return (
        <div className="fs-grid ft-cols" role="group" aria-label={t("des.fields")}>
          <div className="fs-head"><span>{t("ftb.columnName")}</span><span>{t("ftb.columnType")}</span><span>{t("des.notNull")}</span></div>
          <div className="fs-rows">
            {d.columns.map((c, i) => (
              <div key={i} className={`fs-row${colSel === i ? " is-sel" : ""}`} onClick={() => setColSel(i)}>
                <input className="field" value={c.name} spellCheck={false} aria-label={t("ftb.columnName")} onChange={(e) => setCol(i, { name: e.target.value })} />
                <input className="field" value={c.type} spellCheck={false} aria-label={t("ftb.columnType")} onChange={(e) => setCol(i, { type: e.target.value })} />
                <span className="ft-null"><input type="checkbox" checked={c.notNull} aria-label={t("des.notNull")} onChange={(e) => setCol(i, { notNull: e.target.checked })} /></span>
              </div>
            ))}
          </div>
          <div className="fs-tools">
            <button type="button" className="icon-button" aria-label="+" onClick={() => { patch({ columns: [...d.columns, newColumn()] }); setColSel(d.columns.length); }}>+</button>
            <button type="button" className="icon-button" aria-label="−" disabled={!d.columns.length} onClick={() => { patch({ columns: d.columns.filter((_, k) => k !== colSel) }); setColSel(Math.max(0, colSel - 1)); }}>−</button>
          </div>
        </div>
      );
    }
    if (tab === "options") return <OptionsGrid label={t("fsv.options")} rows={d.options} onChange={(options) => patch({ options })} />;
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || isNew ? createStatements : alterStatements;
    return <SqlEditor value={d.name.trim() && statements.length ? statements.join("\n\n") : t("des.noChanges")} readOnly />;
  };

  const problemText: Record<ForeignTableProblem, MessageKey> = {
    name: "ftb.needName", server: "ftb.needServer", noColumns: "ftb.needColumns", columnName: "ftb.needColumnName", columnType: "ftb.needColumnType",
    columnDuplicate: "ftb.duplicateColumn", optionKey: "fsv.needOptionKey",
  };

  return (
    <div className="doc">
      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      <div className="object-toolbar" role="toolbar">
        {tool("save", "des.save", dirty && !loadError ? save : undefined)}
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

      {dialog?.kind === "invalid" && <MessageDialog title={t("ftb.title")} message={t(problemText[dialog.problem])} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />}
      {dialog?.kind === "review" && (
        <MessageDialog
          title={t("des.reviewTitle")}
          message={t("des.reviewMessage")}
          confirmText={production && !isNew ? name : undefined}
          okLabel={t("des.save")}
          busy={saving}
          error={saveError ?? undefined}
          onOk={() => void commit(dialog.statements)}
          onCancel={() => { setSaveError(null); setDialog(null); }}
        />
      )}
    </div>
  );
}
