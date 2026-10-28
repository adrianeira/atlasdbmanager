/** Modelo do Query Builder visual e o gerador do SELECT (puro, sem acesso ao banco). */

export interface QbFk {
  fields: string[];
  refSchema: string;
  refTable: string;
  refFields: string[];
}

export interface QbTable {
  id: string;
  schema: string;
  name: string;
  /** Vazio = usa o nome da tabela. */
  alias: string;
  view: boolean;
  x: number;
  y: number;
  /** Tamanho escolhido pelo usuário (alça do canto); vazio = automático. */
  w?: number;
  h?: number;
  /** Vazio enquanto as colunas ainda estão sendo lidas. */
  columns: string[];
  fks: QbFk[];
}

/** Campo de uma tabela do desenho (`column` "*" = todas as colunas). */
export interface QbRef {
  table: string;
  column: string;
}

export type Aggregate = "" | "COUNT" | "SUM" | "AVG" | "MIN" | "MAX";
export const AGGREGATES: Aggregate[] = ["", "COUNT", "SUM", "AVG", "MIN", "MAX"];

export interface QbField {
  id: string;
  ref: QbRef;
  aggregate: Aggregate;
  alias: string;
}

export type JoinType = "INNER" | "LEFT" | "RIGHT" | "FULL";
export const JOIN_TYPES: JoinType[] = ["INNER", "LEFT", "RIGHT", "FULL"];

/** O banco do desenho; sem valor = PostgreSQL. */
export type QbDialect = "sqlite" | "mssql" | "mysql" | "firebird" | "oracle" | undefined;

/** O MySQL não tem `FULL JOIN`. */
export const joinTypesFor = (d: QbDialect): JoinType[] => (d === "mysql" ? JOIN_TYPES.filter((j) => j !== "FULL") : JOIN_TYPES);

export interface QbJoin {
  id: string;
  type: JoinType;
  left: QbRef;
  right: QbRef;
}

export type Operator = "=" | "<>" | "<" | ">" | "<=" | ">=" | "LIKE" | "NOT LIKE" | "ILIKE" | "IN" | "NOT IN" | "BETWEEN" | "IS NULL" | "IS NOT NULL";
export const OPERATORS: Operator[] = ["=", "<>", "<", ">", "<=", ">=", "LIKE", "NOT LIKE", "ILIKE", "IN", "NOT IN", "BETWEEN", "IS NULL", "IS NOT NULL"];
export const NO_VALUE: Operator[] = ["IS NULL", "IS NOT NULL"];

/** `ILIKE` só existe no PostgreSQL. */
export const operatorsFor = (d: QbDialect): Operator[] => (d ? OPERATORS.filter((o) => o !== "ILIKE") : OPERATORS);

export interface QbCond {
  id: string;
  connector: "AND" | "OR";
  ref: QbRef | null;
  /** Só no HAVING. */
  aggregate: Aggregate;
  op: Operator;
  value: string;
  /** Segundo valor do BETWEEN. */
  value2: string;
}

export interface QbOrder {
  id: string;
  ref: QbRef;
  desc: boolean;
}

export interface QbState {
  tables: QbTable[];
  fields: QbField[];
  joins: QbJoin[];
  where: QbCond[];
  groupBy: QbRef[];
  having: QbCond[];
  orderBy: QbOrder[];
  distinct: boolean;
  limit: string;
  offset: string;
}

export const emptyState = (): QbState => ({ tables: [], fields: [], joins: [], where: [], groupBy: [], having: [], orderBy: [], distinct: false, limit: "", offset: "" });

