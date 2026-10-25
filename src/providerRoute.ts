/**
 * Escolha do comando que atende uma chamada. As telas continuam chamando `pg_*`; quando a conexão é de outro banco (o id da conexão
 * começa pelo nome do banco, como `sqlite_...`), a chamada vai para o comando equivalente desse banco. Para o PostgreSQL nada muda:
 * o comando e os argumentos saem exatamente como entraram.
 */

export const SQLITE_PREFIX = "sqlite_";

/** Os `pg_*` que o SQLite atende (o resto da lista de comandos do PostgreSQL não existe no SQLite). */
const SQLITE_COMMANDS = new Set([
  "test_connection", "open", "close", "close_database", "list_databases", "list_schemas", "list_objects", "table_detail", "list_roles",
  "execute", "cancel", "table_rows", "table_count", "apply", "fk_labels", "data_profile", "table_profile", "find", "find_data", "export_cancel", "schema_diagram",
  "tx_begin", "tx_apply", "tx_execute", "tx_table_rows", "tx_table_count", "tx_end",
]);

export interface Routed {
  cmd: string;
  args?: Record<string, unknown>;
}

export function routeCommand(cmd: string, args?: Record<string, unknown>): Routed {
  if (!cmd.startsWith("pg_")) return { cmd, args };
  const id = args?.sessionId ?? args?.txId ?? args?.exportId;
  const params = args?.params as { provider?: string } | undefined;
  const sqlite = (typeof id === "string" && id.startsWith(SQLITE_PREFIX)) || (cmd === "pg_test_connection" && params?.provider === "sqlite");
  if (!sqlite) return { cmd, args };
  const rest = cmd.slice(3);
  return SQLITE_COMMANDS.has(rest) ? { cmd: SQLITE_PREFIX + rest, args } : { cmd: "sqlite_unsupported", args: { what: cmd } };
}

/** O id de uma conexão nova: o PostgreSQL mantém o formato de sempre; os outros bancos levam o nome do banco na frente. */
export function newConnectionId(provider: string, time: string): string {
  return provider === "sqlite" ? `${SQLITE_PREFIX}${time}` : `c${time}`;
}

/** A conexão (ou transação) é de SQLite? */
export const isSqliteId = (id: string | undefined | null): boolean => typeof id === "string" && id.startsWith(SQLITE_PREFIX);
