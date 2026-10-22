import { useEffect, useRef, useState } from "react";
import type { CommandMap, MenuDef } from "../commands";
import { useT } from "../i18n";
import { PopupMenu } from "./PopupMenu";

export function MenuBar({ menus, commands }: { menus: MenuDef[]; commands: CommandMap }) {
  const t = useT();
  const [open, setOpen] = useState<number | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open === null) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(null);
      if (e.key === "ArrowRight") setOpen((o) => (o === null ? o : (o + 1) % menus.length));
      if (e.key === "ArrowLeft") setOpen((o) => (o === null ? o : (o - 1 + menus.length) % menus.length));
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, menus.length]);

  return (
    <div className="menubar" role="menubar" ref={ref}>
      {menus.map((menu, i) => (
        <div key={menu.label} className="menubar-item-wrap">
          <button
            type="button"
            role="menuitem"
            aria-haspopup="menu"
            aria-expanded={open === i}
            className={`menubar-item${open === i ? " is-open" : ""}`}
            onClick={() => setOpen(open === i ? null : i)}
            onMouseEnter={() => open !== null && setOpen(i)}
          >
            {t(menu.label)}
          </button>
          {open === i && <PopupMenu items={menu.items} commands={commands} onClose={() => setOpen(null)} />}
        </div>
      ))}
    </div>
  );
}
