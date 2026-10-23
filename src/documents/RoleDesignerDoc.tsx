import { useEffect, useMemo, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type RoleDetail } from "../backend";
import { MessageDialog } from "../components/MessageDialog";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon } from "../icons";
import { alterRoleStatements, createRoleStatements, emptyRole, fromDetail, validateRole, type RoleState } from "../sql/role";

/** Acesso ao banco real; ausente = modo de demonstração. */
export interface RoleBackend {
  loadDetail: (name: string) => Promise<RoleDetail>;
  /** Executa o DDL numa transação. */
  apply: (statements: string[]) => Promise<void>;
}

interface Props {
  /** Ausente = papel novo. */
  roleName?: string;
  /** Todos os papéis do servidor (para as listas de participação). */
  allRoles: string[];
  serverVersionNum: number;
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string) => void;
  backend?: RoleBackend;
}

type SubTab = "general" | "membership" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "general", label: "role.general" },
  { id: "membership", label: "role.membership" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

export function RoleDesignerDoc({ roleName, allRoles, serverVersionNum, active, production, onStatus, onDirty, onSaved, backend }: Props) {
  const t = useT();
  const supportsBypass = serverVersionNum === 0 || serverVersionNum >= 90500;
  const [name, setName] = useState(roleName);
  const isNew = !name;
  const [original, setOriginal] = useState<RoleState | null>(null);
  const [d, setD] = useState<RoleState>(emptyRole);
  const [loading, setLoading] = useState(!!backend && !!roleName);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("general");
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(roleName ? "save" : "saveAs");
  const [filter, setFilter] = useState("");
  const [dialog, setDialog] = useState<null | { kind: "invalid"; message: MessageKey } | { kind: "review" }>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!backend || !roleName) return;
    let alive = true;
    backend
      .loadDetail(roleName)
      .then((det) => {
        if (!alive) return;
        const st = fromDetail(det);
        setOriginal(st);
        setD(st);
      })
      .catch((e) => alive && setLoadError(errorText(toAtlasError(e))))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const patch = (p: Partial<RoleState>) => setD((x) => ({ ...x, ...p }));
  const valid = validateRole(d) === null;
  /** Comandos como vão ao servidor (com a senha) e como aparecem na tela (sem ela). */
  const build = (mask: boolean): string[] => {
    if (!valid) return [];
    return isNew || !original ? createRoleStatements(d, supportsBypass, mask) : alterRoleStatements(original, d, supportsBypass, mask);
  };
  const real = useMemo(() => build(false), [d, original, isNew, valid, supportsBypass]); // eslint-disable-line react-hooks/exhaustive-deps
  const shown = useMemo(() => build(true), [d, original, isNew, valid, supportsBypass]); // eslint-disable-line react-hooks/exhaustive-deps
  const dirty = isNew ? d.name.trim() !== "" : real.length > 0;
  useEffect(() => onDirty(dirty), [dirty, onDirty]);
  useEffect(() => {
    if (active) onStatus([{ text: dirty ? t("des.unsaved") : "", grow: true }]);
  }, [active, dirty, t, onStatus]);

  const save = () => {
    const invalid = validateRole(d);
    if (invalid) return setDialog({ kind: "invalid", message: invalid === "name" ? "role.needName" : "role.badLimit" });
    setDialog({ kind: "review" });
  };

  const commit = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      if (backend) await backend.apply(real);
    } catch (e) {
      setSaveError(errorText(toAtlasError(e)));
      setSaving(false);
      return;
    }
    const finalName = d.name.trim();
    try {
      if (backend) {
        const st = fromDetail(await backend.loadDetail(finalName));
        setOriginal(st);
        setD(st);
      } else {
        setOriginal({ ...d, name: finalName, password: "" });
      }
    } catch {
      setOriginal({ ...d, name: finalName, password: "" });
    }
    setSaving(false);
    setDialog(null);
    setName(finalName);
    setPreviewMode("save");
    onSaved(finalName);
  };

  const prop = (label: MessageKey, control: ReactNode, note?: string) => (
    <div className={`prop-row${note ? " is-disabled" : ""}`} title={note}>
      <label>{t(label)}:</label>
      {control}
    </div>
  );
  const check = (key: "canLogin" | "superuser" | "createDb" | "createRole" | "inherit" | "replication" | "bypassRls", label: MessageKey, note?: string) => (
    <label className={`check-row${note ? " is-disabled" : ""}`} title={note}>
      <input type="checkbox" checked={d[key]} disabled={!!note} onChange={(e) => patch({ [key]: e.target.checked } as Partial<RoleState>)} />
      <span>{t(label)}</span>
    </label>
  );

  if (loading) return <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>;

  const others = allRoles.filter((r) => r !== (original?.name ?? name) && r !== d.name.trim() && r.toLowerCase().includes(filter.trim().toLowerCase()));
  const toggle = (key: "memberOf" | "members", role: string, on: boolean) =>
    patch({ [key]: on ? [...d[key], role] : d[key].filter((x) => x !== role) } as Partial<RoleState>);
  const list = (key: "memberOf" | "members", title: MessageKey) => (
    <div className="role-list">
      <div className="role-list-title">{t(title)}</div>
      <div className="role-list-body">
        {[...new Set([...others, ...d[key].filter((r) => r.toLowerCase().includes(filter.trim().toLowerCase()))])].sort().map((r) => (
          <label key={r} className="check-row">
            <input type="checkbox" checked={d[key].includes(r)} onChange={(e) => toggle(key, r, e.target.checked)} />
            <span>{r}</span>
          </label>
        ))}
      </div>
    </div>
  );

  const body = () => {
    if (tab === "general") {
      return (
        <div className="options-form">
          {prop("fn.name", <input className="field prop-field" value={d.name} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />)}
          {prop("role.password", (
            <input
              className="field prop-field"
              type="password"
              autoComplete="new-password"
              value={d.password}
              placeholder={isNew ? "" : t("role.passwordKeep")}
              onChange={(e) => patch({ password: e.target.value })}
            />
          ))}
          {check("canLogin", "role.canLogin")}
          {check("superuser", "role.superuser")}
          {check("createDb", "role.createDb")}
          {check("createRole", "role.createRole")}
          {check("inherit", "role.inherit")}
          {check("replication", "role.replication")}
          {check("bypassRls", "role.bypassRls", supportsBypass ? undefined : t("vw.needsVersion", { v: "9.5" }))}
          {prop("role.connLimit", <input className={`field prop-field is-num${validateRole(d) === "limit" ? " is-invalid" : ""}`} value={d.connLimit} inputMode="numeric" onChange={(e) => patch({ connLimit: e.target.value })} />)}
          {prop("role.validUntil", <input className="field prop-field" value={d.validUntil} placeholder={t("role.validUntilHint")} spellCheck={false} onChange={(e) => patch({ validUntil: e.target.value })} />)}
        </div>
      );
    }
    if (tab === "membership") {
      return (
        <div className="role-membership">
          <input className="field" value={filter} placeholder={t("role.filter")} onChange={(e) => setFilter(e.target.value)} spellCheck={false} />
          <div className="role-lists">
            {list("memberOf", "role.memberOf")}
            {list("members", "role.members")}
          </div>
          <div className="muted snip-hint">{t("role.membershipNote")}</div>
        </div>
      );
    }
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || isNew ? (valid ? createRoleStatements(d, supportsBypass, true) : []) : shown;
    return <SqlEditor value={d.name.trim() && statements.length ? statements.join("\n\n") : t("des.noChanges")} readOnly />;
  };

  return (
    <div className="doc">
      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      <div className="object-toolbar" role="toolbar">
        <button type="button" className="ot-button" disabled={!dirty || !!loadError} onClick={save} title={t("des.save")}>
          <SmallIcon name="save" /><span>{t("des.save")}</span>
        </button>
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
      {dialog?.kind === "invalid" && <MessageDialog title={t("role.title")} message={t(dialog.message)} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />}
      {dialog?.kind === "review" && (
        <MessageDialog
          title={t("des.reviewTitle")}
          message={original && !isNew && d.name.trim() !== original.name ? `${t("des.reviewMessage")} ${t("role.renameNote")}` : t("des.reviewMessage")}
          confirmText={production && !isNew ? name : undefined}
          okLabel={t("des.save")}
          busy={saving}
          error={saveError ?? undefined}
          onOk={() => void commit()}
          onCancel={() => { setSaveError(null); setDialog(null); }}
        />
      )}
    </div>
  );
}
