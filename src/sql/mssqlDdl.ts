import type { ColumnDef, DbObject } from "../model";

/**
 * DDL do SQL Server a partir do modelo do designer. Diferente do SQLite, o SQL Server altera a tabela no lugar (`ALTER TABLE`):
 * a exceção é o `IDENTITY`, que não se liga nem desliga numa coluna que já existe.
 */

export const MSSQL_TYPES = [
  "bigint", "binary", "bit", "char", "date", "datetime", "datetime2", "datetimeoffset", "decimal", "float", "int", "money", "nchar",
  "numeric", "nvarchar", "real", "smalldatetime", "smallint", "smallmoney", "time", "tinyint", "uniqueidentifier", "varbinary", "varchar", "xml",
];

/** Os tipos que aceitam `IDENTITY`. */
const IDENTITY_TYPES = new Set(["tinyint", "smallint", "int", "bigint"]);
export const isIdentityType = (type: string) => IDENTITY_TYPES.has(type.trim().toLowerCase());

/** Entre colchetes, como o SQL Server quer os nomes. */
export const q = (name: string) => `[${name.replace(/\]/g, "]]")}]`;
const lit = (s: string) => `N'${s.replace(/'/g, "''")}'`;

export function mssqlColumnType(c: ColumnDef): string {
  const t = c.type;
  if (["varchar", "nvarchar", "varbinary"].includes(t)) return c.length > 0 ? `${t}(${c.length})` : `${t}(max)`;
  if (["char", "nchar", "binary"].includes(t)) return `${t}(${c.length > 0 ? c.length : 1})`;
  if (t === "decimal" || t === "numeric") return `${t}(${c.length > 0 ? c.length : 18}, ${c.decimal > 0 ? c.decimal : 0})`;
  if (["datetime2", "time", "datetimeoffset"].includes(t) && c.length > 0) return `${t}(${c.length})`;
  if (t === "float" && c.length > 0) return `${t}(${c.length})`;
  return t;
}

const fieldList = (fields: string) => fields.split(",").map((f) => q(f.trim())).filter((f) => f !== "[]").join(", ");

/** Valor padrão entre parênteses (aceita literal, função ou expressão). */
const defaultExpr = (v: string) => (/^\(.*\)$/s.test(v.trim()) ? v.trim() : `(${v.trim()})`);

function columnLine(c: ColumnDef, table: string): string {
  let line = `${q(c.name)} ${mssqlColumnType(c)}`;
  if (c.autoInc && isIdentityType(c.type)) return `${line} IDENTITY(1,1) NOT NULL`;
  line += c.notNull || c.key ? " NOT NULL" : " NULL";
  if (c.default) line += ` CONSTRAINT ${q(`DF_${table}_${c.name}`)} DEFAULT ${defaultExpr(c.default)}`;
  return line;
}

function constraintLines(t: DbObject, name: string): string[] {
  const out: string[] = [];
  const keys = (t.columns ?? []).filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0));
  if (keys.length) out.push(`CONSTRAINT ${q(`PK_${name}`)} PRIMARY KEY (${keys.map((k) => q(k.name)).join(", ")})`);
  for (const u of t.uniques ?? []) if (fieldList(u.fields)) out.push(`${u.name ? `CONSTRAINT ${q(u.name)} ` : ""}UNIQUE (${fieldList(u.fields)})`);
  for (const c of t.checks ?? []) if (c.expression.trim()) out.push(`${c.name ? `CONSTRAINT ${q(c.name)} ` : ""}CHECK (${c.expression.trim()})`);
  for (const f of t.foreignKeys ?? []) if (fieldList(f.fields) && f.refTable) out.push(fkLine(f));
  return out;
}

function fkLine(f: NonNullable<DbObject["foreignKeys"]>[number]): string {
  const ref = `${f.refSchema ? `${q(f.refSchema)}.` : ""}${q(f.refTable)}`;
  const refs = f.refFields.trim() ? ` (${fieldList(f.refFields)})` : "";
  let line = `${f.name ? `CONSTRAINT ${q(f.name)} ` : ""}FOREIGN KEY (${fieldList(f.fields)}) REFERENCES ${ref}${refs}`;
  if (f.onDelete && f.onDelete !== "NO ACTION") line += ` ON DELETE ${f.onDelete}`;
  if (f.onUpdate && f.onUpdate !== "NO ACTION") line += ` ON UPDATE ${f.onUpdate}`;
  return line;
}

