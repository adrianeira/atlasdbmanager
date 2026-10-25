import type { MenuEntry } from "./commands";
import type { MessageKey } from "./i18n";

/**
 * Itens dos menus de contexto que um banco não tem. Os menus nascem com os itens do PostgreSQL; para uma conexão de outro banco
 * estas listas os tiram. Um item sai da lista de um banco quando passa a existir para ele.
 */
const NOT_IN_SQLITE = new Set<MessageKey>([
  "cmd.newDatabase", "cmd.editDatabase", "cmd.deleteDatabase", "cmd.newSchema", "cmd.editSchema", "cmd.deleteSchema",
  "cmd.extractSql", "cmd.reverseToModel", "cmd.share",
  "cmd.truncateTable", "cmd.renameView",
]);

const NOT_IN_MSSQL = new Set<MessageKey>([
  "cmd.newDatabase", "cmd.editDatabase", "cmd.deleteDatabase", "cmd.editSchema",
  "cmd.newBackup", "cmd.restoreBackup", "cmd.extractSql", "cmd.reverseToModel", "cmd.share",
  "sub.duplicateTable", "sub.dumpSql", "act.importWizard", "act.exportWizard", "cmd.console", "cmd.findInDatabase", "cmd.findInSchema",
  "cmd.renameTable", "cmd.renameView",
]);

const SETS: Record<string, Set<MessageKey>> = { sqlite: NOT_IN_SQLITE, sqlserver: NOT_IN_MSSQL };

/** Os bancos que têm lista de itens a esconder (o PostgreSQL não tem). */
export const hasMenuFilter = (provider: string | undefined): boolean => !!provider && provider in SETS;

/** Tira os itens que o banco não tem e os separadores que sobrarem soltos (no começo, no fim ou em sequência). */
export function menuForProvider(provider: string, items: MenuEntry[], labels: Map<string, MessageKey>): MenuEntry[] {
  const hidden = SETS[provider] ?? new Set<MessageKey>();
  const kept = items.filter((e) => {
    if (e.kind === "sub") return !hidden.has(e.label);
    if (e.kind === "item") return !hidden.has(labels.get(e.text) as MessageKey);
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

/** Os textos dos itens que algum banco esconde (o menu guarda o texto, não a chave). */
export const hiddenKeys = (): MessageKey[] => [...new Set([...NOT_IN_SQLITE, ...NOT_IN_MSSQL])].filter((k) => k.startsWith("cmd.") || k.startsWith("act."));
