import { quoteIdent, type ColumnDef, type DbObject } from "../model";

/**
 * DDL do SQLite a partir do modelo do designer. O SQLite só altera por `ALTER TABLE` o básico (acrescentar e renomear coluna);
 * qualquer outra mudança na estrutura refaz a tabela: cria a nova, copia os dados, apaga a antiga e troca o nome.
 */

/** Tipos declarados mais comuns (o SQLite aceita qualquer nome; a afinidade vem do nome). */
export const SQLITE_TYPES = [
  "bigint", "blob", "boolean", "char", "date", "datetime", "decimal", "double", "float", "int", "integer", "json",
  "numeric", "real", "smallint", "text", "time", "timestamp", "tinyint", "varchar",
];

const SIZED = new Set(["varchar", "char"]);
const PRECISION = new Set(["decimal", "numeric"]);
const TEMP_TABLE = "__atlas_new";

/** Só a coluna `INTEGER PRIMARY KEY` vira apelido do rowid e numera sozinha. */
export const isRowidType = (type: string) => type.trim().toLowerCase() === "integer";

export function sqliteColumnType(c: ColumnDef): string {
  if (SIZED.has(c.type) && c.length > 0) return `${c.type}(${c.length})`;
  if (PRECISION.has(c.type) && c.length > 0) return `${c.type}(${c.length}, ${c.decimal})`;
  return c.type;
}

const fieldList = (fields: string) => fields.split(",").map((f) => quoteIdent(f.trim())).filter((f) => f !== '""').join(", ");

/** Valor padrão: literal, palavra reservada ou já entre parênteses vai como está; o resto é expressão e leva parênteses. */
export function sqliteDefault(v: string): string {
  const t = v.trim();
  if (/^(null|true|false|current_(date|time|timestamp))$/i.test(t) || /^[+-]?\d+(\.\d+)?$/.test(t) || /^'([^']|'')*'$/.test(t) || /^x'[0-9a-f]*'$/i.test(t) || /^\(.*\)$/s.test(t)) return t;
  return `(${t})`;
}

/** A coluna que numera sozinha: chave única, de uma coluna, do tipo `integer`, marcada como auto incremento. */
function autoIncColumn(cols: ColumnDef[]): ColumnDef | null {
  const keys = cols.filter((c) => c.key);
  return keys.length === 1 && keys[0].autoInc && isRowidType(keys[0].type) ? keys[0] : null;
}

function createOnly(table: DbObject, name: string): string {
  const cols = table.columns ?? [];
  const auto = autoIncColumn(cols);
  const keys = cols.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0));
  const lines: string[] = cols.map((c) => {
    if (c === auto) return `  ${quoteIdent(c.name)} INTEGER PRIMARY KEY AUTOINCREMENT`;
    let line = `  ${quoteIdent(c.name)} ${sqliteColumnType(c)}`;
    if (c.notNull) line += " NOT NULL";
    if (c.default) line += ` DEFAULT ${sqliteDefault(c.default)}`;
    return line;
  });
  if (keys.length && !auto) lines.push(`  PRIMARY KEY (${keys.map((k) => quoteIdent(k.name)).join(", ")})`);
  for (const u of table.uniques ?? []) {
    if (!fieldList(u.fields)) continue;
    lines.push(`  ${u.name ? `CONSTRAINT ${quoteIdent(u.name)} ` : ""}UNIQUE (${fieldList(u.fields)})`);
  }
  for (const c of table.checks ?? []) {
    if (!c.expression.trim()) continue;
    lines.push(`  ${c.name ? `CONSTRAINT ${quoteIdent(c.name)} ` : ""}CHECK (${c.expression.trim()})`);
  }
  for (const f of table.foreignKeys ?? []) {
    if (!fieldList(f.fields) || !f.refTable) continue;
    const refs = f.refFields.trim() ? ` (${fieldList(f.refFields)})` : "";
    let line = `  ${f.name ? `CONSTRAINT ${quoteIdent(f.name)} ` : ""}FOREIGN KEY (${fieldList(f.fields)}) REFERENCES ${quoteIdent(f.refTable)}${refs}`;
    if (f.onDelete && f.onDelete !== "NO ACTION") line += ` ON DELETE ${f.onDelete}`;
    if (f.onUpdate && f.onUpdate !== "NO ACTION") line += ` ON UPDATE ${f.onUpdate}`;
    lines.push(line);
  }
  return `CREATE TABLE ${quoteIdent(name)} (\n${lines.join(",\n")}\n)`;
}

