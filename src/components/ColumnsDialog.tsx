import { useT } from "../i18n";

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
  onClose: () => void;
}

/** Botão "Columns" da toolbar: mostra/oculta colunas do grid (a ordem e os dados não mudam). */
export function ColumnsDialog({ columns, hidden, onToggle, onShowAll, onHideAll, onClose }: Props) {
  const t = useT();
  const visibleCount = columns.length - hidden.size;

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal cols-dialog" role="dialog" aria-label={t("data.columns")}>
        <div className="modal-header">{t("data.columns")}</div>
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
          <button type="button" className="find-button" onClick={onShowAll}>{t("cols.showAll")}</button>
          <button type="button" className="find-button" onClick={onHideAll}>{t("cols.hideAll")}</button>
          <span className="spacer" />
          <button type="button" className="find-button" onClick={onClose}>{t("common.close")}</button>
        </div>
      </div>
    </div>
  );
}
