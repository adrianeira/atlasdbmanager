import { useT } from "../i18n";
import { SmallIcon } from "../icons";

export interface ColumnsDialogItem {
  name: string;
  type: string;
}

interface Props {
  columns: ColumnsDialogItem[];
  hidden: Set<string>;
  onToggle: (name: string) => void;
  onShowAll: () => void;
  onHideAll: () => void;
  /** Restaura o layout padrão (ordem, largura, visibilidade e ordenação). */
  onReset?: () => void;
  onClose: () => void;
}

/** Botão "Columns" da toolbar: mostra/oculta colunas do grid (a ordem e os dados não mudam). */
export function ColumnsDialog({ columns, hidden, onToggle, onShowAll, onHideAll, onReset, onClose }: Props) {
  const t = useT();
  const visibleCount = columns.length - hidden.size;

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal cols-dialog" role="dialog" aria-label={t("data.columns")}>
        <div className="modal-header"><span className="modal-title">{t("data.columns")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="modal-body">
          <div className="cols-list">
            {columns.map((c) => (
              <label key={c.name} className="check-row cols-row">
                <input type="checkbox" checked={!hidden.has(c.name)} onChange={() => onToggle(c.name)} />
                <span className="cols-name">{c.name}</span>
                <span className="muted cols-type">{c.type}</span>
              </label>
            ))}
          </div>
          <div className="cols-count muted">{t("cols.visible", { n: visibleCount, total: columns.length })}</div>
        </div>
        <div className="modal-footer">
          <button type="button" className="btn" onClick={onShowAll}>{t("cols.showAll")}</button>
          <button type="button" className="btn" onClick={onHideAll}>{t("cols.hideAll")}</button>
          {onReset && <button type="button" className="btn" title={t("cols.restoreHint")} onClick={onReset}>{t("cols.restore")}</button>}
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={onClose}>{t("common.close")}</button>
        </div>
      </div>
    </div>
  );
}
