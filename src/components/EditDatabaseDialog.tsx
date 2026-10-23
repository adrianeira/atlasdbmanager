import { useEffect, useMemo, useState } from "react";
import type { DatabaseDetail } from "../backend";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import { databaseState, databaseStatements, validateDatabase, type DatabaseEditState } from "../sql/dbedit";
import { Dropdown } from "./Dropdown";

interface Props {
  detail: DatabaseDetail;
  /** Papéis do servidor para o campo Dono. */
  owners: string[];
  /** `false` no banco da própria conexão: o PostgreSQL não renomeia o banco em que a sessão está. */
  canRename: boolean;
  /** `ALLOW_CONNECTIONS` e `IS_TEMPLATE` só existem no `ALTER DATABASE` a partir do PostgreSQL 9.5. */
  supportsFlags: boolean;
  onApply: (state: DatabaseEditState) => void;
  onClose: () => void;
}

/** "Editar banco de dados": nome, dono, limite de conexões, opções e comentário; o resto só é exibido. */
export function EditDatabaseDialog({ detail, owners, canRename, supportsFlags, onApply, onClose }: Props) {
  const t = useT();
  const [state, setState] = useState<DatabaseEditState>(() => databaseState(detail));
  const patch = (p: Partial<DatabaseEditState>) => setState((s) => ({ ...s, ...p }));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const ownerItems = useMemo(
    () => [...new Set([detail.owner, ...owners])].map((o) => ({ value: o, label: o })),
    [detail.owner, owners],
  );
  const problem = validateDatabase(state);
  const changed = databaseStatements(detail, state).length > 0;

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal ed-dialog" role="dialog" aria-label={t("cmd.editDatabase")}>
        <div className="modal-header">
          <span className="modal-title">{t("cmd.editDatabase").replace(/\.\.\.$/, "")} — {detail.name}</span>
          <button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button>
        </div>
        <div className="modal-body ed-body">
          <div className={`ed-row${problem === "name" ? " has-error" : ""}`}>
            <label>{t("ed.dbName")}</label>
            <input
              className="field"
              value={state.name}
              disabled={!canRename}
              title={canRename ? undefined : t("ed.renameBlocked")}
              spellCheck={false}
              onChange={(e) => patch({ name: e.target.value })}
            />
          </div>
          {!canRename && <div className="muted ed-note">{t("ed.renameBlocked")}</div>}
          <div className="ed-row">
            <label>{t("ed.owner")}</label>
            <Dropdown className="ed-dd" ariaLabel={t("ed.owner")} value={state.owner} items={ownerItems} onChange={(v) => patch({ owner: v })} />
          </div>
          <div className={`ed-row${problem === "limit" ? " has-error" : ""}`}>
            <label>{t("ed.connLimit")}</label>
            <input
              className="field ed-num"
              inputMode="numeric"
              value={state.connLimit}
              title={t("ed.connLimitHint")}
              onChange={(e) => patch({ connLimit: e.target.value.replace(/[^\d-]/g, "") })}
            />
          </div>
          {supportsFlags && (
            <div className="ed-row">
              <span />
              <div className="ed-checks">
                <label className="check-row"><input type="checkbox" checked={state.allowConnections} onChange={(e) => patch({ allowConnections: e.target.checked })} /><span>{t("ed.allowConnections")}</span></label>
                <label className="check-row"><input type="checkbox" checked={state.isTemplate} onChange={(e) => patch({ isTemplate: e.target.checked })} /><span>{t("ed.isTemplate")}</span></label>
              </div>
            </div>
          )}
          <div className="ed-row">
            <label>{t("ed.comment")}</label>
            <textarea className="field ed-comment" value={state.comment} onChange={(e) => patch({ comment: e.target.value })} />
          </div>

          <div className="ed-sep" />
          <div className="ed-row"><label>{t("ed.encoding")}</label><input className="field" value={detail.encoding} disabled readOnly /></div>
          <div className="ed-row"><label>{t("ed.collate")}</label><input className="field" value={detail.collate} disabled readOnly /></div>
          <div className="ed-row"><label>{t("ed.ctype")}</label><input className="field" value={detail.ctype} disabled readOnly /></div>
          <div className="ed-row"><label>{t("ed.tablespace")}</label><input className="field" value={detail.tablespace} disabled readOnly /></div>
          <div className="muted ed-note">{t("ed.notChangeable")}</div>
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
