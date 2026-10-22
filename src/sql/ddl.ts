import { quoteIdent, type ColumnDef, type DbObject, type TriggerDef } from "../model";

/**
 * Pré-visualização de DDL PostgreSQL gerada a partir do modelo do designer.
 * Prévia apenas: no M1 a geração passa para o provedor Rust (ISchemaScripter),
 * que também valida versão do servidor e política de transação.
 */

const SIZED = new Set(["varchar", "bpchar", "char", "bit", "varbit"]);
const PRECISION = new Set(["numeric", "decimal"]);
const TIME = new Set(["timestamp", "timestamptz", "time", "timetz", "interval"]);

export function columnType(c: ColumnDef): string {
  if (SIZED.has(c.type) && c.length > 0) return `${c.type}(${c.length})`;
  if (PRECISION.has(c.type) && c.length > 0) return `${c.type}(${c.length}, ${c.decimal})`;
  if (TIME.has(c.type) && c.length > 0) return `${c.type}(${c.length})`;
  return c.type;
}

function fieldList(fields: string): string {
  return fields.split(",").map((f) => quoteIdent(f.trim())).filter((f) => f !== '""').join(", ");
}

export function createTableSql(schema: string, table: DbObject, nameOverride?: string): string {
  return createTableStatements(schema, table, nameOverride).join("\n\n");
}

/** Os mesmos comandos de `createTableSql`, separados para execução um a um na transação. */
export function createTableStatements(schema: string, table: DbObject, nameOverride?: string): string[] {
  const name = `${quoteIdent(schema)}.${quoteIdent(nameOverride ?? table.name)}`;
  const cols = table.columns ?? [];
  const lines: string[] = cols.map((c) => {
    let line = `  ${quoteIdent(c.name)} ${columnType(c)}`;
    if (c.collation) line += ` COLLATE ${c.collation}`;
    if (c.notNull) line += " NOT NULL";
    if (c.default) line += ` DEFAULT ${c.default}`;
    return line;
  });

  const pk = cols.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0));
  if (pk.length) lines.push(`  PRIMARY KEY (${pk.map((c) => quoteIdent(c.name)).join(", ")})`);
  for (const u of table.uniques ?? []) lines.push(`  CONSTRAINT ${quoteIdent(u.name)} UNIQUE (${fieldList(u.fields)})`);
  for (const ch of table.checks ?? []) lines.push(`  CONSTRAINT ${quoteIdent(ch.name)} CHECK (${ch.expression})${ch.noInherit ? " NO INHERIT" : ""}`);
  for (const fk of table.foreignKeys ?? []) {
    lines.push(
      `  CONSTRAINT ${quoteIdent(fk.name)} FOREIGN KEY (${fieldList(fk.fields)}) REFERENCES ${quoteIdent(fk.refSchema)}.${quoteIdent(fk.refTable)} (${fieldList(fk.refFields)}) ON DELETE ${fk.onDelete} ON UPDATE ${fk.onUpdate}`,
    );
  }

  const out = [`CREATE TABLE ${name} (\n${lines.join(",\n")}\n)\n;`];

  for (const ix of table.indexes ?? []) {
    out.push(`CREATE ${ix.unique ? "UNIQUE " : ""}INDEX ${ix.concurrently ? "CONCURRENTLY " : ""}${quoteIdent(ix.name)} ON ${name} USING ${ix.method} (\n  ${fieldList(ix.fields)}\n);`);
  }
  for (const tg of table.triggers ?? []) out.push(triggerSql(name, tg), ...triggerExtras(name, tg));
  if (table.owner) out.push(`ALTER TABLE ${name} \n  OWNER TO ${quoteIdent(table.owner)};`);
  for (const c of cols) {
    if (c.comment) out.push(`COMMENT ON COLUMN ${name}.${quoteIdent(c.name)} IS '${c.comment.replace(/'/g, "''")}';`);
  }
  if (table.comment) out.push(`COMMENT ON TABLE ${name} IS '${table.comment.replace(/'/g, "''")}';`);
  return out;
}

/** `CREATE TRIGGER` de um gatilho (momento, eventos com `UPDATE OF`, `WHEN`, função e argumentos). */
export function triggerSql(target: string, tg: TriggerDef): string {
  const events = [
    tg.insert && "INSERT",
    tg.update && (tg.updateOf?.trim() ? `UPDATE OF ${fieldList(tg.updateOf)}` : "UPDATE"),
    tg.delete && "DELETE",
    tg.truncate && "TRUNCATE",
  ].filter(Boolean).join(" OR ");
  const when = tg.when?.trim() ? `\nWHEN (${tg.when.trim()})` : "";
  return `CREATE TRIGGER ${quoteIdent(tg.name)} ${tg.fires} ${events} ON ${target}\nFOR EACH ${tg.forEach}${when}\nEXECUTE PROCEDURE ${tg.fn}(${tg.args?.trim() ?? ""});`;
}

/** Comandos que acompanham o `CREATE TRIGGER`: desabilitar (quando for o caso) e comentar. */
export function triggerExtras(target: string, tg: TriggerDef): string[] {
  const out: string[] = [];
  if (!tg.enable) out.push(`ALTER TABLE ${target} DISABLE TRIGGER ${quoteIdent(tg.name)};`);
  if (tg.comment) out.push(`COMMENT ON TRIGGER ${quoteIdent(tg.name)} ON ${target} IS '${tg.comment.replace(/'/g, "''")}';`);
  return out;
}

export function selectSql(schema: string, table: string, limit: number, offset: number): string {
  return `SELECT * FROM ${quoteIdent(schema)}.${quoteIdent(table)} LIMIT ${limit} OFFSET ${offset}`;
}
