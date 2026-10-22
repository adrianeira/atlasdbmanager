import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { SmallIcon } from "../icons";

export interface DropdownItem {
  value: string;
  label: string;
}

interface Props {
  value: string;
  items: DropdownItem[];
  onChange: (value: string) => void;
  disabled?: boolean;
  className?: string;
  ariaLabel?: string;
  /** Ícone opcional à esquerda do valor selecionado (ex.: ícone do provedor). */
  leftIcon?: ReactNode;
  /** Placeholder quando o valor atual não bate com nenhum item. */
  placeholder?: string;
  /** Chamado quando a lista abre (ex.: para reler os itens do servidor). */
  onOpen?: () => void;
}

/**
 * Dropdown próprio: substitui o <select> nativo porque o popup do Chromium ignora CSS
 * no hover do item (sempre fica cinza escuro). Com DOM próprio seguimos os tokens do tema.
 */
export function Dropdown({ value, items, onChange, disabled, className, ariaLabel, leftIcon, placeholder, onOpen }: Props) {
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const btnRef = useRef<HTMLButtonElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);

  const selected = items.find((i) => i.value === value) ?? null;
  const label = selected?.label ?? placeholder ?? "";

  useEffect(() => {
    if (!open) return;
    const idx = items.findIndex((i) => i.value === value);
    setHighlight(idx >= 0 ? idx : 0);
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node;
      if (btnRef.current?.contains(t) || popupRef.current?.contains(t)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", onDoc, true);
    return () => document.removeEventListener("mousedown", onDoc, true);
  }, [open, items, value]);

  const pick = (v: string) => { onChange(v); setOpen(false); btnRef.current?.focus(); };

  const onKey = (e: React.KeyboardEvent) => {
    if (disabled) return;
    if (!open) {
      if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") { e.preventDefault(); setOpen(true); onOpen?.(); }
      return;
    }
    if (e.key === "Escape") { e.preventDefault(); setOpen(false); btnRef.current?.focus(); }
    else if (e.key === "ArrowDown") { e.preventDefault(); setHighlight((h) => Math.min(items.length - 1, h + 1)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setHighlight((h) => Math.max(0, h - 1)); }
    else if (e.key === "Home") { e.preventDefault(); setHighlight(0); }
    else if (e.key === "End") { e.preventDefault(); setHighlight(items.length - 1); }
    else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); if (items[highlight]) pick(items[highlight].value); }
  };

  const rect = btnRef.current?.getBoundingClientRect();

  return (
    <>
      <button
        type="button"
        ref={btnRef}
        className={`dd-button${className ? ` ${className}` : ""}${open ? " is-open" : ""}`}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => { if (disabled) return; if (!open) onOpen?.(); setOpen((o) => !o); }}
        onKeyDown={onKey}
      >
        {leftIcon && <span className="dd-left">{leftIcon}</span>}
        <span className="dd-label">{label}</span>
        <span className="dd-chevron"><SmallIcon name="chevronDown" size={10} /></span>
      </button>
      {open && rect && createPortal(
        <div
          ref={popupRef}
          className="dd-popup"
          role="listbox"
          style={{
            position: "fixed",
            left: rect.left,
            top: Math.min(rect.bottom + 1, window.innerHeight - 300),
            minWidth: rect.width,
          }}
        >
          {items.map((it, i) => (
            <button
              key={it.value || `__${i}`}
              type="button"
              role="option"
              aria-selected={it.value === value}
              className={`dd-item${i === highlight ? " is-active" : ""}${it.value === value ? " is-selected" : ""}`}
              onMouseEnter={() => setHighlight(i)}
              onMouseDown={(e) => { e.preventDefault(); pick(it.value); }}
            >
              {it.label}
            </button>
          ))}
        </div>,
        document.body,
      )}
    </>
  );
}
