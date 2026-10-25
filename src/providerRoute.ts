/**
 * Escolha do comando que atende uma chamada. As telas continuam chamando `pg_*`; quando a conexão é de outro banco (o id da conexão
 * começa pelo nome do banco, como `sqlite_...` e `mssql_...`), a chamada vai para o comando equivalente desse banco. Para o
 * PostgreSQL nada muda: o comando e os argumentos saem exatamente como entraram.
 */

export const SQLITE_PREFIX = "sqlite_";
export const MSSQL_PREFIX = "mssql_";

interface Provider {
  /** Começo do id da conexão e do nome dos comandos (`sqlite_...`, `mssql_...`). */
  prefix: string;
  /** Valor de `provider` nos parâmetros de conexão. */
  param: string;
  /** Os `pg_*` que o banco atende; o resto vira "indisponível". */
  commands: Set<string>;
  /** O console tem comandos sem o prefixo `pg_`. */
  console: boolean;
}

const TX = ["tx_begin", "tx_apply", "tx_execute", "tx_table_rows", "tx_table_count", "tx_end"];

const SQLITE: Provider = {
  prefix: SQLITE_PREFIX,
  param: "sqlite",
  console: true,
  commands: new Set([
    "test_connection", "open", "close", "close_database", "list_databases", "list_schemas", "list_objects", "table_detail", "list_roles",
    "execute", "cancel", "table_rows", "table_count", "apply", "fk_labels", "data_profile", "table_profile", "find", "find_data", "export_cancel", "schema_diagram", "export", "import", "backup", "backup_list", "restore", "dump_sql", "view_detail",
    ...TX,
  ]),
};

const MSSQL: Provider = {
  prefix: MSSQL_PREFIX,
  param: "sqlserver",
  console: true,
  commands: new Set([
    "test_connection", "open", "close", "close_database", "list_databases", "list_schemas", "list_objects", "table_detail", "list_roles",
    "execute", "cancel", "table_rows", "table_count", "apply", "fk_labels", "schema_diagram", "find", "find_data", "export_cancel", "export", "import", "view_detail", "data_profile", "table_profile",
    ...TX,
  ]),
};

const PROVIDERS = [SQLITE, MSSQL];

export interface Routed {
  cmd: string;
  args?: Record<string, unknown>;
}

const idOf = (args?: Record<string, unknown>): unknown =>
  args?.sessionId ?? args?.txId ?? args?.exportId ?? (args?.req as { sessionId?: string } | undefined)?.sessionId;

export function routeCommand(cmd: string, args?: Record<string, unknown>): Routed {
  const id = idOf(args);
  if (cmd.startsWith("console_") && cmd !== "console_is_complete" && typeof id === "string") {
    const p = PROVIDERS.find((x) => x.console && id.startsWith(x.prefix));
    if (p) return { cmd: `${p.prefix}${cmd}`, args };
  }
  if (!cmd.startsWith("pg_")) return { cmd, args };
  const params = args?.params as { provider?: string } | undefined;
  const p = PROVIDERS.find((x) => (typeof id === "string" && id.startsWith(x.prefix)) || (cmd === "pg_test_connection" && params?.provider === x.param));
  if (!p) return { cmd, args };
  const rest = cmd.slice(3);
  return p.commands.has(rest) ? { cmd: p.prefix + rest, args } : { cmd: `${p.prefix}unsupported`, args: { what: cmd } };
}

/** O id de uma conexão nova: o PostgreSQL mantém o formato de sempre; os outros bancos levam o nome do banco na frente. */
export function newConnectionId(provider: string, time: string): string {
  const p = PROVIDERS.find((x) => x.param === provider);
  return p ? `${p.prefix}${time}` : `c${time}`;
}

/** O começo do id de uma operação longa (busca, exportação) que o roteador usa para achar o banco; vazio no PostgreSQL. */
export const idPrefix = (connId: string | undefined | null): string => PROVIDERS.find((p) => typeof connId === "string" && connId.startsWith(p.prefix))?.prefix ?? "";

/** A conexão (ou transação) é de SQLite? */
export const isSqliteId = (id: string | undefined | null): boolean => typeof id === "string" && id.startsWith(SQLITE_PREFIX);

/** A conexão (ou transação) é de SQL Server? */
export const isMssqlId = (id: string | undefined | null): boolean => typeof id === "string" && id.startsWith(MSSQL_PREFIX);
