/**
 * Escolha do comando que atende uma chamada. As telas continuam chamando `pg_*`; quando a conexão é de outro banco (o id da conexão
 * começa pelo nome do banco, como `sqlite_...` e `mssql_...`), a chamada vai para o comando equivalente desse banco. Para o
 * PostgreSQL nada muda: o comando e os argumentos saem exatamente como entraram.
 */

export const SQLITE_PREFIX = "sqlite_";
export const MSSQL_PREFIX = "mssql_";
export const MYSQL_PREFIX = "mysql_";
export const FIREBIRD_PREFIX = "firebird_";

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
    "execute", "cancel", "table_rows", "table_count", "apply", "fk_labels", "data_profile", "table_profile", "find", "find_data", "export_cancel", "schema_diagram", "export", "import", "backup", "backup_list", "restore", "dump_sql", "transfer", "view_detail",
    ...TX,
  ]),
};

const MSSQL: Provider = {
  prefix: MSSQL_PREFIX,
  param: "sqlserver",
  console: true,
  commands: new Set([
    "test_connection", "open", "close", "close_database", "list_databases", "list_schemas", "list_objects", "table_detail", "list_roles",
    "execute", "cancel", "table_rows", "table_count", "apply", "fk_labels", "schema_diagram", "find", "find_data", "export_cancel", "export", "import", "view_detail", "data_profile", "table_profile", "routine_definition", "dump_sql", "transfer", "backup", "backup_list", "backup_delete", "restore",
    ...TX,
  ]),
};

const MYSQL: Provider = {
  prefix: MYSQL_PREFIX,
  param: "mysql",
  console: true,
  commands: new Set([
    "test_connection", "open", "close", "close_database", "list_databases", "list_schemas", "list_objects", "table_detail", "list_roles",
    "execute", "cancel", "table_rows", "table_count", "apply", "fk_labels", "schema_diagram", "view_detail", "export", "export_cancel", "import", "dump_sql", "transfer", "routine_definition", "find", "find_data", "data_profile", "table_profile", "database_detail", "collations", "backup", "backup_list", "restore",
    ...TX,
  ]),
};

const FIREBIRD: Provider = {
  prefix: FIREBIRD_PREFIX,
  param: "firebird",
  console: true,
  commands: new Set([
    "test_connection", "open", "close", "close_database", "list_databases", "list_schemas", "list_objects", "table_detail", "list_roles",
    "execute", "cancel", "table_rows", "table_count", "apply", "view_detail", "routine_definition", "export", "export_cancel", "import", "data_profile", "table_profile", "schema_diagram", "fk_labels", "find", "find_data", "dump_sql", "backup", "backup_list", "restore", "transfer",
    ...TX,
  ]),
};

const PROVIDERS = [SQLITE, MSSQL, MYSQL, FIREBIRD];

export interface Routed {
  cmd: string;
  args?: Record<string, unknown>;
}

const idOf = (args?: Record<string, unknown>): unknown =>
  args?.sessionId ?? args?.txId ?? args?.exportId ?? args?.conn ?? (args?.req as { sourceConn?: string } | undefined)?.sourceConn ?? args?.compareId ?? (args?.req as { compareId?: string; targetConn?: string } | undefined)?.compareId ?? (args?.req as { targetConn?: string } | undefined)?.targetConn ?? (args?.req as { sessionId?: string; conn?: string } | undefined)?.sessionId ?? (args?.req as { conn?: string } | undefined)?.conn;

export function routeCommand(cmd: string, args?: Record<string, unknown>): Routed {
  const id = idOf(args);
  if (cmd === "pg_transfer") {
    const r = args?.req as { sourceConn?: string; targetConn?: string } | undefined;
    if (productOf(r?.sourceConn) !== productOf(r?.targetConn)) return { cmd: "x_transfer", args };
  }
  if (cmd === "pg_export_cancel" && typeof args?.exportId === "string" && args.exportId.startsWith("xp_")) return { cmd: "ext_cancel", args };
  if (cmd.startsWith("datasync_") || cmd.startsWith("structsync_")) {
    // Fora do PostgreSQL e entre produtos as comparações são as `ext_*`; qualquer id de um lado ou da operação diz isso.
    const r = args?.req as { compareId?: string; deployId?: string; sourceConn?: string; targetConn?: string } | undefined;
    const ids = [args?.compareId, r?.compareId, r?.deployId, r?.sourceConn, r?.targetConn];
    if (ids.some((x) => typeof x === "string" && (x.startsWith("xp_") || PROVIDERS.some((p) => x.startsWith(p.prefix))))) return { cmd: `ext_${cmd}`, args };
  }
  if ((cmd.startsWith("monitor_") || cmd.startsWith("datagen_")) && typeof id === "string") {
    const p = PROVIDERS.find((x) => id.startsWith(x.prefix));
    if (p) return { cmd: `${p.prefix}${cmd}`, args };
  }
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
/** O produto de um id de conexão ("pg" para o PostgreSQL, que não tem prefixo). */
const productOf = (id: unknown): string => (typeof id === "string" ? PROVIDERS.find((p) => id.startsWith(p.prefix))?.param ?? "pg" : "pg");

/**
 * O começo do id de uma operação entre duas conexões: o do produto, quando os dois lados são o mesmo, e `xp_` quando são
 * produtos diferentes (as ferramentas entre produtos têm comandos próprios).
 */
export const opPrefix = (a: string | undefined | null, b: string | undefined | null): string => (productOf(a) !== productOf(b) ? "xp_" : idPrefix(a));

export const idPrefix = (connId: string | undefined | null): string => PROVIDERS.find((p) => typeof connId === "string" && connId.startsWith(p.prefix))?.prefix ?? "";

/** O banco de um id de conexão, no formato que as telas de SQL usam (PostgreSQL = sem valor). */
export const dialectOfId = (id: string | undefined | null): "sqlite" | "mssql" | "mysql" | "firebird" | undefined => (isSqliteId(id) ? "sqlite" : isMssqlId(id) ? "mssql" : isMysqlId(id) ? "mysql" : isFirebirdId(id) ? "firebird" : undefined);

/** A conexão (ou transação) é de SQLite? */
export const isSqliteId = (id: string | undefined | null): boolean => typeof id === "string" && id.startsWith(SQLITE_PREFIX);

/** A conexão (ou transação) é de MySQL? */
export const isMysqlId = (id: string | undefined | null): boolean => typeof id === "string" && id.startsWith(MYSQL_PREFIX);

/** A conexão (ou transação) é de Firebird? */
export const isFirebirdId = (id: string | undefined | null): boolean => typeof id === "string" && id.startsWith(FIREBIRD_PREFIX);

/** A conexão (ou transação) é de SQL Server? */
export const isMssqlId = (id: string | undefined | null): boolean => typeof id === "string" && id.startsWith(MSSQL_PREFIX);
