import { useEffect, useMemo, useState, type ReactNode } from "react";
import { errorText, toAtlasError, type SequenceDetail } from "../backend";
import { MessageDialog } from "../components/MessageDialog";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import {
  alterSeqStatements, createSeqStatements, emptySeq, fromDetail, isInteger, nextValue, restartSeqSql, SEQ_TYPES, setValueSql, validateSeq,
  type SeqState, type SeqType,
} from "../sql/sequence";

/** Acesso ao banco real; ausente = modo de demonstração. */
export interface SequenceBackend {
  loadDetail: (name: string) => Promise<SequenceDetail>;
  /** Executa o DDL (ou a chamada de `setval`) numa transação. */
  apply: (statements: string[]) => Promise<void>;
}

interface Props {
  schema: string;
  /** Ausente = sequência nova. */
  seqName?: string;
  owners: string[];
  serverVersionNum: number;
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: (name: string) => void;
  backend?: SequenceBackend;
}

type SubTab = "definition" | "comment" | "sql";
const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "definition", label: "fn.definition" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

type Dialog =
  | null
  | { kind: "invalid"; message: MessageKey }
  | { kind: "review"; statements: string[]; reload?: boolean; title?: MessageKey }
  | { kind: "value"; mode: "restart" | "set" };

export function SequenceDesignerDoc({ schema, seqName, owners, serverVersionNum, active, production, onStatus, onDirty, onSaved, backend }: Props) {
  const t = useT();
  const [name, setName] = useState(seqName);
  const isNew = !name;
  const [original, setOriginal] = useState<SeqState | null>(null);
  const [detail, setDetail] = useState<SequenceDetail | null>(null);
  const [d, setD] = useState<SeqState>(emptySeq);
  const [loading, setLoading] = useState(!!backend && !!seqName);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<SubTab>("definition");
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(seqName ? "save" : "saveAs");
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
    if (!backend || !seqName) return;
    let alive = true;
    backend
      .loadDetail(seqName)
      .then((det) => {
        if (!alive) return;
        const st = fromDetail(det);
        setDetail(det);
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

  const patch = (p: Partial<SeqState>) => setD((x) => ({ ...x, ...p }));
  const alterStatements = useMemo(
    () => (name && original && validateSeq(d) === null ? alterSeqStatements(schema, original, d, serverVersionNum) : []),
    [name, original, d, schema, serverVersionNum],
  );
  const dirty = isNew ? d.name.trim() !== "" : alterStatements.length > 0;
  useEffect(() => onDirty(dirty), [dirty, onDirty]);

  useEffect(() => {
    if (!active) return;
    onStatus([{ text: dirty ? t("des.unsaved") : "", grow: true }]);
  }, [active, dirty, t, onStatus]);

  const save = () => {
    const invalid = validateSeq(d);
    if (invalid) return setDialog({ kind: "invalid", message: invalid === "name" ? "seq.needName" : invalid === "number" ? "seq.needNumber" : "seq.badOwnedBy" });
    setDialog({ kind: "review", statements: isNew ? createSeqStatements(schema, d, serverVersionNum) : alterStatements });
  };

  const commit = async (statements: string[], reload: boolean) => {
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
      if (reload && name) await load(name);
      else if (!reload) await load(finalName);
    } catch {
      // A gravação deu certo; só a releitura falhou (a aba continua utilizável).
    }
    setSaving(false);
    setDialog(null);
    if (!reload) {
      setName(finalName);
      setPreviewMode("save");
      onSaved(finalName);
    }
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
  const num = (key: "increment" | "min" | "max" | "start" | "cache", placeholder?: string) => (
    <input
      className={`field prop-field is-num${d[key].trim() !== "" && !isInteger(d[key]) ? " is-invalid" : ""}`}
      value={d[key]}
      placeholder={placeholder}
      inputMode="numeric"
      spellCheck={false}
      onChange={(e) => patch({ [key]: e.target.value } as Partial<SeqState>)}
    />
  );

  if (loading) return <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>;

  const typeNote = serverVersionNum !== 0 && serverVersionNum < 100000 ? t("vw.needsVersion", { v: "10" }) : undefined;
  const body = () => {
    if (tab === "definition") {
      return (
        <div className="options-form">
          {prop("fn.name", <input className="field prop-field" value={d.name} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />)}
          {prop("seq.dataType", (
            <select className="field prop-field" value={d.dataType} disabled={!!typeNote} onChange={(e) => patch({ dataType: e.target.value as SeqType })}>
              {SEQ_TYPES.map((x) => <option key={x}>{x}</option>)}
            </select>
          ), typeNote)}
          {prop("seq.increment", num("increment"))}
          {prop("seq.min", num("min", t("seq.default")))}
          {prop("seq.max", num("max", t("seq.default")))}
          {prop("seq.start", num("start", t("seq.default")))}
          {prop("seq.cache", num("cache"))}
          <label className="check-row">
            <input type="checkbox" checked={d.cycle} onChange={(e) => patch({ cycle: e.target.checked })} />
            <span>{t("seq.cycle")}</span>
          </label>
          {prop("seq.ownedBy", <input className="field prop-field" value={d.ownedBy} placeholder={t("seq.ownedByPlaceholder")} spellCheck={false} onChange={(e) => patch({ ownedBy: e.target.value })} />)}
          {prop("des.owner", (
            <select className="field prop-field" value={d.owner} onChange={(e) => patch({ owner: e.target.value })}>
              <option value="" />
              {[...new Set([...(d.owner ? [d.owner] : []), ...owners])].map((o) => <option key={o}>{o}</option>)}
            </select>
          ))}
          {detail && !isNew && (
            <div className="seq-position">
              <div>{t("seq.current")}: <b>{detail.isCalled ? detail.lastValue : t("seq.notUsed")}</b></div>
              <div>{t("seq.next")}: <b>{nextValue(detail, original?.increment ?? detail.increment)}</b></div>
              {detail.cache !== "1" && <div className="muted">{t("seq.cacheNote")}</div>}
            </div>
          )}
        </div>
      );
    }
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    const statements = previewMode === "saveAs" || isNew ? createSeqStatements(schema, d, serverVersionNum) : alterStatements;
    return <SqlEditor value={d.name.trim() && statements.length ? statements.join("\n\n") : t("des.noChanges")} readOnly />;
  };

  const canAct = !!backend && !isNew && !dirty;
  return (
    <div className="doc">
      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      <div className="object-toolbar" role="toolbar">
        {tool("save", "des.save", dirty && !loadError ? save : undefined)}
        <span className="ot-sep" />
        <button type="button" className="ot-button" disabled={!canAct} title={canAct ? t("seq.restart") : t("seq.saveFirst")} onClick={() => setDialog({ kind: "value", mode: "restart" })}>
          <SmallIcon name="refresh" /><span>{t("seq.restart")}</span>
        </button>
        <button type="button" className="ot-button" disabled={!canAct} title={canAct ? t("seq.setValue") : t("seq.saveFirst")} onClick={() => setDialog({ kind: "value", mode: "set" })}>
          <SmallIcon name="apply" /><span>{t("seq.setValue")}</span>
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

      {dialog?.kind === "invalid" && <MessageDialog title={t("seq.title")} message={t(dialog.message)} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />}
      {dialog?.kind === "value" && (
        <MessageDialog
          title={t(dialog.mode === "restart" ? "seq.restart" : "seq.setValue")}
          message={t(dialog.mode === "restart" ? "seq.restartHint" : "seq.setValueHint")}
          prompt={{ label: t(dialog.mode === "restart" ? "seq.restartValue" : "seq.newValue"), initial: dialog.mode === "restart" ? original?.start ?? "" : detail?.lastValue ?? "" }}
          onOk={(v) => {
            const value = (v ?? "").trim();
            if (dialog.mode === "set" && !isInteger(value)) return setDialog({ kind: "invalid", message: "seq.needNumber" });
            if (value !== "" && !isInteger(value)) return setDialog({ kind: "invalid", message: "seq.needNumber" });
            const sql = dialog.mode === "restart" ? restartSeqSql(schema, name ?? "", value) : setValueSql(schema, name ?? "", value);
            setDialog({ kind: "review", statements: [sql], reload: true });
          }}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "review" && (
        <MessageDialog
          title={t("des.reviewTitle")}
          message={t("des.reviewMessage")}
          sql={dialog.statements.join("\n\n")}
          confirmText={production && !isNew ? name : undefined}
          okLabel={t(dialog.reload ? "seq.run" : "des.save")}
          busy={saving}
          error={saveError ?? undefined}
          onOk={() => void commit(dialog.statements, !!dialog.reload)}
          onCancel={() => { setSaveError(null); setDialog(null); }}
        />
      )}
    </div>
  );
}