/** Índices do designer (os automáticos de chave e de UNIQUE não entram: nascem com a tabela). */
function indexStatements(table: DbObject, tableName: string): string[] {
  return (table.indexes ?? [])
    .filter((i) => fieldList(i.fields) && i.name)
    .map((i) => `CREATE ${i.unique ? "UNIQUE " : ""}INDEX ${quoteIdent(i.name)} ON ${quoteIdent(tableName)} (${fieldList(i.fields)})`);
}

export function createTableStatementsSqlite(table: DbObject, nameOverride?: string): string[] {
  const name = nameOverride ?? table.name;
  return [createOnly(table, name), ...indexStatements(table, name)];
}

/** Criar a view (ou, ao alterar, apagar e criar de novo: o SQLite não tem `CREATE OR REPLACE VIEW`). */
export function viewStatementsSqlite(name: string, definition: string, replace: boolean): string[] {
  const create = `CREATE VIEW ${quoteIdent(name)} AS\n${definition.trim().replace(/;+\s*$/, "")}`;
  return replace ? [`DROP VIEW ${quoteIdent(name)}`, create] : [create];
}

export interface SqliteAlterPlan { sql: string; statements: string[]; destructive: boolean }

const sameType = (a: ColumnDef, b: ColumnDef) => sqliteColumnType(a).toLowerCase() === sqliteColumnType(b).toLowerCase();
const same = (a?: string, b?: string) => (a ?? "").trim() === (b ?? "").trim();

/**
 * Comandos para levar `original` ao que o designer montou em `next`. `origNames[i]` é o nome que a coluna `i` de `next` tinha
 * ao abrir (vazio = coluna nova). Colunas, chaves estrangeiras, UNIQUE e CHECK alterados refazem a tabela; índices e o nome
 * das colunas (sem outra mudança) e colunas novas simples saem por `ALTER`/`CREATE INDEX`.
 */
