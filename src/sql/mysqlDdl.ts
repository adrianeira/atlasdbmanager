import type { ColumnDef, DbObject } from "../model";

/**
 * DDL do MySQL a partir do modelo do designer. O MySQL altera a tabela no lugar (`ALTER TABLE`), inclusive o `AUTO_INCREMENT`
 * de uma coluna que já existe (desde que ela seja chave). No MySQL o schema é o próprio banco.
 */

export const MYSQL_TYPES = [
  "bigint", "binary", "bit", "blob", "char", "date", "datetime", "decimal", "double", "float", "int", "json", "longblob", "longtext",
  "mediumblob", "mediumint", "mediumtext", "smallint", "text", "time", "timestamp", "tinyblob", "tinyint", "tinytext", "varbinary", "varchar", "year",
];

/** Os tipos que aceitam `AUTO_INCREMENT`. */
const AUTO_TYPES = new Set(["tinyint", "smallint", "mediumint", "int", "bigint"]);
export const isAutoType = (type: string) => AUTO_TYPES.has(type.trim().toLowerCase());

/** Entre crases, como o MySQL quer os nomes. */
export const q = (name: string) => `\`${name.replace(/`/g, "``")}\``;
const lit = (s: string) => `'${s.replace(/\\/g, "\\\\").replace(/'/g, "''")}'`;

export function mysqlColumnType(c: ColumnDef): string {
  const t = c.type;
  let base = t;
  if (t === "varchar" || t === "varbinary") base = `${t}(${c.length > 0 ? c.length : 255})`;
  else if (t === "char" || t === "binary") base = `${t}(${c.length > 0 ? c.length : 1})`;
  else if (t === "decimal") base = `${t}(${c.length > 0 ? c.length : 10}, ${c.decimal > 0 ? c.decimal : 0})`;
  else if (["datetime", "timestamp", "time"].includes(t) && c.length > 0) base = `${t}(${c.length})`;
  else if (t === "bit") base = `${t}(${c.length > 0 ? c.length : 1})`;
  // `UNSIGNED` vem do tipo que o servidor devolveu (`int unsigned`) e vale enquanto o tipo não mudar.
  const unsigned = /unsigned/i.test(c.castType ?? "") && /int|decimal|float|double/.test(t);
  return unsigned ? `${base} UNSIGNED` : base;
}

const fieldList = (fields: string) => fields.split(",").map((f) => q(f.trim())).filter((f) => f !== "``").join(", ");

/** Valor padrão: literal, função conhecida ou já entre parênteses vai como está; o resto é expressão e leva parênteses. */
const defaultExpr = (v: string) => {
  const t = v.trim();
  if (/^(null|true|false|current_timestamp(\(\d*\))?|now\(\))$/i.test(t) || /^[+-]?\d+(\.\d+)?$/.test(t) || /^'([^']|'')*'$/.test(t) || /^\(.*\)$/s.test(t)) return t;
  return `(${t})`;
};

function columnLine(c: ColumnDef): string {
  let line = `${q(c.name)} ${mysqlColumnType(c)}`;
  if (c.autoInc && isAutoType(c.type)) return `${line} NOT NULL AUTO_INCREMENT`;
  line += c.notNull || c.key ? " NOT NULL" : " NULL";
  if (c.default) line += ` DEFAULT ${defaultExpr(c.default)}`;
  return line;
}

function fkLine(db: string, f: NonNullable<DbObject["foreignKeys"]>[number]): string {
  const ref = `${q(f.refSchema || db)}.${q(f.refTable)}`;
  const refs = f.refFields.trim() ? ` (${fieldList(f.refFields)})` : "";
  let line = `${f.name ? `CONSTRAINT ${q(f.name)} ` : ""}FOREIGN KEY (${fieldList(f.fields)}) REFERENCES ${ref}${refs}`;
  if (f.onDelete && f.onDelete !== "NO ACTION") line += ` ON DELETE ${f.onDelete}`;
  if (f.onUpdate && f.onUpdate !== "NO ACTION") line += ` ON UPDATE ${f.onUpdate}`;
  return line;
}

