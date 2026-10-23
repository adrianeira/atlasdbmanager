import { useT } from "../../i18n";
import type { Advanced } from "./importModel";

interface Props {
  value: Advanced;
  onChange: (next: Advanced) => void;
  onClose: () => void;
}

/** Botão "Avançado" do modo de importação: como os INSERTs são montados e o que fazer com erros. */
export function ImportAdvancedDialog({ value, onChange, onClose }: Props) {
  const t = useT();
  const patch = (p: Partial<Advanced>) => onChange({ ...value, ...p });
  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal exp-options-dialog" role="dialog" aria-label={t("imp.adv.title")}>
        <div className="modal-header">{t("imp.adv.title")}</div>
        <div className="modal-body exp-options">
          <label className="check-row"><input type="checkbox" checked={value.extendedInsert} onChange={(e) => patch({ extendedInsert: e.target.checked })} /><span>{t("imp.adv.extended")}</span></label>
          <label className="check-row imp-indent">
            <input type="checkbox" disabled={!value.extendedInsert} checked={value.limitStatementSize} onChange={(e) => patch({ limitStatementSize: e.target.checked })} />
            <span>{t("imp.adv.limit")}</span>
            <input
              className="field imp-num"
              type="number"
              min={1}
              disabled={!value.extendedInsert || !value.limitStatementSize}
              value={value.maxStatementKb}
              onChange={(e) => patch({ maxStatementKb: Math.max(1, Math.floor(Number(e.target.value) || 1)) })}
            />
            <span>{t("imp.adv.kb")}</span>
          </label>
          <label className="check-row"><input type="checkbox" checked={value.emptyAsNull} onChange={(e) => patch({ emptyAsNull: e.target.checked })} /><span>{t("imp.adv.emptyNull")}</span></label>
          <label className="check-row"><input type="checkbox" checked={value.continueOnError} onChange={(e) => patch({ continueOnError: e.target.checked })} /><span>{t("imp.adv.continue")}</span></label>
          {!value.continueOnError && <div className="muted exp-note">{t("imp.adv.stopNote")}</div>}
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={onClose}>OK</button>
        </div>
      </div>
    </div>
  );
}
