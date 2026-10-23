import type { DatabaseDetail, SchemaDetail } from "../backend";
import { quoteIdent } from "../model";

const literal = (s: string) => `'${s.replace(/'/g, "''")}'`;
/** `COMMENT ON ... IS` com texto vazio remove o comentário (NULL). */
const commentValue = (s: string) => (s === "" ? "NULL" : literal(s));

/** Estado editável de um banco de dados. */
export interface DatabaseEditState {
  name: string;
  owner: string;
  /** -1 = sem limite. */
  connLimit: string;
  allowConnections: boolean;
  isTemplate: boolean;
  comment: string;
}

export function databaseState(d: DatabaseDetail): DatabaseEditState {
  return {
    name: d.name, owner: d.owner, connLimit: String(d.connectionLimit), allowConnections: d.allowConnections,
    isTemplate: d.isTemplate, comment: d.comment ?? "",
  };
}

export function validateDatabase(s: DatabaseEditState): "name" | "owner" | "limit" | null {
  if (!s.name.trim()) return "name";
  if (!s.owner.trim()) return "owner";
  if (!/^-?\d+$/.test(s.connLimit.trim()) || Number(s.connLimit) < -1) return "limit";
  return null;
}

/**
 * Comandos que levam o banco do estado `before` ao `after`. Os `ALTER`/`COMMENT` usam o nome antigo, então
 * o `RENAME TO` fica por último. Vazio = nada mudou.
 */
export function databaseStatements(before: DatabaseDetail, after: DatabaseEditState): string[] {
  const db = quoteIdent(before.name);
  const out: string[] = [];
  if (after.owner !== before.owner) out.push(`ALTER DATABASE ${db} OWNER TO ${quoteIdent(after.owner)};`);
  const options: string[] = [];
  if (Number(after.connLimit) !== before.connectionLimit) options.push(`CONNECTION LIMIT ${Number(after.connLimit)}`);
  if (after.allowConnections !== before.allowConnections) options.push(`ALLOW_CONNECTIONS ${after.allowConnections}`);
  if (after.isTemplate !== before.isTemplate) options.push(`IS_TEMPLATE ${after.isTemplate}`);
  if (options.length) out.push(`ALTER DATABASE ${db} WITH ${options.join(" ")};`);
  if (after.comment !== (before.comment ?? "")) out.push(`COMMENT ON DATABASE ${db} IS ${commentValue(after.comment)};`);
  const newName = after.name.trim();
  if (newName !== before.name) out.push(`ALTER DATABASE ${db} RENAME TO ${quoteIdent(newName)};`);
  return out;
}

/** Estado editável de um schema. */
export interface SchemaEditState {
  name: string;
  owner: string;
  comment: string;
}

export function schemaState(d: SchemaDetail): SchemaEditState {
  return { name: d.name, owner: d.owner, comment: d.comment ?? "" };
}

export function validateSchema(s: SchemaEditState): "name" | "owner" | null {
  if (!s.name.trim()) return "name";
  if (!s.owner.trim()) return "owner";
  return null;
}

/** Mesmo critério do banco: `RENAME TO` por último. */
export function schemaStatements(before: SchemaDetail, after: SchemaEditState): string[] {
  const sc = quoteIdent(before.name);
  const out: string[] = [];
  if (after.owner !== before.owner) out.push(`ALTER SCHEMA ${sc} OWNER TO ${quoteIdent(after.owner)};`);
  if (after.comment !== (before.comment ?? "")) out.push(`COMMENT ON SCHEMA ${sc} IS ${commentValue(after.comment)};`);
  const newName = after.name.trim();
  if (newName !== before.name) out.push(`ALTER SCHEMA ${sc} RENAME TO ${quoteIdent(newName)};`);
  return out;
}