const RESERVED = new Set([
  "all", "analyse", "analyze", "and", "any", "array", "as", "asc", "asymmetric", "both", "case", "cast", "check", "collate", "column", "constraint", "create",
  "current_date", "current_time", "current_timestamp", "current_user", "default", "deferrable", "desc", "distinct", "do", "else", "end", "except", "false",
  "fetch", "for", "foreign", "from", "grant", "group", "having", "in", "initially", "intersect", "into", "lateral", "leading", "limit", "localtime",
  "localtimestamp", "not", "null", "offset", "on", "only", "or", "order", "placing", "primary", "references", "returning", "select", "session_user", "some",
  "symmetric", "table", "then", "to", "trailing", "true", "union", "unique", "user", "using", "variadic", "when", "where", "window", "with",
  "left", "right", "inner", "outer", "join", "full", "cross", "natural", "like", "ilike", "is", "isnull", "notnull", "overlaps", "similar", "verbose", "binary", "freeze",
]);

/** Aspas só quando preciso (maiúsculas, espaços, palavra reservada...), como num SELECT escrito à mão. */
export function ident(name: string, d?: QbDialect): string {
  // Firebird: sem aspas o nome vira maiúsculo; os que o banco guarda em minúsculas só se alcançam entre aspas.
  // Oracle: os nomes ficam em maiúsculas no catálogo, então só entre aspas se alcançam todos.
  if (d === "firebird" || d === "oracle") return `"${name.replace(/"/g, '""')}"`;
  if (/^[a-z_][a-z0-9_$]*$/.test(name) && !RESERVED.has(name)) return name;
  if (d === "mysql") return `\`${name.replace(/`/g, "``")}\``;
  if (d === "mssql") return `[${name.replace(/]/g, "]]")}]`;
  return `"${name.replace(/"/g, '""')}"`;
}

const qualifierOf = (t: QbTable, d?: QbDialect) => (t.alias.trim() ? ident(t.alias.trim(), d) : ident(t.name, d));

const tableSql = (t: QbTable, defaultSchema: string, d?: QbDialect) => {
  const base = t.schema === defaultSchema ? ident(t.name, d) : `${ident(t.schema, d)}.${ident(t.name, d)}`;
  return t.alias.trim() ? `${base} AS ${ident(t.alias.trim(), d)}` : base;
};

function refSql(state: QbState, r: QbRef, d?: QbDialect): string {
  const t = state.tables.find((x) => x.id === r.table);
  if (!t) return ident(r.column, d);
  return r.column === "*" ? `${qualifierOf(t, d)}.*` : `${qualifierOf(t, d)}.${ident(r.column, d)}`;
}

/** Valor digitado → literal SQL: número, true/false/null ficam como estão; o resto vira texto entre aspas simples. */
export function literal(value: string, d?: QbDialect): string {
  const v = value.trim();
  // O SQL Server e o SQLite não têm `true`/`false` literais (o SQLite os aceita só a partir da 3.23).
  if (/^(true|false)$/i.test(v) && d === "mssql") return /^true$/i.test(v) ? "1" : "0";
  if (/^-?\d+(\.\d+)?$/.test(v) || /^(true|false|null)$/i.test(v)) return v;
  if (/^'(?:[^']|'')*'$/.test(v)) return v;
  // O MySQL trata a barra invertida como escape dentro do texto.
  const text = d === "mysql" ? value.replace(/\\/g, "\\\\") : value;
  return `'${text.replace(/'/g, "''")}'`;
}

function condSql(state: QbState, c: QbCond, having: boolean, d?: QbDialect): string | null {
  if (!c.ref && !(having && c.aggregate === "COUNT")) return null;
  let left: string;
  if (c.ref) {
    const base = refSql(state, c.ref, d);
    left = having && c.aggregate ? `${c.aggregate}(${c.ref.column === "*" ? "*" : base})` : base;
  } else {
    left = "COUNT(*)";
  }
  if (NO_VALUE.includes(c.op)) return `${left} ${c.op}`;
  if (c.op === "IN" || c.op === "NOT IN") {
    const items = c.value.split(",").map((x) => x.trim()).filter(Boolean);
    return items.length ? `${left} ${c.op} (${items.map((x) => literal(x, d)).join(", ")})` : null;
  }
  if (c.op === "BETWEEN") return c.value.trim() && c.value2.trim() ? `${left} BETWEEN ${literal(c.value, d)} AND ${literal(c.value2, d)}` : null;
  return c.value.trim() === "" && !/LIKE/.test(c.op) ? null : `${left} ${c.op} ${literal(c.value, d)}`;
}

