import type { ReactNode } from "react";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import type { ViewMode } from "./ObjectsView";

export interface StatusCell {
  text: ReactNode;
  width?: number;
  grow?: boolean;
}

interface Props {
  /** Largura da primeira célula: acompanha o painel de navegação, como na referência. */
  leftWidth: number;
  left: ReactNode;
  cells: StatusCell[];
  viewMode?: ViewMode;
  navVisible: boolean;
  infoVisible: boolean;
  onViewMode?: (m: ViewMode) => void;
  onToggleNav: () => void;
  onToggleInfo: () => void;
}

export function StatusBar(p: Props) {
  const t = useT();
  const mode = (m: ViewMode, icon: "viewList" | "viewDetail" | "viewIcons", label: "status.viewList" | "status.viewDetail" | "status.viewIcons") => (
    <button type="button" className={`sb-button${p.viewMode === m ? " is-active" : ""}`} title={t(label)} aria-pressed={p.viewMode === m} onClick={() => p.onViewMode?.(m)}>
      <SmallIcon name={icon} />
    </button>
  );
  return (
    <footer className="status-bar">
      <div className="sb-cell sb-left" style={{ width: p.leftWidth }}>{p.left}</div>
      {p.cells.map((c, i) => (
        <div key={i} className={`sb-cell sb-doc${c.grow ? " is-grow" : ""}`} style={c.width ? { width: c.width } : undefined} title={typeof c.text === "string" ? c.text : undefined}>
          {c.text}
        </div>
      ))}
      {!p.cells.some((c) => c.grow) && <div className="sb-cell is-grow" />}
      <div className="sb-right">
        {p.viewMode && (
          <>
            {mode("list", "viewList", "status.viewList")}
            {mode("detail", "viewDetail", "status.viewDetail")}
            {mode("icons", "viewIcons", "status.viewIcons")}
            <span className="sb-gap" />
          </>
        )}
        <button type="button" className={`sb-button${p.navVisible ? " is-active" : ""}`} title={t("status.toggleNav")} aria-pressed={p.navVisible} onClick={p.onToggleNav}>
          <SmallIcon name="paneNav" />
        </button>
        <button type="button" className={`sb-button${p.infoVisible ? " is-active" : ""}`} title={t("status.toggleInfo")} aria-pressed={p.infoVisible} onClick={p.onToggleInfo}>
          <SmallIcon name="paneInfo" />
        </button>
      </div>
    </footer>
  );
}
