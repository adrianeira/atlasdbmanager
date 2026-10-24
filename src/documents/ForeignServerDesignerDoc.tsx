import { useEffect, useMemo, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type ForeignServerDetail } from "../backend";
import { MessageDialog } from "../components/MessageDialog";
import { OptionsGrid } from "../components/OptionsGrid";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import {
  alterServerStatements, createServerStatements, emptyServer, fromDetail, validateServer, type ServerProblem, type ServerState,
} from "../sql/foreignServer";

/** Acesso ao banco real; ausente = modo de demonstração (o wrapper é digitado). */
export interface ForeignServerBackend {
  loadDetail: (name: string) => Promise<ForeignServerDetail>;
  /** Wrappers de dados externos instalados no banco. */
  loadWrappers: () => Promise<string[]>;
  /** Executa o DDL numa transação. */
  apply: (statements: string[]) => Promise<void>;
}

interface Props {
  /** Ausente = servidor novo. */
  serverName?: string;
  /** Papéis do servidor, para sugerir o usuário de um mapeamento. */
  roles: string[];
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string) => void;
  backend?: ForeignServerBackend;
}

type SubTab = "general" | "options" | "mappings" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "general", label: "role.general" },
  { id: "options", label: "fsv.options" },
  { id: "mappings", label: "fsv.mappings" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

type Dialog = null | { kind: "invalid"; problem: ServerProblem } | { kind: "review"; statements: string[] };

export function ForeignServerDesignerDoc({ serverName, roles, active, production, onStatus, onDirty, onSaved, backend }: Props) {
  const t = useT();
  const [name, setName] = useState(serverName);
  const isNew = !name;
  const [original, setOriginal] = useState<ServerState | null>(null);
  const [d, setD] = useState<ServerState>(() => emptyServer());
  const [wrappers, setWrappers] = useState<string[]>([]);
  const [loading, setLoading] = useState(!!backend);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("general");
  const [mapSel, setMapSel] = useState(0);
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(serverName ? "save" : "saveAs");
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
    if (!backend) return;
    let alive = true;
    const task = Promise.all([backend.loadWrappers().then((w) => alive && setWrappers(w)), serverName ? backend.loadDetail(serverName).then((det) => {
      if (!alive) return;
      const st = fromDetail(det);
      setOriginal(st);
      setD(st);
    }) : Promise.resolve()]);
    task.catch((e) => alive && setLoadError(errorText(toAtlasError(e)))).finally(() => alive && setLoading(false));
    return () => { alive = false; };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const patch = (p: Partial<ServerState>) => setD((x) => ({ ...x, ...p }));
  const createStatements = useMemo(() => (d.name.trim() && d.wrapper.trim() ? createServerStatements(d) : []), [d]);
  const alterStatements = useMemo(() => (original && !isNew ? alterServerStatements(original, d) : []), [original, d, isNew]);
  const dirty = isNew ? d.name.trim() !== "" || d.wrapper.trim() !== "" : alterStatements.length > 0;
  useEffect(() => onDirty(dirty), [dirty, onDirty]);
  useEffect(() => {
    if (active) onStatus([{ text: dirty ? t("des.unsaved") : "", grow: true }]);
  }, [active, dirty, t, onStatus]);

  const save = () => {
    const problem = validateServer(d);
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

  const mappingUsers = [...new Set(["public", ...roles])].filter((u) => !d.mappings.some((m) => m.user.toLowerCase() === u.toLowerCase()));
  const mapping = d.mappings[Math.min(mapSel, d.mappings.length - 1)];
  const setMapping = (i: number, p: Partial<(typeof d.mappings)[number]>) => patch({ mappings: d.mappings.map((m, k) => (k === i ? { ...m, ...p } : m)) });

  const body = () => {
    if (tab === "general") {
      return (
        <div className="options-form">
          {prop("fn.name", <input className="field prop-field" value={d.name} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />)}
          {prop("fsv.wrapper", isNew ? (
            wrappers.length || backend ? (
              <select className="field prop-field" value={d.wrapper} onChange={(e) => patch({ wrapper: e.target.value })}>
                <option value="" />
                {wrappers.map((w) => <option key={w} value={w}>{w}</option>)}
              </select>
            ) : (
              <input className="field prop-field" value={d.wrapper} spellCheck={false} onChange={(e) => patch({ wrapper: e.target.value })} />
            )
          ) : <input className="field prop-field" value={d.wrapper} readOnly spellCheck={false} />, !isNew ? t("fsv.wrapperFixed") : undefined)}
          {isNew && backend && !wrappers.length && <div className="prop-row"><span /><div className="muted ext-note">{t("fsv.noWrapper")}</div></div>}
          {prop("fsv.type", <input className="field prop-field" value={d.type} readOnly={!isNew} spellCheck={false} onChange={(e) => patch({ type: e.target.value })} />, !isNew ? t("fsv.typeFixed") : undefined)}
          {prop("ext.version", <input className="field prop-field" value={d.version} spellCheck={false} onChange={(e) => patch({ version: e.target.value })} />)}
          {prop("des.owner", <input className="field prop-field" value={d.owner} list="fs-roles" spellCheck={false} onChange={(e) => patch({ owner: e.target.value })} />)}
          <datalist id="fs-roles">{roles.map((r) => <option key={r} value={r} />)}</datalist>
        </div>
      );
    }
    if (tab === "options") return <OptionsGrid label={t("fsv.options")} rows={d.options} onChange={(options) => patch({ options })} />;
    if (tab === "mappings") {
      return (
        <div className="fs-mappings">
          <div className="fs-users">
            <div className="fs-head"><span>{t("fsv.user")}</span></div>
            <div className="fs-rows">
              {d.mappings.map((m, i) => (
                <div key={i} className={`fs-row${mapSel === i ? " is-sel" : ""}`} onClick={() => setMapSel(i)}>
                  <input className="field" value={m.user} list="fs-users" spellCheck={false} aria-label={t("fsv.user")} onChange={(e) => setMapping(i, { user: e.target.value })} />
                </div>
              ))}
            </div>
            <datalist id="fs-users">{mappingUsers.map((u) => <option key={u} value={u} />)}</datalist>
            <div className="fs-tools">
              <button type="button" className="icon-button" aria-label="+" onClick={() => { patch({ mappings: [...d.mappings, { user: mappingUsers[0] ?? "", options: [] }] }); setMapSel(d.mappings.length); }}>+</button>
              <button type="button" className="icon-button" aria-label="−" disabled={!d.mappings.length} onClick={() => { patch({ mappings: d.mappings.filter((_, k) => k !== mapSel) }); setMapSel(Math.max(0, mapSel - 1)); }}>−</button>
            </div>
          </div>
          <div className="fs-map-options">
            {mapping ? <OptionsGrid label={t("fsv.options")} rows={mapping.options} onChange={(options) => setMapping(Math.min(mapSel, d.mappings.length - 1), { options })} /> : <div className="muted">{t("fsv.noMappings")}</div>}
          </div>
        </div>
      );
    }
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || isNew ? createStatements : alterStatements;
    return <SqlEditor value={d.name.trim() && statements.length ? statements.join("\n\n") : t("des.noChanges")} readOnly />;
  };

  const problemText: Record<ServerProblem, MessageKey> = {
    name: "fsv.needName", wrapper: "fsv.needWrapper", optionKey: "fsv.needOptionKey", mappingUser: "fsv.needUser", mappingDuplicate: "fsv.duplicateUser",
  };

  return (
    <div className="doc">
      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      <div className="object-toolbar" role="toolbar">
        {tool("save", "des.save", dirty && !loadError ? save : undefined)}
        <div className="ot-spacer" />
        <button type="button" className="icon-button" title={t("notImplemented")} disabled><SmallIcon name="fullscreen" /></button>
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

      {dialog?.kind === "invalid" && <MessageDialog title={t("fsv.title")} message={t(problemText[dialog.problem])} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />}
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
