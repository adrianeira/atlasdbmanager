import type { MenuEntry } from "./commands";
import type { MessageKey } from "./i18n";

/**
 * Itens dos menus de contexto que o SQLite não tem (schemas, tablespaces, TRUNCATE, `LIKE` na cópia de tabela...). Os menus nascem
 * com os itens do PostgreSQL; para uma conexão SQLite esta lista os tira. Um item sai desta lista quando passa a existir para o SQLite.
 */
const NOT_IN_SQLITE = new Set<MessageKey>([
  "cmd.newDatabase", "cmd.editDatabase", "cmd.deleteDatabase", "cmd.newSchema", "cmd.editSchema", "cmd.deleteSchema",
  "cmd.extractSql", "cmd.reverseToModel", "cmd.share",
  "cmd.truncateTable", "cmd.console",
]);

/** Tira os itens que o SQLite não tem e os separadores que sobrarem soltos (no começo, no fim ou em sequência). */
export function menuForSqlite(items: MenuEntry[], labels: Map<string, MessageKey>): MenuEntry[] {
  const kept = items.filter((e) => {
    if (e.kind === "sub") return !NOT_IN_SQLITE.has(e.label);
    if (e.kind === "item") return !NOT_IN_SQLITE.has(labels.get(e.text) as MessageKey);
    return true;
  });
  const out: MenuEntry[] = [];
  for (const e of kept) {
    if (e.kind === "sep" && (out.length === 0 || out[out.length - 1].kind === "sep")) continue;
    out.push(e);
  }
  while (out.length && out[out.length - 1].kind === "sep") out.pop();
  return out;
}

export const sqliteHiddenKeys = (): MessageKey[] => [...NOT_IN_SQLITE].filter((k) => k.startsWith("cmd.") || k.startsWith("act."));