const indexSql = (target: string, i: { name: string; fields: string; unique: boolean }) =>
  `CREATE ${i.unique ? "UNIQUE " : ""}INDEX ${q(i.name)} ON ${target} (${fieldList(i.fields)})`;

export function createTableStatementsMysql(database: string, table: DbObject, nameOverride?: string): string[] {
  const name = nameOverride ?? table.name;
  const target = `${q(database)}.${q(name)}`;
  const cols = table.columns ?? [];
  const lines = cols.map(columnLine);
  const keys = cols.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0));
  if (keys.length) lines.push(`PRIMARY KEY (${keys.map((k) => q(k.name)).join(", ")})`);
  for (const u of table.uniques ?? []) if (fieldList(u.fields)) lines.push(`${u.name ? `CONSTRAINT ${q(u.name)} ` : ""}UNIQUE (${fieldList(u.fields)})`);
  for (const c of table.checks ?? []) if (c.expression.trim()) lines.push(`${c.name ? `CONSTRAINT ${q(c.name)} ` : ""}CHECK (${c.expression.trim()})`);
  for (const f of table.foreignKeys ?? []) if (fieldList(f.fields) && f.refTable) lines.push(fkLine(database, f));
  const out = [`CREATE TABLE ${target} (\n  ${lines.join(",\n  ")}\n)`];
  for (const i of table.indexes ?? []) if (i.name && fieldList(i.fields)) out.push(indexSql(target, i));
  return out;
}

export interface MysqlAlterPlan { sql: string; statements: string[]; destructive: boolean }

const sameType = (a: ColumnDef, b: ColumnDef) => mysqlColumnType(a).toLowerCase() === mysqlColumnType(b).toLowerCase();
const same = (a?: string, b?: string) => (a ?? "").trim() === (b ?? "").trim();

/**
 * Comandos para levar `original` ao que o designer montou em `next`. `origNames[i]` é o nome que a coluna `i` de `next` tinha ao
 * abrir (vazio = coluna nova).
 */
