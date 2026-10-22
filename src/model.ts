/*
 * Modelo de navegação do shell e dados fictícios do M0.
 * As fixtures seguem a especificação (§44.3); nenhum servidor, usuário ou dado real.
 * No M1 estes dados passam a vir dos comandos IPC do núcleo Rust.
 */

export type Category =
  | "table" | "view" | "matview" | "function" | "sequence" | "type"
  | "user" | "query" | "backup" | "automation" | "model" | "bi";

export const OTHERS_CATEGORIES: Category[] = ["sequence", "type"];

export type ProviderId = "postgresql" | "sqlserver" | "mysql" | "mariadb" | "sqlite";

/* ---------- Estrutura de tabela ---------- */

export interface ColumnDef {
  name: string;
  type: string;
  length: number;
  decimal: number;
  notNull: boolean;
  /** Posição na chave primária (1..n). */
  key?: number;
  default?: string;
  comment?: string;
  collation?: string;
  /** Tipo completo para casts de parâmetros (ex.: "numeric(19,4)"), quando vindo do servidor. */
  castType?: string;
}

export interface IndexDef { name: string; fields: string; method: string; unique: boolean; concurrently: boolean; comment?: string }
export interface ForeignKeyDef { name: string; fields: string; refSchema: string; refTable: string; refFields: string; onDelete: string; onUpdate: string; comment?: string }
export interface UniqueDef { name: string; fields: string; comment?: string }
export interface CheckDef { name: string; expression: string; noInherit: boolean; comment?: string }
export interface TriggerDef {
  name: string; forEach: "ROW" | "STATEMENT"; fires: "BEFORE" | "AFTER" | "INSTEAD OF";
  insert: boolean; update: boolean; delete: boolean; truncate: boolean; enable: boolean; fn: string; comment?: string;
}

export interface DbObject {
  name: string;
  rows?: number;
  dataLength?: number;
  modified?: string;
  comment?: string;
  owner?: string;
  returnType?: string;
  language?: string;
  dataType?: string;
  args?: string;
  canLogin?: boolean;
  superuser?: boolean;
  modifiedUser?: string;
  columns?: ColumnDef[];
  indexes?: IndexDef[];
  foreignKeys?: ForeignKeyDef[];
  uniques?: UniqueDef[];
  checks?: CheckDef[];
  triggers?: TriggerDef[];
  /** View: o SELECT (dados de demonstração e após salvar no designer). */
  definition?: string;
}

/** `loaded` = false indica nível ainda não lido do servidor (carga preguiçosa). */
export interface SchemaMock { name: string; objects: Partial<Record<Category, DbObject[]>>; loaded?: boolean }
export interface DatabaseMock { name: string; schemas: SchemaMock[]; loaded?: boolean }

export interface ConnectionMock {
  id: string;
  name: string;
  provider: ProviderId;
  host: string;
  port: number;
  user: string;
  initialDatabase: string;
  serverVersion: string;
  serverVersionNum: number;
  databases: DatabaseMock[];
  hiddenDatabases: number;
  roles: DbObject[];
  /** Local | Development | Test | Staging | Production | Custom (spec §42.4). */
  environment?: string;
  /** Conexão real (app desktop) em vez de dados fictícios. */
  live?: boolean;
  sslMode?: "disable" | "prefer" | "require" | "verify-ca" | "verify-full";
  /** Cor de identificação na árvore (menu "Cor"). */
  color?: ConnectionColor;
  /** Favorita (menu "Adicionar estrela"). */
  starred?: boolean;
  /** Há senha salva no cofre de credenciais do sistema para este perfil. */
  hasPassword?: boolean;
}

export type ConnectionColor = "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "gray";
export const CONNECTION_COLORS: ConnectionColor[] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

const col = (name: string, type: string, length = 0, decimal = 0, notNull = false, extra: Partial<ColumnDef> = {}): ColumnDef =>
  ({ name, type, length, decimal, notNull, ...extra });