function condList(state: QbState, list: QbCond[], having: boolean, d?: QbDialect): string[] {
  const out: string[] = [];
  for (const c of list) {
    const s = condSql(state, c, having, d);
    if (s) out.push(out.length === 0 ? s : `${c.connector} ${s}`);
  }
  return out;
}

/** FROM com os JOINs: cada tabela entra ligada às que já estão no FROM; sem ligação, vira CROSS JOIN. */
function fromSql(state: QbState, defaultSchema: string, d?: QbDialect): string[] {
  const [first, ...rest] = state.tables;
  if (!first) return [];
  const lines = [`  ${tableSql(first, defaultSchema, d)}`];
  const placed = new Set([first.id]);
  let pending = rest;
  while (pending.length) {
    let progressed = false;
    for (const t of [...pending]) {
      const links = state.joins.filter((j) => (j.left.table === t.id && placed.has(j.right.table)) || (j.right.table === t.id && placed.has(j.left.table)));
      if (!links.length) continue;
      lines.push(`  ${links[0].type} JOIN`, `  ${tableSql(t, defaultSchema, d)}`, "  ON");
      links.forEach((j, i) => lines.push(`    ${i ? "AND " : ""}${refSql(state, j.left, d)} = ${refSql(state, j.right, d)}`));
      placed.add(t.id);
      pending = pending.filter((x) => x.id !== t.id);
      progressed = true;
    }
    if (!progressed) {
      for (const t of pending) lines.push("  CROSS JOIN", `  ${tableSql(t, defaultSchema, d)}`);
      break;
    }
  }
  return lines;
}

/** O SELECT do desenho, em várias linhas (uma cláusula por bloco). */
export function buildSql(state: QbState, defaultSchema: string, d?: QbDialect): string {
  const out: string[] = [];
  const limit = /^\d+$/.test(state.limit.trim()) ? state.limit.trim() : "";
  const offset = /^\d+$/.test(state.offset.trim()) ? state.offset.trim() : "";
  // SQL Server: sem deslocamento, `TOP n`; com deslocamento, `OFFSET ... FETCH` (que exige ORDER BY).
  const top = d === "mssql" && limit && !offset ? ` TOP ${limit}` : "";
  out.push(`${state.distinct ? "SELECT DISTINCT" : "SELECT"}${top}`);
  const items = state.fields.map((f) => {
    const base = f.aggregate ? `${f.aggregate}(${f.ref.column === "*" && f.aggregate === "COUNT" ? "*" : refSql(state, f.ref, d)})` : refSql(state, f.ref, d);
    return f.alias.trim() ? `${base} AS ${ident(f.alias.trim(), d)}` : base;
  });
  if (items.length === 0) out.push("  *");
  else items.forEach((s, i) => out.push(`  ${s}${i < items.length - 1 ? "," : ""}`));

  const from = fromSql(state, defaultSchema, d);
  if (from.length) out.push("FROM", ...from);
  const where = condList(state, state.where, false, d);
  if (where.length) out.push("WHERE", ...where.map((l) => `  ${l}`));
  const group = state.groupBy.map((r) => refSql(state, r, d));
  if (group.length) out.push("GROUP BY", ...group.map((l, i) => `  ${l}${i < group.length - 1 ? "," : ""}`));
  const having = condList(state, state.having, true, d);
  if (having.length) out.push("HAVING", ...having.map((l) => `  ${l}`));
  const order = state.orderBy.map((o) => `${refSql(state, o.ref, d)}${o.desc ? " DESC" : ""}`);
  if (order.length) out.push("ORDER BY", ...order.map((l, i) => `  ${l}${i < order.length - 1 ? "," : ""}`));
  if (d === "mssql") {
    if (offset) {
      if (!order.length) out.push("ORDER BY", "  (SELECT NULL)");
      out.push(`OFFSET ${offset} ROWS`);
      if (limit) out.push(`FETCH NEXT ${limit} ROWS ONLY`);
    }
    return out.join("\n");
  }
  // Oracle (12c em diante): `OFFSET n ROWS FETCH NEXT m ROWS ONLY`.
  if (d === "oracle") {
    if (offset) out.push(`OFFSET ${offset} ROWS`);
    if (limit) out.push(`FETCH NEXT ${limit} ROWS ONLY`);
    return out.join("\n");
  }
  // Firebird: `ROWS inicio TO fim`.
  if (d === "firebird") {
    if (limit || offset) out.push(`ROWS ${Number(offset || 0) + 1} TO ${limit ? Number(offset || 0) + Number(limit) : 2147483647}`);
    return out.join("\n");
  }
  // Fora do PostgreSQL o deslocamento sozinho não vale: o MySQL e o SQLite pedem um LIMIT (o maior possível).
  if (limit) out.push(`LIMIT ${limit}`);
  else if (offset && d) out.push(`LIMIT ${d === "mysql" ? "18446744073709551615" : "-1"}`);
  if (offset) out.push(`OFFSET ${offset}`);
  return out.join("\n");
}