export function alterStatementsSqlite(original: DbObject, next: DbObject, origNames: (string | undefined)[]): SqliteAlterPlan {
  const table = original.name;
  const oldCols = original.columns ?? [];
  const newCols = next.columns ?? [];
  const kept = new Set(origNames.filter(Boolean));
  const dropped = oldCols.filter((c) => !kept.has(c.name));
  let rebuild = dropped.length > 0;
  const added: ColumnDef[] = [];
  const renames: [string, string][] = [];
  newCols.forEach((c, i) => {
    const from = origNames[i];
    const o = oldCols.find((x) => x.name === from);
    if (!o) {
      // Coluna nova: o ALTER aceita só a que não é chave, nem única, e (se obrigatória) tem valor padrão.
      if (c.key || c.autoInc || (c.notNull && !c.default) || (next.uniques ?? []).some((u) => fieldList(u.fields).split(", ").includes(quoteIdent(c.name)))) rebuild = true;
      else added.push(c);
      return;
    }
    if (o.name !== c.name) renames.push([o.name, c.name]);
    if (!sameType(o, c) || !!o.notNull !== !!c.notNull || !same(o.default, c.default) || (o.key ?? 0) !== (c.key ?? 0) || !!o.identity !== !!c.autoInc) rebuild = true;
  });
  // A ordem das colunas existentes também é da tabela: trocar de lugar refaz.
  const order = newCols.map((_, i) => origNames[i]).filter(Boolean).map((n) => oldCols.findIndex((c) => c.name === n));
  if (order.some((v, i) => i > 0 && v < order[i - 1])) rebuild = true;

  const norm = (list: { name: string; fields: string }[]) => JSON.stringify(list.map((x) => [x.name, x.fields.replace(/\s+/g, "")]));
  const oldUniques = original.uniques ?? [];
  if (norm(oldUniques) !== norm(next.uniques ?? [])) rebuild = true;
  const checks = (l: DbObject["checks"]) => JSON.stringify((l ?? []).map((c) => [c.name, c.expression.trim()]));
  if (checks(original.checks) !== checks(next.checks)) rebuild = true;
  const fks = (l: DbObject["foreignKeys"]) => JSON.stringify((l ?? []).map((f) => [f.name, f.fields, f.refTable, f.refFields, f.onDelete, f.onUpdate]));
  if (fks(original.foreignKeys) !== fks(next.foreignKeys)) rebuild = true;

  const out: string[] = [];
  // Índices que o designer lista (sem os automáticos).
  const userIndexes = (o: DbObject) => (o.indexes ?? []).filter((i) => !i.name.startsWith("sqlite_autoindex_"));
  const idxKey = (i: { name: string; fields: string; unique: boolean }) => `${i.name}|${i.fields.replace(/\s+/g, "")}|${i.unique}`;

  if (rebuild) {
    out.push("PRAGMA defer_foreign_keys = ON");
    out.push(createOnly(next, TEMP_TABLE));
    const pairs = newCols.map((c, i) => ({ c, from: origNames[i] })).filter((p) => p.from && oldCols.some((o) => o.name === p.from));
    if (pairs.length) {
      out.push(`INSERT INTO ${quoteIdent(TEMP_TABLE)} (${pairs.map((p) => quoteIdent(p.c.name)).join(", ")}) SELECT ${pairs.map((p) => quoteIdent(p.from as string)).join(", ")} FROM ${quoteIdent(table)}`);
    }
    out.push(`DROP TABLE ${quoteIdent(table)}`);
    // O RENAME comum recusa se uma view usa a tabela; o modo antigo só troca o nome.
    out.push("PRAGMA legacy_alter_table = ON", `ALTER TABLE ${quoteIdent(TEMP_TABLE)} RENAME TO ${quoteIdent(table)}`, "PRAGMA legacy_alter_table = OFF");
    out.push(...indexStatements(next, table));
    // Os gatilhos morrem com a tabela antiga: voltam com o mesmo texto.
    for (const g of original.triggers ?? []) if (g.sql) out.push(g.sql);
  } else {
    for (const [from, to] of renames) out.push(`ALTER TABLE ${quoteIdent(table)} RENAME COLUMN ${quoteIdent(from)} TO ${quoteIdent(to)}`);
    for (const c of added) {
      let line = `ALTER TABLE ${quoteIdent(table)} ADD COLUMN ${quoteIdent(c.name)} ${sqliteColumnType(c)}`;
      if (c.notNull) line += " NOT NULL";
      if (c.default) line += ` DEFAULT ${sqliteDefault(c.default)}`;
      out.push(line);
    }
    const before = new Map(userIndexes(original).map((i) => [idxKey(i), i]));
    const after = new Map(userIndexes(next).map((i) => [idxKey(i), i]));
    for (const [k, i] of before) if (!after.has(k)) out.push(`DROP INDEX ${quoteIdent(i.name)}`);
    for (const [k, i] of after) if (!before.has(k) && fieldList(i.fields) && i.name) out.push(`CREATE ${i.unique ? "UNIQUE " : ""}INDEX ${quoteIdent(i.name)} ON ${quoteIdent(table)} (${fieldList(i.fields)})`);
  }
  return { sql: out.map((s) => `${s};`).join("\n"), statements: out, destructive: dropped.length > 0 };
}