const TEXT_COLLATION = '"pg_catalog"."default"';

const fixtureTables = (): DbObject[] => [
  {
    name: "audit_log", rows: 34, owner: "atlas_test", comment: "",
    columns: [
      col("id", "int4", 32, 0, true, { key: 1, default: "nextval('audit_log_id_seq'::regclass)" }),
      col("table_name", "text", 0, 0, false, { collation: TEXT_COLLATION }),
      col("op", "text", 0, 0, false, { collation: TEXT_COLLATION }),
      col("at", "timestamptz", 6, 0, true, { default: "now()" }),
    ],
  },
  {
    name: "audit_trigger_test", rows: 0, owner: "atlas_test",
    columns: [col("id", "int4", 32, 0, true, { key: 1, default: "nextval('audit_trigger_test_id_seq'::regclass)" }), col("label", "text", 0, 0, false, { collation: TEXT_COLLATION })],
    triggers: [{ name: "trg_audit", forEach: "ROW", fires: "AFTER", insert: true, update: true, delete: true, truncate: false, enable: true, fn: "public.fn_audit_trigger" }],
  },
  {
    name: "client_notes", rows: 0, owner: "atlas_test",
    columns: [
      col("client_id", "uuid", 0, 0, true, { key: 1 }),
      col("note_id", "int4", 32, 0, true, { key: 2 }),
      col("body", "text", 0, 0, false, { collation: TEXT_COLLATION }),
    ],
    foreignKeys: [{ name: "client_notes_client_id_fkey", fields: "client_id", refSchema: "public", refTable: "clients", refFields: "id", onDelete: "CASCADE", onUpdate: "NO ACTION" }],
  },
  {
    name: "clients", rows: 4, owner: "atlas_test", comment: "Fixture: PK simples, numeric(19,4), jsonb, Unicode.",
    columns: [
      col("id", "uuid", 0, 0, true, { key: 1 }),
      col("name", "varchar", 150, 0, true, { collation: TEXT_COLLATION }),
      col("total", "numeric", 19, 4, false),
      col("created", "timestamptz", 6, 0, true, { default: "now()" }),
      col("metadata", "jsonb", 0, 0, false),
    ],
    indexes: [{ name: "idx_clients_name", fields: "name", method: "btree", unique: false, concurrently: false }],
    checks: [{ name: "clients_total_check", expression: "total >= 0", noInherit: false }],
  },
  {
    name: "conflict_test", rows: 2, owner: "atlas_test",
    columns: [col("id", "int4", 32, 0, true, { key: 1 }), col("value", "text", 0, 0, false, { collation: TEXT_COLLATION }), col("version", "int4", 32, 0, true, { default: "1" })],
  },
  {
    name: "large_blob", rows: 0, owner: "atlas_test",
    columns: [col("id", "int4", 32, 0, true, { key: 1 }), col("payload", "bytea", 0, 0, false)],
  },
  {
    name: "legacy_no_key", rows: 0, owner: "atlas_test", comment: "",
    columns: [col("code", "text", 0, 0, false, { collation: TEXT_COLLATION }), col("description", "text", 0, 0, false, { collation: TEXT_COLLATION })],
  },
  {
    name: "orders", rows: 3, owner: "atlas_test",
    columns: [
      col("id", "int8", 64, 0, true, { key: 1, default: "nextval('orders_id_seq'::regclass)" }),
      col("client_id", "uuid", 0, 0, true),
      col("amount", "numeric", 12, 2, true),
      col("status", "text", 0, 0, true, { default: "'open'::text", collation: TEXT_COLLATION }),
    ],
    foreignKeys: [{ name: "orders_client_id_fkey", fields: "client_id", refSchema: "public", refTable: "clients", refFields: "id", onDelete: "RESTRICT", onUpdate: "NO ACTION" }],
    uniques: [{ name: "orders_client_status_key", fields: "client_id, status" }],
  },
];