let counter = 0;
export const uid = (prefix: string) => `${prefix}${++counter}`;

/** Ligações automáticas entre uma tabela recém-lida e as que já estão no desenho (pelas chaves estrangeiras). */
export function foreignKeyJoins(added: QbTable, others: QbTable[], existing: QbJoin[]): QbJoin[] {
  const out: QbJoin[] = [];
  const has = (a: QbRef, b: QbRef) =>
    [...existing, ...out].some((j) => (same(j.left, a) && same(j.right, b)) || (same(j.left, b) && same(j.right, a)));
  const same = (a: QbRef, b: QbRef) => a.table === b.table && a.column === b.column;
  const link = (from: QbTable, to: QbTable) => {
    for (const fk of from.fks) {
      if (fk.refSchema !== to.schema || fk.refTable !== to.name) continue;
      fk.fields.forEach((f, i) => {
        const a = { table: from.id, column: f };
        const b = { table: to.id, column: fk.refFields[i] ?? fk.refFields[0] };
        if (!has(a, b)) out.push({ id: uid("j"), type: "INNER", left: a, right: b });
      });
    }
  };
  for (const o of others) {
    link(added, o);
    link(o, added);
  }
  return out;
}

/** Posição dos cartões de uma consulta, guardada pelo texto do SELECT (sem espaços extras nem o `;` final). */
export type QbLayout = Record<string, { x: number; y: number; w?: number; h?: number }>;

export function layoutKey(sql: string): string {
  const norm = sql.replace(/\s+/g, " ").replace(/\s*;\s*$/, "").trim();
  let h = 5381;
  for (let i = 0; i < norm.length; i++) h = ((h << 5) + h + norm.charCodeAt(i)) | 0;
  return `${norm.length}:${(h >>> 0).toString(36)}`;
}

/** Identifica um cartão entre aberturas do builder (o `id` muda a cada vez). */
export const layoutId = (t: Pick<QbTable, "schema" | "name" | "alias">) => `${t.schema}.${t.name}#${t.alias.trim()}`;

/** Chave pelo conjunto de tabelas do desenho: serve quando o texto do SELECT mudou mas as tabelas são as mesmas. */
export const tableSetKey = (tables: Pick<QbTable, "schema" | "name" | "alias">[]) => `tables:${tables.map(layoutId).sort().join("|")}`;
