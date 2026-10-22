import { useEffect, useRef, useState, type ReactNode } from "react";
import type { MessageKey } from "../i18n";
import { useT } from "../i18n";
import { LargeIcon, SmallIcon, type LargeIconName } from "../icons";
import { OTHERS_CATEGORIES, type Category, type ProviderId } from "../model";

interface Props {
  /** Categoria realçada: a da lista de objetos ou a do documento ativo (ex.: Query). */
  highlight: Category;
  provider?: ProviderId;
  showCaptions: boolean;
  onCategory: (c: Category) => void;
  onNewConnection: () => void;
  onNewQuery: () => void;
}

type ButtonDef = { id: Category | "others"; icon: LargeIconName; label: MessageKey };

function buttonsFor(provider?: ProviderId): ButtonDef[] {
  const pg = provider === "postgresql";
  return [
    { id: "table", icon: "table", label: "tb.table" },
    { id: "view", icon: "view", label: "tb.view" },
    ...(pg ? [{ id: "matview", icon: "matview", label: "tb.matview" } as ButtonDef] : []),
    { id: "function", icon: "function", label: "tb.function" },
    { id: "user", icon: "user", label: pg ? "tb.role" : "tb.user" },
    { id: "others", icon: "others", label: "tb.others" },
    { id: "query", icon: "query", label: "tb.query" },
    { id: "backup", icon: "backup", label: "tb.backup" },
    { id: "automation", icon: "automation", label: "tb.automation" },
    { id: "model", icon: "model", label: "tb.model" },
    { id: "bi", icon: "bi", label: "tb.bi" },
  ];
}

const OTHERS_LABEL: Record<string, MessageKey> = { matview: "cat.matview", sequence: "cat.sequence", type: "cat.type" };

export function MainToolbar({ highlight, provider, showCaptions, onCategory, onNewConnection, onNewQuery }: Props) {
  const t = useT();
  const [othersOpen, setOthersOpen] = useState(false);
  const othersRef = useRef<HTMLDivElement>(null);
  const others: Category[] = provider === "postgresql" ? OTHERS_CATEGORIES : ["matview", ...OTHERS_CATEGORIES];

  useEffect(() => {
    if (!othersOpen) return;
    const close = (e: MouseEvent) => {
      if (!othersRef.current?.contains(e.target as Node)) setOthersOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [othersOpen]);

  const button = (key: string, icon: LargeIconName, label: MessageKey, onClick: (() => void) | undefined, active = false, extra?: ReactNode) => (
    <button
      key={key}
      type="button"
      className={`tb-button${active ? " is-active" : ""}${showCaptions ? "" : " no-caption"}`}
      onClick={onClick}
      disabled={!onClick}
      title={onClick ? t(label) : t("notImplemented")}
      aria-pressed={active}
    >
      <LargeIcon name={icon} />
      {showCaptions && <span className="tb-caption">{t(label)}</span>}
      {extra}
    </button>
  );

  return (
    <div className="main-toolbar" role="toolbar" aria-label="Main toolbar">
      {button("connection", "connection", "tb.connection", onNewConnection)}
      {button("newQuery", "newQuery", "tb.newQuery", onNewQuery)}
      <div className="tb-separator" />
      {buttonsFor(provider).map((b) => {
        if (b.id === "others") {
          const active = others.includes(highlight);
          return (
            <div key="others" className="tb-others" ref={othersRef}>
              {button("others", "others", "tb.others", () => setOthersOpen((o) => !o), active, (
                <span className="tb-dropdown"><SmallIcon name="chevronDown" size={10} /></span>
              ))}
              {othersOpen && (
                <div className="menu-popup tb-others-popup" role="menu">
                  {others.map((c) => (
                    <button key={c} type="button" role="menuitem" className="menu-entry" onClick={() => { setOthersOpen(false); onCategory(c); }}>
                      <span className="menu-check"><SmallIcon name={c} /></span>
                      <span className="menu-label">{t(OTHERS_LABEL[c])}</span>
                      <span className="menu-shortcut" />
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        }
        const cat = b.id;
        return button(cat, b.icon, b.label, () => onCategory(cat), highlight === cat);
      })}
      <div className="tb-spacer" />
      <button type="button" className="tb-account" title={t("tb.account")} disabled>
        <LargeIcon name="account" />
      </button>
    </div>
  );
}