const publicSchema = (): SchemaMock => ({
  name: "public",
  objects: {
    table: fixtureTables(),
    view: [{
      name: "v_open_orders", owner: "atlas_test",
      definition: "SELECT o.id AS order_id,\n    o.client_id,\n    o.amount\n   FROM orders o\n  WHERE o.status = 'open'::text",
      columns: [col("order_id", "int8", 64, 0, false), col("client_id", "uuid", 0, 0, false), col("amount", "numeric", 12, 2, false)],
    }],
    matview: [],
    function: [
      { name: "fn_audit_trigger", args: "", returnType: "trigger", language: "plpgsql", owner: "atlas_test" },
      { name: "fn_total_by_client", args: "p_client uuid", returnType: "numeric", language: "sql", owner: "atlas_test" },
    ],
    sequence: [
      { name: "audit_log_id_seq", dataType: "int8", owner: "atlas_test" },
      { name: "orders_id_seq", dataType: "int8", owner: "atlas_test" },
    ],
    type: [],
    query: [],
    backup: [],
  },
});

const weirdSchema = (): SchemaMock => ({
  name: "Weird.Schema",
  objects: {
    table: [
      { name: "Mixed Case", rows: 1, owner: "atlas_test", columns: [col("Id", "int4", 32, 0, true, { key: 1 }), col("Ação", "text", 0, 0, false, { collation: TEXT_COLLATION })] },
      { name: "dot.table", rows: 0, owner: "atlas_test", columns: [col("id", "int4", 32, 0, true, { key: 1 })] },
    ],
    view: [], matview: [], function: [], sequence: [], type: [], query: [], backup: [],
  },
});

const emptyPublic = (): SchemaMock => ({ name: "public", objects: { table: [], view: [], matview: [], function: [], query: [], backup: [] } });

export const INITIAL_CONNECTIONS: ConnectionMock[] = [
  {
    id: "pg96", name: "PostgreSQL 9.6", provider: "postgresql", host: "localhost", port: 5432,
    user: "postgres", initialDatabase: "postgres", serverVersion: "9.6.19", serverVersionNum: 90619,
    databases: [
      { name: "atlas_demo", schemas: [publicSchema(), weirdSchema()] },
      { name: "inventory_demo", schemas: [emptyPublic()] },
      { name: "postgres", schemas: [emptyPublic()] },
    ],
    hiddenDatabases: 2,
    roles: [
      { name: "atlas_test", canLogin: true, superuser: false },
      { name: "postgres", canLogin: true, superuser: true },
    ],
  },
  {
    id: "pg18", name: "PostgreSQL 18", provider: "postgresql", host: "localhost", port: 5433,
    user: "postgres", initialDatabase: "postgres", serverVersion: "18.0", serverVersionNum: 180000,
    databases: [
      { name: "atlas_demo", schemas: [publicSchema()] },
      { name: "postgres", schemas: [emptyPublic()] },
    ],
    hiddenDatabases: 2,
    roles: [{ name: "postgres", canLogin: true, superuser: true }],
  },
];

/* ---------- Linhas fictícias para o grid ---------- */

export type CellValue = string | null;

function uuid(n: number): string {
  const h = (n * 2654435761 >>> 0).toString(16).padStart(8, "0");
  return `${h}-${h.slice(0, 4)}-4${h.slice(1, 4)}-a${h.slice(2, 5)}-${h}${h.slice(0, 4)}`;
}

