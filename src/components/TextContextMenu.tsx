import { useEffect, useState } from "react";
import { readClipboard, copyToClipboard } from "../clipboard";
import type { MenuEntry } from "../commands";
import { useT } from "../i18n";
import { PopupMenu } from "./PopupMenu";

type Field = HTMLInputElement | HTMLTextAreaElement | HTMLElement;

const isTextControl = (el: Element): el is HTMLInputElement | HTMLTextAreaElement =>
  el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && !["checkbox", "radio", "button", "submit", "range", "color", "file"].includes(el.type));

/** O texto selecionado no campo (campos de texto têm a seleção própria, o resto usa a seleção da página). */
function selectedText(el: Field): string {
  if (isTextControl(el)) return (el.selectionStart ?? 0) === (el.selectionEnd ?? 0) ? "" : el.value.slice(el.selectionStart ?? 0, el.selectionEnd ?? 0);
  return window.getSelection()?.toString() ?? "";
}

/**
 * Menu do botão direito nos campos de texto e no editor: Recortar, Copiar, Colar e Selecionar tudo, no estilo do Atlas.
 * Fora dos campos o menu do navegador (Voltar, Imprimir, Inspecionar...) não aparece; as telas com menu próprio o abrem por conta delas.
 */
export function TextContextMenu() {
  const t = useT();
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);

  useEffect(() => {
    const onContext = (e: MouseEvent) => {
      if (e.defaultPrevented) return; // uma tela com menu próprio já tratou
      const target = e.target as HTMLElement | null;
      const field = target?.closest?.("input, textarea, [contenteditable='true'], .cm-editor") as Field | null;
      e.preventDefault();
      if (!field) return;
      // O editor (CodeMirror) é um bloco editável dentro de `.cm-editor`.
      const el = (field.classList?.contains("cm-editor") ? field.querySelector<HTMLElement>(".cm-content") : null) ?? field;
      const editable = isTextControl(el) ? !el.readOnly && !el.disabled : el.isContentEditable;
      const text = selectedText(el);
      const back = () => el.focus();
      const items: MenuEntry[] = [
        { kind: "item", text: t("cmd.cut"), run: editable && text ? () => { back(); void copyToClipboard(text).then(() => { back(); document.execCommand("delete"); }); } : undefined },
        { kind: "item", text: t("cmd.copy"), run: text ? () => void copyToClipboard(text) : undefined },
        { kind: "item", text: t("cmd.paste"), run: editable ? () => { void readClipboard().then((clip) => { back(); if (clip) document.execCommand("insertText", false, clip); }).catch(() => undefined); } : undefined },
        { kind: "sep" },
        { kind: "item", text: t("cmd.selectAll"), run: () => { back(); if (isTextControl(el)) el.select(); else document.execCommand("selectAll"); } },
      ];
      setMenu({ x: e.clientX, y: e.clientY, items });
    };
    window.addEventListener("contextmenu", onContext);
    return () => window.removeEventListener("contextmenu", onContext);
  }, [t]);

  useEffect(() => {
    if (!menu) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof MouseEvent && (e.target as HTMLElement | null)?.closest?.(".text-ctx-menu")) return;
      setMenu(null);
    };
    window.addEventListener("mousedown", close, true);
    window.addEventListener("keydown", close, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("keydown", close, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [menu]);

  if (!menu) return null;
  return (
    <PopupMenu
      items={menu.items}
      commands={{}}
      onClose={() => setMenu(null)}
      className="ctx-menu text-ctx-menu"
      style={{ position: "fixed", left: Math.min(menu.x, window.innerWidth - 200), top: Math.min(menu.y, window.innerHeight - 140), zIndex: 2000 }}
    />
  );
}
