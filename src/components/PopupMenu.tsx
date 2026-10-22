import { useState, type CSSProperties } from "react";
import type { CommandMap, MenuEntry } from "../commands";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";

interface Props {
  items: MenuEntry[];
  commands: CommandMap;
  onClose: () => void;
  style?: CSSProperties;
  className?: string;
}

function isEnabled(e: MenuEntry, commands: CommandMap): boolean {
  switch (e.kind) {
    case "cmd": return !!commands[e.id]?.run;
    case "item": return !!e.run;
    case "sub": return e.items.some((i) => isEnabled(i, commands));
    default: return false;
  }
}

/** Menu suspenso com submenus aninhados (abrem ao passar o mouse, como no Windows). */
export function PopupMenu({ items, commands, onClose, style, className }: Props) {
  const t = useT();
  const [openSub, setOpenSub] = useState<number | null>(null);

  return (
    <div className={`menu-popup${className ? ` ${className}` : ""}`} role="menu" style={style}>
      {items.map((e, i) => {
        if (e.kind === "sep") return <div key={`sep${i}`} className="menu-sep" role="separator" />;
        const enabled = isEnabled(e, commands);

        if (e.kind === "sub") {
          return (
            <div key={`sub${i}`} className="menu-sub-wrap" onMouseEnter={() => setOpenSub(i)}>
              <button type="button" role="menuitem" aria-haspopup="menu" aria-expanded={openSub === i} className={`menu-entry${openSub === i ? " is-open" : ""}`} disabled={!enabled}>
                <span className="menu-check" />
                <span className="menu-label">{t(e.label)}</span>
                <span className="menu-shortcut menu-arrow"><SmallIcon name="chevronRight" size={12} /></span>
              </button>
              {openSub === i && enabled && <PopupMenu items={e.items} commands={commands} onClose={onClose} className="menu-popup-sub" />}
            </div>
          );
        }

        const cmd = e.kind === "cmd" ? commands[e.id] : undefined;
        if (e.kind === "cmd" && !cmd) return null;
        const label = cmd ? t(cmd.label) : e.kind === "item" ? e.text : "";
        const icon = cmd?.icon ?? (e.kind === "item" ? e.icon : undefined);
        const run = cmd ? cmd.run : e.kind === "item" ? e.run : undefined;
        return (
          <button
            key={cmd?.id ?? `item${i}`}
            type="button"
            role={cmd?.checked === undefined ? "menuitem" : "menuitemcheckbox"}
            aria-checked={cmd?.checked}
            className="menu-entry"
            disabled={!enabled}
            title={enabled ? undefined : t("notImplemented")}
            onMouseEnter={() => setOpenSub(null)}
            onClick={() => {
              onClose();
              run?.();
            }}
          >
            <span className="menu-check">{cmd?.checked ? <SmallIcon name="check" /> : icon ? <SmallIcon name={icon} /> : null}</span>
            <span className="menu-label">{label}</span>
            <span className="menu-shortcut">{cmd?.shortcut ?? ""}</span>
          </button>
        );
      })}
    </div>
  );
}