/** Gera linhas determinísticas coerentes com os tipos das colunas. */
export function mockRows(table: DbObject): CellValue[][] {
  const cols = table.columns ?? [];
  const count = table.rows ?? 0;
  const special: Record<string, CellValue[][]> = {
    clients: [
      [uuid(1), "Ana Souza", "1520.5000", "2026-10-15 17:04:14.333783-03", '{"tier": "gold"}'],
      [uuid(2), "José Ação Ünicode 😀", "0.0000", "2026-10-15 17:05:01.120044-03", null],
      [uuid(3), "", "99999.9999", "2026-10-16 09:12:44.000001-03", '{"tags": ["a", "b"]}'],
      [uuid(4), "O'Brien \"quoted\"", null, "2026-10-20 20:02:17.776452-03", "{}"],
    ],
    conflict_test: [["1", "original", "1"], ["2", "segundo", "3"]],
    // FK client_id → clients.id (demonstra o lookup de chave estrangeira).
    orders: [
      ["1", uuid(1), "150.00", "open"],
      ["2", uuid(2), "89.90", "paid"],
      ["3", uuid(1), "42.00", "open"],
    ],
    v_open_orders: [["1", uuid(1), "150.00"], ["3", uuid(1), "42.00"]],
    "Mixed Case": [["1", "ação"]],
  };
  if (special[table.name]) return special[table.name];
  const out: CellValue[][] = [];
  for (let i = 0; i < count; i++) {
    out.push(
      cols.map((c) => {
        if (c.key === 1 && c.type.startsWith("int")) return String(2 + i * 2 + (i > 20 ? 1 : 0));
        if (c.type === "uuid") return uuid(i + 10);
        if (c.type === "timestamptz") {
          const m = 4 + i * 3;
          return `2026-10-15 ${String(17 + Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}:${String((i * 7) % 60).padStart(2, "0")}.${String(100000 + i * 37717).slice(0, 6)}-03`;
        }
        if (c.name === "table_name") return "audit_trigger_test";
        if (c.name === "op") return "INSERT";
        if (c.type.startsWith("int") || c.type === "numeric") return String(i + 1);
        return `${c.name} ${i + 1}`;
      }),
    );
  }
  return out;
}

/* ---------- Árvore de navegação ---------- */

export type NodeKind = "root" | "connection" | "database" | "schema" | "folder" | "object" | "column";

export interface TreeNode {
  id: string;
  kind: NodeKind;
  label: string;
  depth: number;
  expandable: boolean;
  category?: Category;
  provider?: ProviderId;
  color?: ConnectionColor;
  starred?: boolean;
}

/** Pastas exibidas sob um schema PostgreSQL, na ordem da referência. */
export const SCHEMA_FOLDERS: Category[] = ["table", "view", "matview", "function", "query", "backup"];

export interface NodeContext {
  connection?: ConnectionMock;
  database?: DatabaseMock;
  schema?: SchemaMock;
  category?: Category;
  objectName?: string;
  object?: DbObject;
}

/**
 * Ids: conn:<c> | db:<c>/<d> | schema:<c>/<d>/<s> | folder:<c>/<d>/<s>/<cat>
 *      | obj:<c>/<d>/<s>/<cat>/<nome> | col:<c>/<d>/<s>/table/<tabela>/<coluna>
 * Cada segmento é codificado com encodeURIComponent (nomes podem conter "/" ou ".").
 */
export const seg = encodeURIComponent;

export function resolveNode(id: string | null, connections: ConnectionMock[]): NodeContext {
  if (!id || id === "root") return {};
  const sep = id.indexOf(":");
  const parts = id.slice(sep + 1).split("/").map(decodeURIComponent);
  const [c, d, s, cat, name] = parts;
  const connection = connections.find((x) => x.id === c);
  const database = connection?.databases.find((x) => x.name === d);
  const schema = database?.schemas.find((x) => x.name === s);
  const category = cat as Category | undefined;
  const object = name && category ? schema?.objects[category]?.find((o) => o.name === name) : undefined;
  return { connection, database, schema, category, objectName: name, object };
}

export interface TreeState {
  expanded: Set<string>;
  openConnections: Set<string>;
  openDatabases: Set<string>;
  filter: string;
  /** Mostrar só conexões com estrela (botão ★ da busca). */
  starredOnly?: boolean;
}

export function functionLabel(o: DbObject): string {
  return o.args !== undefined ? `${o.name}(${o.args})` : o.name;
}