export function alterStatementsMysql(database: string, original: DbObject, next: DbObject, origNames: (string | undefined)[]): MysqlAlterPlan {
  const table = original.name;
  const target = `${q(database)}.${q(table)}`;
  const oldCols = original.columns ?? [];
  const newCols = next.columns ?? [];
  const out: string[] = [];
  let destructive = false;
  const kept = new Set(origNames.filter(Boolean));

  // Chave primária: se mudou, cai antes das colunas mudarem e volta depois (uma coluna AUTO_INCREMENT precisa continuar sendo chave).
  const oldKey = oldCols.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0)).map((c) => c.name);
  const newKeyOrig = newCols.map((c, i) => ({ c, n: origNames[i] ?? c.name })).filter((x) => x.c.key).sort((a, b) => (a.c.key ?? 0) - (b.c.key ?? 0)).map((x) => x.n);
  const newKeyNow = newCols.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0)).map((c) => c.name);
  const pkChanged = JSON.stringify(oldKey) !== JSON.stringify(newKeyOrig);
  // Antes de apagar a chave, a coluna que numera sozinha perde o AUTO_INCREMENT (o MySQL não deixa a chave sair com ele).
  if (pkChanged && oldKey.length) {
    for (const o of oldCols.filter((c) => c.identity && c.key)) out.push(`ALTER TABLE ${target} MODIFY COLUMN ${q(o.name)} ${mysqlColumnType(o)} NOT NULL`);
    out.push(`ALTER TABLE ${target} DROP PRIMARY KEY`);
  }
  for (const o of oldCols) {
    if (kept.has(o.name)) continue;
    out.push(`ALTER TABLE ${target} DROP COLUMN ${q(o.name)}`);
    destructive = true;
  }
  newCols.forEach((c, i) => {
    const from = origNames[i];
    const o = oldCols.find((x) => x.name === from);
    if (!o) {
      out.push(`ALTER TABLE ${target} ADD COLUMN ${columnLine(c)}`);
      return;
    }
    if (o.name !== c.name) out.push(`ALTER TABLE ${target} RENAME COLUMN ${q(o.name)} TO ${q(c.name)}`);
    const changed = !sameType(o, c) || !!o.notNull !== !!c.notNull || !same(o.default, c.default) || !!o.identity !== !!c.autoInc;
    if (changed) {
      // O novo `AUTO_INCREMENT` só vale com a chave já criada: quem liga numa coluna que ainda não é chave espera a chave (abaixo).
      out.push(`ALTER TABLE ${target} MODIFY COLUMN ${columnLine(c.autoInc && !c.key ? { ...c, autoInc: false } : c)}`);
      if (!sameType(o, c)) destructive = true;
    }
  });
  if (pkChanged && newKeyNow.length) out.push(`ALTER TABLE ${target} ADD PRIMARY KEY (${newKeyNow.map(q).join(", ")})`);
  // `AUTO_INCREMENT` ligado numa coluna que acabou de virar chave.
  newCols.forEach((c, i) => {
    const o = oldCols.find((x) => x.name === origNames[i]);
    if (o && c.autoInc && !c.key) return;
    if (o && c.autoInc && c.key && !o.key && !o.identity && pkChanged) out.push(`ALTER TABLE ${target} MODIFY COLUMN ${columnLine(c)}`);
  });

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
  diff(original.foreignKeys, next.foreignKeys, (f) => `${f.fields}|${f.refSchema}|${f.refTable}|${f.refFields}|${f.onDelete}|${f.onUpdate}`, (f) => `ALTER TABLE ${target} DROP FOREIGN KEY ${q(f.name)}`,
    (f) => (fieldList(f.fields) && f.refTable ? `ALTER TABLE ${target} ADD ${fkLine(database, f)}` : null));
  diff(original.uniques, next.uniques, (u) => u.fields.replace(/\s+/g, ""), (u) => `DROP INDEX ${q(u.name)} ON ${target}`,
    (u) => (fieldList(u.fields) ? `ALTER TABLE ${target} ADD ${u.name ? `CONSTRAINT ${q(u.name)} ` : ""}UNIQUE (${fieldList(u.fields)})` : null));
  diff(original.checks, next.checks, (c) => c.expression.trim(), (c) => `ALTER TABLE ${target} DROP CHECK ${q(c.name)}`,
    (c) => (c.expression.trim() ? `ALTER TABLE ${target} ADD ${c.name ? `CONSTRAINT ${q(c.name)} ` : ""}CHECK (${c.expression.trim()})` : null));
  // Os índices únicos aparecem como restrições UNIQUE, não como índices.
  const userIndexes = (original.indexes ?? []).filter((i) => !i.primary && !(original.uniques ?? []).some((u) => u.name === i.name));
  diff(userIndexes, next.indexes, (i) => `${i.fields.replace(/\s+/g, "")}|${i.unique}`, (i) => `DROP INDEX ${q(i.name)} ON ${target}`, (i) => (i.name && fieldList(i.fields) ? indexSql(target, i) : null));

  return { sql: out.map((s) => `${s};`).join("\n"), statements: out, destructive };
}

/** Criar a view; ao alterar, `CREATE OR REPLACE VIEW`. */
export function viewStatementsMysql(database: string, name: string, definition: string, replace: boolean): string[] {
  return [`${replace ? "CREATE OR REPLACE" : "CREATE"} VIEW ${q(database)}.${q(name)} AS\n${definition.trim().replace(/;+\s*$/, "")}`];
}

/** Renomear tabela ou view. */
export const renameStatementMysql = (database: string, name: string, newName: string) => `RENAME TABLE ${q(database)}.${q(name)} TO ${q(database)}.${q(newName)}`;

export { lit as mysqlLiteral };
