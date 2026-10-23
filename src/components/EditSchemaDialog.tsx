import { useEffect, useMemo, useState } from "react";
import type { SchemaDetail } from "../backend";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import { schemaState, schemaStatements, validateSchema, type SchemaEditState } from "../sql/dbedit";
import { Dropdown } from "./Dropdown";

interface Props {
  detail: SchemaDetail;
  /** Papéis do servidor para o campo Dono. */
  owners: string[];
  onApply: (state: SchemaEditState) => void;
  onClose: () => void;
}

/** "Editar schema": nome, dono e comentário. */
export function EditSchemaDialog({ detail, owners, onApply, onClose }: Props) {
  const t = useT();
  const [state, setState] = useState<SchemaEditState>(() => schemaState(detail));
  const patch = (p: Partial<SchemaEditState>) => setState((s) => ({ ...s, ...p }));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const ownerItems = useMemo(
    () => [...new Set([detail.owner, ...owners])].map((o) => ({ value: o, label: o })),
    [detail.owner, owners],
  );
  const problem = validateSchema(state);
  const changed = schemaStatements(detail, state).length > 0;

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal ed-dialog" role="dialog" aria-label={t("cmd.editSchema")}>
        <div className="modal-header">
          <span className="modal-title">{t("cmd.editSchema").replace(/\.\.\.$/, "")} — {detail.name}</span>
          <button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button>
        </div>
        <div className="modal-body ed-body">
          <div className={`ed-row${problem === "name" ? " has-error" : ""}`}>
            <label>{t("ed.schemaName")}</label>
            <input className="field" value={state.name} spellCheck={false} onChange={(e) => patch({ name: e.target.value })} />
          </div>
          <div className="ed-row">
            <label>{t("ed.owner")}</label>
            <Dropdown className="ed-dd" ariaLabel={t("ed.owner")} value={state.owner} items={ownerItems} onChange={(v) => patch({ owner: v })} />
          </div>
          <div className="ed-row">
            <label>{t("ed.comment")}</label>
            <textarea className="field ed-comment" value={state.comment} onChange={(e) => patch({ comment: e.target.value })} />
          </div>
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" disabled={!!problem || !changed} onClick={() => onApply(state)}>{t("common.apply")}</button>
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
        </div>
      </div>
    </div>
  );
}
