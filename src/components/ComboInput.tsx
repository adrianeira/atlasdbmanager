import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { SmallIcon } from "../icons";

interface Props {
  value: string;
  options: string[];
  onChange: (value: string) => void;
  className?: string;
  spellCheck?: boolean;
}

/**
 * Campo de texto com lista de sugestões. Substitui o `<input list>` nativo, cuja seta (e o popup do
 * Chromium) não seguem o tema nem o alinhamento dos demais controles.
 */
export function ComboInput({ value, options, onChange, className, spellCheck }: Props) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLSpanElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node;
      if (wrapRef.current?.contains(t) || popupRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", onDoc, true);
    return () => document.removeEventListener("mousedown", onDoc, true);
  }, [open]);

  const rect = wrapRef.current?.getBoundingClientRect();

  return (
    <span ref={wrapRef} className={`combo${className ? ` ${className}` : ""}`}>
      <input
        className="field combo-input"
        value={value}
        spellCheck={spellCheck}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") { e.preventDefault(); setOpen(true); }
          else if (e.key === "Escape") setOpen(false);
        }}
      />
      <button type="button" className="combo-arrow" tabIndex={-1} aria-label="▾" onMouseDown={(e) => { e.preventDefault(); setOpen((o) => !o); }}>
        <SmallIcon name="chevronDown" size={10} />
      </button>
      {open && rect && createPortal(
        <div
          ref={popupRef}
          className="dd-popup"
          role="listbox"
          style={{ position: "fixed", left: rect.left, top: Math.min(rect.bottom + 1, window.innerHeight - 260), minWidth: rect.width }}
        >
          {options.map((o) => (
            <button
              key={o}
              type="button"
              role="option"
              aria-selected={o === value}
              className={`dd-item${o === value ? " is-selected" : ""}`}
              onMouseDown={(e) => { e.preventDefault(); onChange(o); setOpen(false); }}
            >
              {o}
            </button>
          ))}
        </div>,
        document.body,
      )}
    </span>
  );
}
