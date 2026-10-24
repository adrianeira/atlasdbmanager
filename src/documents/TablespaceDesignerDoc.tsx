import { useEffect, useMemo, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type TablespaceDetail } from "../backend";
import { FullscreenButton } from "../components/FullscreenButton";
import { MessageDialog } from "../components/MessageDialog";
import { OptionsGrid } from "../components/OptionsGrid";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import {
  alterTablespaceStatements, createTablespaceStatements, emptyTablespace, fromDetail, isSystemTablespace, validateTablespace,
  type TablespaceProblem, type TablespaceState,
} from "../sql/tablespace";

/** Acesso ao banco real; ausente = modo de demonstração. */
export interface TablespaceBackend {
  loadDetail: (name: string) => Promise<TablespaceDetail>;
  /** Executa o DDL um comando por vez, fora de transação (`CREATE TABLESPACE` não aceita bloco de transação). */
  apply: (statements: string[]) => Promise<void>;
}

interface Props {
  /** Ausente = tablespace novo. */
  tablespaceName?: string;
  roles: string[];
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string) => void;
  backend?: TablespaceBackend;
}

type SubTab = "general" | "options" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "general", label: "role.general" },
  { id: "options", label: "fsv.options" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

type Dialog = null | { kind: "invalid"; problem: TablespaceProblem } | { kind: "review"; statements: string[] };

export function TablespaceDesignerDoc({ tablespaceName, roles, active, production, onStatus, onDirty, onSaved, backend }: Props) {
  const t = useT();
  const [name, setName] = useState(tablespaceName);
  const isNew = !name;
  const system = !!name && isSystemTablespace(name);
  const [original, setOriginal] = useState<TablespaceState | null>(null);
  const [d, setD] = useState<TablespaceState>(() => emptyTablespace());
  const [loading, setLoading] = useState(!!backend && !!tablespaceName);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("general");
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(tablespaceName ? "save" : "saveAs");
  const [dialog, setDialog] = useState<Dialog>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const load = async (n: string) => {
    if (!backend) return;
    const st = fromDetail(await backend.loadDetail(n));
    setOriginal(st);
    setD(st);
  };

  useEffect(() => {
    if (!backend || !tablespaceName) return;
    let alive = true;
    backend.loadDetail(tablespaceName)
      .then((det) => {
        if (!alive) return;
        const st = fromDetail(det);
        setOriginal(st);
        setD(st);
      })
      .catch((e) => alive && setLoadError(errorText(toAtlasError(e))))
      .finally(() => alive && setLoading(false));
    return () => { alive = false; };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const patch = (p: Partial<TablespaceState>) => setD((x) => ({ ...x, ...p }));
  const createStatements = useMemo(() => (d.name.trim() && d.location.trim() ? createTablespaceStatements(d) : []), [d]);
  const alterStatements = useMemo(() => (original && !isNew ? alterTablespaceStatements(original, d) : []), [original, d, isNew]);
  const dirty = isNew ? d.name.trim() !== "" || d.location.trim() !== "" : alterStatements.length > 0;
  useEffect(() => onDirty(dirty), [dirty, onDirty]);
  useEffect(() => {
    if (active) onStatus([{ text: dirty ? t("des.unsaved") : "", grow: true }]);
  }, [active, dirty, t, onStatus]);

  const save = () => {
    const problem = validateTablespace(d, isNew);
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

  const body = () => {
    if (tab === "general") {
      return (
        <div className="options-form">
          {prop("fn.name", <input className="field prop-field" value={d.name} readOnly={system} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />, system ? t("tsp.systemNote") : undefined)}
          {prop("des.owner", <input className="field prop-field" value={d.owner} list="ts-roles" readOnly={system} spellCheck={false} onChange={(e) => patch({ owner: e.target.value })} />, system ? t("tsp.systemNote") : undefined)}
          <datalist id="ts-roles">{roles.map((r) => <option key={r} value={r} />)}</datalist>
          {prop("tsp.location", <input className="field prop-field" value={system ? t("tsp.builtin") : d.location} readOnly={!isNew} spellCheck={false} placeholder={isNew ? "/var/lib/postgresql/ts1" : ""} onChange={(e) => patch({ location: e.target.value })} />, !isNew ? t("tsp.locationFixed") : undefined)}
          {isNew && <div className="prop-row"><span /><div className="muted ext-note">{t("tsp.locationNote")}</div></div>}
        </div>
      );
    }
    if (tab === "options") return <OptionsGrid label={t("fsv.options")} rows={d.options} onChange={(options) => patch({ options })} />;
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || isNew ? createStatements : alterStatements;
    return <SqlEditor value={d.name.trim() && statements.length ? statements.join("\n\n") : t("des.noChanges")} readOnly />;
  };

  const problemText: Record<TablespaceProblem, MessageKey> = {
    name: "tsp.needName", reserved: "tsp.reserved", location: "tsp.needLocation", optionKey: "fsv.needOptionKey",
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

      {dialog?.kind === "invalid" && <MessageDialog title={t("tsp.title")} message={t(problemText[dialog.problem])} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />}
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
