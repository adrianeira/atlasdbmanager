import { useEffect, useMemo, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type TypeDetail } from "../backend";
import { EditTable, type EditColumn } from "../components/EditTable";
import { MessageDialog } from "../components/MessageDialog";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import {
  alterTypeStatements, CREATABLE_KINDS, createTypeStatements, emptyType, fromDetail, isAddValue, validateType,
  type Attr, type DomainCheck, type EnumLabel, type TypeKind, type TypeState,
} from "../sql/typedef";

/** Acesso ao banco real; ausente = modo de demonstração. */
export interface TypeBackend {
  loadDetail: (name: string) => Promise<TypeDetail>;
  /** Executa o DDL numa transação. */
  apply: (statements: string[]) => Promise<void>;
  /** Executa um comando sozinho, fora de transação (`ALTER TYPE ... ADD VALUE` antes do PostgreSQL 12). */
  exec: (sql: string) => Promise<void>;
}

interface Props {
  schema: string;
  /** Ausente = tipo novo. */
  typeName?: string;
  owners: string[];
  serverVersionNum: number;
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string) => void;
  backend?: TypeBackend;
}

type SubTab = "definition" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "definition", label: "fn.definition" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

const KIND_LABEL: Record<TypeKind, MessageKey> = { enum: "ty.enum", composite: "ty.composite", domain: "ty.domain", range: "ty.range" };