export function buildVisibleTree(
  connections: ConnectionMock[],
  state: TreeState,
  folderLabel: (c: Category) => string,
  rootLabel: string,
): TreeNode[] {
  const out: TreeNode[] = [];
  const filter = state.filter.trim().toLowerCase();
  const matches = (label: string) => !filter || label.toLowerCase().includes(filter);

  out.push({ id: "root", kind: "root", label: rootLabel, depth: 0, expandable: false });

  for (const conn of connections) {
    const connId = `conn:${seg(conn.id)}`;
    const isOpen = state.openConnections.has(conn.id);
    if (state.starredOnly && !conn.starred) continue;
    if (!isOpen && !matches(conn.name)) continue;
    out.push({ id: connId, kind: "connection", label: conn.name, depth: 0, expandable: true, provider: conn.provider, color: conn.color, starred: conn.starred });
    if (!isOpen || !state.expanded.has(connId)) continue;

    for (const db of conn.databases) {
      const dbId = `db:${seg(conn.id)}/${seg(db.name)}`;
      const dbOpen = state.openDatabases.has(dbId);
      out.push({ id: dbId, kind: "database", label: db.name, depth: 1, expandable: true });
      if (!dbOpen || !state.expanded.has(dbId)) continue;

      for (const sc of db.schemas) {
        const scPath = `${seg(conn.id)}/${seg(db.name)}/${seg(sc.name)}`;
        const scId = `schema:${scPath}`;
        out.push({ id: scId, kind: "schema", label: sc.name, depth: 2, expandable: true });
        if (!state.expanded.has(scId)) continue;

        for (const cat of SCHEMA_FOLDERS) {
          const fId = `folder:${scPath}/${cat}`;
          const items = sc.objects[cat] ?? [];
          const listsChildren = cat === "table" || cat === "view" || cat === "matview" || cat === "function";
          out.push({ id: fId, kind: "folder", label: folderLabel(cat), depth: 3, expandable: listsChildren && items.length > 0, category: cat });
          if (!listsChildren || !state.expanded.has(fId)) continue;
          for (const o of items) {
            if (!matches(o.name)) continue;
            const oId = `obj:${scPath}/${cat}/${seg(o.name)}`;
            // Colunas ainda não lidas (conexão real) também deixam a tabela expansível.
            const hasCols = cat === "table" && (o.columns === undefined || o.columns.length > 0);
            out.push({ id: oId, kind: "object", label: cat === "function" ? functionLabel(o) : o.name, depth: 4, expandable: hasCols, category: cat });
            if (!hasCols || !state.expanded.has(oId)) continue;
            // DECISÃO_ATLAS: tabela expandida mostra suas colunas com o tipo.
            for (const c of o.columns ?? []) {
              out.push({ id: `col:${scPath}/table/${seg(o.name)}/${seg(c.name)}`, kind: "column", label: `${c.name} (${c.type})`, depth: 5, expandable: false });
            }
          }
        }
      }
    }
  }
  return out;
}

export function parentId(id: string): string | null {
  const sep = id.indexOf(":");
  const kind = id.slice(0, sep);
  const parts = id.slice(sep + 1).split("/");
  switch (kind) {
    case "col": return `obj:${parts.slice(0, 5).join("/")}`;
    case "obj": return `folder:${parts.slice(0, 4).join("/")}`;
    case "folder": return `schema:${parts.slice(0, 3).join("/")}`;
    case "schema": return `db:${parts.slice(0, 2).join("/")}`;
    case "db": return `conn:${parts[0]}`;
    default: return null;
  }
}

/* ---------- Formatação ---------- */

export function formatBytes(n?: number): string {
  if (n === undefined) return "";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(2)} ${units[i]}`;
}

export function formatInt(n: number | undefined, lang: string): string {
  return n === undefined ? "" : n.toLocaleString(lang);
}

/** Quoting de identificador PostgreSQL (aspas duplas, aspas internas duplicadas). */
export function quoteIdent(name: string): string {
  return `"${name.replace(/"/g, '""')}"`;
}
