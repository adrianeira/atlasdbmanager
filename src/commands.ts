import type { MessageKey } from "./i18n";
import type { SmallIconName } from "./icons";

/**
 * Registro único de comandos (UX-009): menus, toolbar e atalhos usam o mesmo id.
 * Comando sem `run` aparece desabilitado com o motivo "Ainda não disponível".
 */
export interface Command {
  id: string;
  label: MessageKey;
  shortcut?: string;
  run?: () => void;
  checked?: boolean;
  icon?: SmallIconName;
}

export type CommandMap = Record<string, Command>;

export type MenuEntry =
  | { kind: "cmd"; id: string }
  | { kind: "sep" }
  | { kind: "sub"; label: MessageKey; items: MenuEntry[] }
  | { kind: "item"; text: string; run?: () => void; icon?: SmallIconName; iconColor?: string; checked?: boolean };

export interface MenuDef {
  label: MessageKey;
  items: MenuEntry[];
}

/** Atalhos de construção: "-" vira separador, string vira comando. */
export function entries(...items: (string | MenuEntry)[]): MenuEntry[] {
  return items.map((i) => (typeof i !== "string" ? i : i === "-" ? { kind: "sep" } : { kind: "cmd", id: i }));
}

export function sub(label: MessageKey, ...items: (string | MenuEntry)[]): MenuEntry {
  return { kind: "sub", label, items: entries(...items) };
}

/** Converte "Ctrl+Shift+P" para comparação com um KeyboardEvent. */
export function matchesShortcut(e: KeyboardEvent, shortcut: string): boolean {
  const parts = shortcut.toLowerCase().split("+");
  const key = parts[parts.length - 1];
  const want = { ctrl: parts.includes("ctrl"), shift: parts.includes("shift"), alt: parts.includes("alt") };
  if (want.ctrl !== (e.ctrlKey || e.metaKey) || want.shift !== e.shiftKey || want.alt !== e.altKey) return false;
  return e.key.toLowerCase() === key;
}