export function TypeDesignerDoc({ schema, typeName, owners, serverVersionNum, active, production, onStatus, onDirty, onSaved, backend }: Props) {
  const t = useT();
  const [name, setName] = useState(typeName);
  const isNew = !name;
  const [original, setOriginal] = useState<TypeState | null>(null);
  const [d, setD] = useState<TypeState>(() => emptyType());
  const [loading, setLoading] = useState(!!backend && !!typeName);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("definition");
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(typeName ? "save" : "saveAs");
  const [selected, setSelected] = useState(0);
  const [dialog, setDialog] = useState<null | { kind: "invalid"; message: MessageKey } | { kind: "review"; statements: string[] }>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!backend || !typeName) return;
    let alive = true;
    backend
      .loadDetail(typeName)
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

  const patch = (p: Partial<TypeState>) => setD((x) => ({ ...x, ...p }));
  const readOnlyKind = d.kind === "range";
  const alterStatements = useMemo(
    () => (name && original && !readOnlyKind && validateType(d) === null ? alterTypeStatements(schema, original, d) : []),
    [name, original, d, schema, readOnlyKind],
  );
  const dirty = isNew ? d.name.trim() !== "" : alterStatements.length > 0;
  useEffect(() => onDirty(dirty), [dirty, onDirty]);
  useEffect(() => {
    if (active) onStatus([{ text: dirty ? t("des.unsaved") : "", grow: true }]);
  }, [active, dirty, t, onStatus]);

  const save = () => {
    const invalid = validateType(d);
    if (invalid) {
      const keys = { name: "ty.needName", labels: "ty.badLabels", attrs: "ty.badAttrs", domain: "ty.badDomain" } as const;
      return setDialog({ kind: "invalid", message: keys[invalid] });
    }
    setDialog({ kind: "review", statements: isNew ? createTypeStatements(schema, d) : alterStatements });
  };

  const commit = async (statements: string[]) => {
    setSaving(true);
    setSaveError(null);
    try {
      if (backend) {
        // `ADD VALUE` roda sozinho (antes do PostgreSQL 12 não cabe numa transação); o resto, junto.
        const rest = statements.filter((s) => !isAddValue(s));
        if (rest.length) await backend.apply(rest);
        for (const s of statements.filter(isAddValue)) await backend.exec(s);
      }
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
        setOriginal({ ...d, name: finalName });
      }
    } catch {
      setOriginal({ ...d, name: finalName });
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
  const bar = (title: MessageKey, onAdd: () => void, onDelete: () => void, canDelete: boolean) => (
    <div className="fn-params-bar">
      <span>{t(title)}</span>
      <button type="button" className="ot-button" onClick={onAdd}><SmallIcon name="addRow" /><span>{t("ty.add")}</span></button>
      <button type="button" className="ot-button" disabled={!canDelete} onClick={onDelete}><SmallIcon name="removeRow" /><span>{t("ty.delete")}</span></button>
    </div>
  );

  if (loading) return <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>;

  const labelCols: EditColumn<EnumLabel>[] = [{ key: "value", label: t("ty.label"), width: 360, kind: "text" }];
  const attrCols: EditColumn<Attr>[] = [
    { key: "name", label: t("fn.paramName"), width: 200, kind: "text" },
    { key: "type", label: t("fn.paramType"), width: 240, kind: "text" },
  ];
  const checkCols: EditColumn<DomainCheck>[] = [
    { key: "name", label: t("fn.paramName"), width: 200, kind: "text" },
    { key: "expr", label: t("ty.expression"), width: 360, kind: "text" },
  ];

  const removeAt = <T,>(list: T[], i: number) => list.filter((_, j) => j !== i);
  const lockedLabel = !!d.labels[selected]?.orig;

  const body = () => {
    if (tab === "definition") {
      return (
        <div className="options-form">
          {prop("fn.name", <input className="field prop-field" value={d.name} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />)}
          {prop("fn.kind", (
            <select className="field prop-field" value={d.kind} disabled={!isNew} onChange={(e) => patch({ kind: e.target.value as TypeKind })}>
              {(isNew ? CREATABLE_KINDS : [d.kind]).map((k) => <option key={k} value={k}>{t(KIND_LABEL[k])}</option>)}
            </select>
          ), !isNew ? t("ty.kindLocked") : undefined)}
          {prop("des.owner", (
            <select className="field prop-field" value={d.owner} onChange={(e) => patch({ owner: e.target.value })}>
              <option value="" />
              {[...new Set([...(d.owner ? [d.owner] : []), ...owners])].map((o) => <option key={o}>{o}</option>)}
            </select>
          ))}

          {d.kind === "enum" && (
            <div className="fn-params">
              {bar("ty.labels",
                () => { patch({ labels: [...d.labels, { value: "" }] }); setSelected(d.labels.length); },
                () => { patch({ labels: removeAt(d.labels, selected) }); setSelected(Math.max(0, selected - 1)); },
                !!d.labels[selected] && !lockedLabel)}
              <EditTable<EnumLabel> columns={labelCols} rows={d.labels} selected={selected} onSelect={setSelected}
                onChange={(i, _k, v) => patch({ labels: d.labels.map((l, j) => (j === i ? { ...l, value: String(v) } : l)) })} />
              <div className="muted snip-hint">{t(serverVersionNum !== 0 && serverVersionNum < 100000 ? "ty.enumNoRename" : "ty.enumNote")}</div>
            </div>
          )}

          {d.kind === "composite" && (
            <div className="fn-params">
              {bar("ty.attributes",
                () => { patch({ attrs: [...d.attrs, { name: "", type: "" }] }); setSelected(d.attrs.length); },
                () => { patch({ attrs: removeAt(d.attrs, selected) }); setSelected(Math.max(0, selected - 1)); },
                !!d.attrs[selected])}
              <EditTable<Attr> columns={attrCols} rows={d.attrs} selected={selected} onSelect={setSelected}
                onChange={(i, key, v) => patch({ attrs: d.attrs.map((a, j) => (j === i ? { ...a, [key]: String(v) } : a)) })} />
            </div>
          )}

          {d.kind === "domain" && (
            <>
              {prop("ty.baseType", <input className="field prop-field" value={d.baseType} disabled={!isNew} spellCheck={false} onChange={(e) => patch({ baseType: e.target.value })} />, !isNew ? t("ty.baseLocked") : undefined)}
              {prop("fn.paramDefault", <input className="field prop-field" value={d.default} spellCheck={false} onChange={(e) => patch({ default: e.target.value })} />)}
              <label className="check-row"><input type="checkbox" checked={d.notNull} onChange={(e) => patch({ notNull: e.target.checked })} /><span>{t("ty.notNull")}</span></label>
              <div className="fn-params">
                {bar("ty.checks",
                  () => { patch({ checks: [...d.checks, { name: "", expr: "" }] }); setSelected(d.checks.length); },
                  () => { patch({ checks: removeAt(d.checks, selected) }); setSelected(Math.max(0, selected - 1)); },
                  !!d.checks[selected])}
                <EditTable<DomainCheck> columns={checkCols} rows={d.checks} selected={selected} onSelect={setSelected}
                  onChange={(i, key, v) => patch({ checks: d.checks.map((c, j) => (j === i ? { ...c, [key]: String(v) } : c)) })} />
                <div className="muted snip-hint">{t("ty.checkNote")}</div>
              </div>
            </>
          )}

          {d.kind === "range" && (
            <div className="seq-position">
              <div>{t("ty.subtype")}: <b>{d.subtype}</b></div>
              <div className="muted">{t("ty.rangeNote")}</div>
            </div>
          )}
        </div>
      );
    }
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || isNew ? createTypeStatements(schema, d) : alterStatements;
    return <SqlEditor value={d.name.trim() && statements.length ? statements.join("\n\n") : t("des.noChanges")} readOnly />;
  };

  return (
    <div className="doc">
      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      <div className="object-toolbar" role="toolbar">
        {tool("save", "des.save", dirty && !loadError && !readOnlyKind ? save : undefined)}
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
      {dialog?.kind === "invalid" && <MessageDialog title={t("ty.title")} message={t(dialog.message)} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />}
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
