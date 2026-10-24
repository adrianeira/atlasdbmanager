import { useEffect, useMemo, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type AvailableExtension, type ExtensionDetail } from "../backend";
import { MessageDialog } from "../components/MessageDialog";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import { FullscreenButton } from "../components/FullscreenButton";
import {
  alterExtStatements, compareVersions, createExtStatements, emptyExt, fromDetail, validateExt, type ExtState,
} from "../sql/extension";

/** Acesso ao banco real; ausente = modo de demonstração (o nome da extensão nova é digitado). */
export interface ExtensionBackend {
  loadDetail: (name: string) => Promise<ExtensionDetail>;
  /** Extensões que o servidor oferece e ainda não estão instaladas no banco. */
  loadAvailable: () => Promise<AvailableExtension[]>;
  /** Executa o DDL numa transação. */
  apply: (statements: string[]) => Promise<void>;
}

interface Props {
  /** Schema da aba: o padrão de uma extensão nova. */
  schema: string;
  /** Schemas do banco para o campo Schema. */
  schemas: string[];
  /** Ausente = extensão nova. */
  extName?: string;
  serverVersionNum: number;
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string) => void;
  backend?: ExtensionBackend;
}

type SubTab = "definition" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "definition", label: "fn.definition" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

type Dialog = null | { kind: "invalid" } | { kind: "review"; statements: string[] };

export function ExtensionDesignerDoc({ schema, schemas, extName, serverVersionNum, active, production, onStatus, onDirty, onSaved, backend }: Props) {
  const t = useT();
  const [name, setName] = useState(extName);
  const isNew = !name;
  const [original, setOriginal] = useState<ExtState | null>(null);
  const [detail, setDetail] = useState<ExtensionDetail | null>(null);
  const [available, setAvailable] = useState<AvailableExtension[]>([]);
  const [d, setD] = useState<ExtState>(() => emptyExt(schema));
  const [loading, setLoading] = useState(!!backend);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("definition");
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(extName ? "save" : "saveAs");
  const [dialog, setDialog] = useState<Dialog>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const load = async (n: string) => {
    if (!backend) return;
    const det = await backend.loadDetail(n);
    const st = fromDetail(det);
    setDetail(det);
    setOriginal(st);
    setD(st);
  };

  useEffect(() => {
    if (!backend) return;
    let alive = true;
    const task = extName
      ? backend.loadDetail(extName).then((det) => {
          if (!alive) return;
          const st = fromDetail(det);
          setDetail(det);
          setOriginal(st);
          setD(st);
        })
      : backend.loadAvailable().then((list) => alive && setAvailable(list));
    task.catch((e) => alive && setLoadError(errorText(toAtlasError(e)))).finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const patch = (p: Partial<ExtState>) => setD((x) => ({ ...x, ...p }));
  const chosen = isNew ? available.find((a) => a.name === d.name) : undefined;
  const fixedSchema = chosen?.fixedSchema ?? null;
  const createStatements = useMemo(
    () => (d.name.trim() ? createExtStatements(d, { fixedSchema, supportsCascade: serverVersionNum === 0 || serverVersionNum >= 90600 }) : []),
    [d, fixedSchema, serverVersionNum],
  );
  const alterStatements = useMemo(() => (original && !isNew ? alterExtStatements(original, d) : []), [original, d, isNew]);
  const dirty = isNew ? d.name.trim() !== "" : alterStatements.length > 0;
  useEffect(() => onDirty(dirty), [dirty, onDirty]);

  useEffect(() => {
    if (!active) return;
    onStatus([{ text: dirty ? t("des.unsaved") : "", grow: true }]);
  }, [active, dirty, t, onStatus]);

  const save = () => {
    if (validateExt(d)) return setDialog({ kind: "invalid" });
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
    const finalName = (name ?? d.name).trim();
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
  const note = (text: string) => (
    <div className="prop-row"><span /><div className="muted ext-note">{text}</div></div>
  );

  if (loading) return <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>;

  const schemaItems = [...new Set([d.schema, ...schemas].filter(Boolean))];
  const versions = detail ? [detail.version, ...[...detail.updateVersions].sort(compareVersions)] : [];
  const body = () => {
    if (tab === "definition") {
      return (
        <div className="options-form">
          {isNew ? (
            <>
              {prop("fn.name", available.length || backend ? (
                <select
                  className="field prop-field"
                  value={d.name}
                  onChange={(e) => {
                    const pick = available.find((a) => a.name === e.target.value);
                    patch({ name: e.target.value, version: "", schema: pick?.fixedSchema ?? d.schema });
                  }}
                >
                  <option value="" />
                  {available.map((a) => <option key={a.name} value={a.name}>{a.name}</option>)}
                </select>
              ) : (
                <input className="field prop-field" value={d.name} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />
              ))}
              {backend && !available.length && note(t("ext.noneAvailable"))}
              {chosen && chosen.comment && note(chosen.comment)}
              {prop("ext.version", (
                <input className="field prop-field" value={d.version} placeholder={chosen ? t("ext.defaultVersion", { v: chosen.defaultVersion }) : ""} spellCheck={false} onChange={(e) => patch({ version: e.target.value })} />
              ))}
              {prop("info.schema", (
                <select className="field prop-field" value={d.schema} disabled={!!fixedSchema} onChange={(e) => patch({ schema: e.target.value })}>
                  {schemaItems.map((s) => <option key={s}>{s}</option>)}
                </select>
              ), fixedSchema ? t("ext.fixedSchema", { schema: fixedSchema }) : undefined)}
              {(serverVersionNum === 0 || serverVersionNum >= 90600) && (
                <label className="check-row">
                  <input type="checkbox" checked={d.cascade} onChange={(e) => patch({ cascade: e.target.checked })} />
                  <span>{t("ext.cascade")}</span>
                </label>
              )}
            </>
          ) : (
            <>
              {prop("fn.name", <input className="field prop-field" value={d.name} readOnly spellCheck={false} />)}
              {prop("ext.version", (
                <select className="field prop-field" value={d.version} disabled={versions.length < 2} onChange={(e) => patch({ version: e.target.value })}>
                  {versions.map((v) => <option key={v}>{v}</option>)}
                </select>
              ), versions.length < 2 ? t("ext.noUpdate") : undefined)}
              {prop("info.schema", (
                <select className="field prop-field" value={d.schema} disabled={!detail?.relocatable} onChange={(e) => patch({ schema: e.target.value })}>
                  {schemaItems.map((s) => <option key={s}>{s}</option>)}
                </select>
              ), detail && !detail.relocatable ? t("ext.notRelocatable") : undefined)}
              {prop("des.owner", <input className="field prop-field" value={detail?.owner ?? ""} readOnly spellCheck={false} />)}
            </>
          )}
        </div>
      );
    }
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || isNew ? createStatements : alterStatements;
    return <SqlEditor value={d.name.trim() && statements.length ? statements.join("\n\n") : t("des.noChanges")} readOnly />;
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

      {dialog?.kind === "invalid" && <MessageDialog title={t("ext.title")} message={t("ext.needName")} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />}
      {dialog?.kind === "review" && (
        <MessageDialog
          title={t("des.reviewTitle")}
          message={[t("des.reviewMessage"), !isNew && original && d.version !== original.version ? t("ext.updateNote") : ""].filter(Boolean).join("\n\n")}
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