const indexSql = (target: string, i: { name: string; fields: string; unique: boolean }) =>
  `CREATE ${i.unique ? "UNIQUE " : ""}INDEX ${q(i.name)} ON ${target} (${fieldList(i.fields)})`;

export function createTableStatementsMssql(schema: string, table: DbObject, nameOverride?: string): string[] {
  const name = nameOverride ?? table.name;
  const target = `${q(schema)}.${q(name)}`;
  const lines = [...(table.columns ?? []).map((c) => columnLine(c, name)), ...constraintLines(table, name)];
  const out = [`CREATE TABLE ${target} (\n  ${lines.join(",\n  ")}\n)`];
  for (const i of table.indexes ?? []) if (i.name && fieldList(i.fields)) out.push(indexSql(target, i));
  return out;
}

export interface MssqlAlterPlan { sql: string; statements: string[]; destructive: boolean; notes: string[] }

const sameType = (a: ColumnDef, b: ColumnDef) => mssqlColumnType(a).toLowerCase() === mssqlColumnType(b).toLowerCase();
const same = (a?: string, b?: string) => (a ?? "").trim() === (b ?? "").trim();

/** Apaga a restrição de valor padrão da coluna, qualquer que seja o nome que o servidor lhe deu. */
const dropDefault = (target: string, schema: string, table: string, col: string) =>
  `DECLARE @n sysname = (SELECT dc.name FROM sys.default_constraints dc JOIN sys.columns c ON c.object_id = dc.parent_object_id AND c.column_id = dc.parent_column_id ` +
  `WHERE dc.parent_object_id = OBJECT_ID(${lit(`${q(schema)}.${q(table)}`)}) AND c.name = ${lit(col)}); IF @n IS NOT NULL EXEC(N'ALTER TABLE ' + ${lit(target)} + N' DROP CONSTRAINT ' + QUOTENAME(@n))`;

/**
 * Comandos para levar `original` ao que o designer montou em `next`. `origNames[i]` é o nome que a coluna `i` de `next` tinha ao
 * abrir (vazio = coluna nova). `notes` traz o que não pôde ser feito (ligar ou desligar `IDENTITY` numa coluna que já existe).
 */
export function alterStatementsMssql(schema: string, original: DbObject, next: DbObject, origNames: (string | undefined)[]): MssqlAlterPlan {
  const table = original.name;
  const target = `${q(schema)}.${q(table)}`;
  const oldCols = original.columns ?? [];
  const newCols = next.columns ?? [];
  const out: string[] = [];
  const notes: string[] = [];
  let destructive = false;
  const kept = new Set(origNames.filter(Boolean));

  // Chave primária: se mudou, cai antes das colunas mudarem e volta depois.
  const keyOf = (cols: ColumnDef[], names: (string | undefined)[]) => cols.map((c, i) => ({ c, n: names[i] ?? c.name })).filter((x) => x.c.key).sort((a, b) => (a.c.key ?? 0) - (b.c.key ?? 0)).map((x) => x.n);
  const oldKey = oldCols.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0)).map((c) => c.name);
  const newKeyOrig = keyOf(newCols, origNames);
  const newKeyNow = newCols.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0)).map((c) => c.name);
  const pkChanged = JSON.stringify(oldKey) !== JSON.stringify(newKeyOrig);
  const pkName = (original.indexes ?? []).find((i) => i.primary)?.name;
  if (pkChanged && oldKey.length) out.push(`ALTER TABLE ${target} DROP CONSTRAINT ${q(pkName ?? `PK_${table}`)}`);

  // Colunas que saem.
  for (const o of oldCols) {
    if (kept.has(o.name)) continue;
    out.push(dropDefault(target, schema, table, o.name), `ALTER TABLE ${target} DROP COLUMN ${q(o.name)}`);
    destructive = true;
  }
  newCols.forEach((c, i) => {
    const from = origNames[i];
    const o = oldCols.find((x) => x.name === from);
    if (!o) {
      out.push(`ALTER TABLE ${target} ADD ${columnLine(c, table)}`);
      return;
    }
    let name = o.name;
    if (o.name !== c.name) {
      out.push(`EXEC sp_rename ${lit(`${q(schema)}.${q(table)}.${q(o.name)}`)}, ${lit(c.name)}, 'COLUMN'`);
      name = c.name;
    }
    const wasIdentity = !!o.identity;
    if (wasIdentity !== !!c.autoInc) notes.push(`${c.name}: IDENTITY`);
    const changed = !sameType(o, c) || !!o.notNull !== !!c.notNull;
    if (changed && !(wasIdentity && c.autoInc && sameType(o, c))) {
      if (o.default) out.push(dropDefault(target, schema, table, name));
      out.push(`ALTER TABLE ${target} ALTER COLUMN ${q(name)} ${mssqlColumnType(c)} ${c.notNull || c.key ? "NOT NULL" : "NULL"}`);
      if (c.default) out.push(`ALTER TABLE ${target} ADD CONSTRAINT ${q(`DF_${table}_${name}`)} DEFAULT ${defaultExpr(c.default)} FOR ${q(name)}`);
      if (!sameType(o, c)) destructive = true;
    } else if (!same(o.default, c.default)) {
      out.push(dropDefault(target, schema, table, name));
      if (c.default) out.push(`ALTER TABLE ${target} ADD CONSTRAINT ${q(`DF_${table}_${name}`)} DEFAULT ${defaultExpr(c.default)} FOR ${q(name)}`);
    }
  });
  if (pkChanged && newKeyNow.length) out.push(`ALTER TABLE ${target} ADD CONSTRAINT ${q(`PK_${table}`)} PRIMARY KEY (${newKeyNow.map(q).join(", ")})`);

  // Restrições e índices, por nome.
  const byName = <T extends { name: string }>(l: T[] | undefined) => new Map((l ?? []).map((x) => [x.name, x]));
  const diff = <T extends { name: string }>(a: T[] | undefined, b: T[] | undefined, key: (x: T) => string, drop: (x: T) => string, add: (x: T) => string | null) => {
    const before = byName(a);
    const after = byName(b);
    for (const [n, x] of before) {
      const y = after.get(n);
      if (!y || key(x) !== key(y)) out.push(drop(x));
    }
    for (const [n, y] of after) {
      const x = before.get(n);
      if (!x || key(x) !== key(y)) {
        const sql = add(y);
        if (sql) out.push(sql);
      }
    }
  };
  const dropConstraint = (x: { name: string }) => `ALTER TABLE ${target} DROP CONSTRAINT ${q(x.name)}`;
  diff(original.foreignKeys, next.foreignKeys, (f) => `${f.fields}|${f.refSchema}|${f.refTable}|${f.refFields}|${f.onDelete}|${f.onUpdate}`, dropConstraint,
    (f) => (fieldList(f.fields) && f.refTable ? `ALTER TABLE ${target} ADD ${fkLine(f)}` : null));
  diff(original.uniques, next.uniques, (u) => u.fields.replace(/\s+/g, ""), dropConstraint, (u) => (fieldList(u.fields) ? `ALTER TABLE ${target} ADD ${u.name ? `CONSTRAINT ${q(u.name)} ` : ""}UNIQUE (${fieldList(u.fields)})` : null));
  diff(original.checks, next.checks, (c) => c.expression.trim(), dropConstraint, (c) => (c.expression.trim() ? `ALTER TABLE ${target} ADD ${c.name ? `CONSTRAINT ${q(c.name)} ` : ""}CHECK (${c.expression.trim()})` : null));
  // Os índices que sustentam a chave primária e as restrições UNIQUE aparecem como restrições, não como índices.
  const userIndexes = (original.indexes ?? []).filter((i) => !i.primary && !(original.uniques ?? []).some((u) => u.name === i.name));
  diff(userIndexes, next.indexes, (i) => `${i.fields.replace(/\s+/g, "")}|${i.unique}`, (i) => `DROP INDEX ${q(i.name)} ON ${target}`, (i) => (i.name && fieldList(i.fields) ? indexSql(target, i) : null));

  return { sql: out.map((s) => `${s};`).join("\n"), statements: out, destructive, notes };
}
