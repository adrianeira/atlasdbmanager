import type { ExtensionDetail } from "../backend";
import { quoteIdent } from "../model";

/** Estado editável de uma extensão (nova ou instalada). */
export interface ExtState {
  name: string;
  /** Nova: vazio = a versão padrão. Instalada: a versão atual, ou a escolhida para atualizar. */
  version: string;
  schema: string;
  /** `CASCADE`: instala também as extensões de que esta depende (só ao criar). */
  cascade: boolean;
  comment: string;
}

export const emptyExt = (schema: string): ExtState => ({ name: "", version: "", schema, cascade: false, comment: "" });

export function fromDetail(d: ExtensionDetail): ExtState {
  return { name: d.name, version: d.version, schema: d.schema, cascade: false, comment: d.comment ?? "" };
}

export const validateExt = (s: ExtState): "name" | null => (s.name.trim() ? null : "name");

const literal = (s: string) => `'${s.replace(/'/g, "''")}'`;

/**
 * `CREATE EXTENSION` (também a prévia "Salvar como"). `fixedSchema`: o arquivo de controle da extensão já fixa o
 * schema, então o `SCHEMA` não vai. `CASCADE` só existe a partir do PostgreSQL 9.6.
 */
export function createExtStatements(s: ExtState, opts: { fixedSchema?: string | null; supportsCascade: boolean }): string[] {
  const name = quoteIdent(s.name.trim());
  const parts = [`CREATE EXTENSION ${name}`];
  if (!opts.fixedSchema && s.schema) parts.push(`SCHEMA ${quoteIdent(s.schema)}`);
  if (s.version.trim()) parts.push(`VERSION ${literal(s.version.trim())}`);
  if (s.cascade && opts.supportsCascade) parts.push("CASCADE");
  const out = [`${parts.join("\n  ")};`];
  if (s.comment.trim()) out.push(`COMMENT ON EXTENSION ${name} IS ${literal(s.comment)};`);
  return out;
}

/** Só o que mudou: atualizar a versão, mover de schema (extensão relocável) e o comentário. */
export function alterExtStatements(before: ExtState, after: ExtState): string[] {
  const name = quoteIdent(before.name);
  const out: string[] = [];
  if (after.version !== before.version) out.push(`ALTER EXTENSION ${name} UPDATE TO ${literal(after.version)};`);
  if (after.schema !== before.schema) out.push(`ALTER EXTENSION ${name} SET SCHEMA ${quoteIdent(after.schema)};`);
  if (after.comment !== before.comment) out.push(`COMMENT ON EXTENSION ${name} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}

export const dropExtSql = (name: string): string => `DROP EXTENSION ${quoteIdent(name)};`;

/** Ordena versões como `1.9`, `1.10`, `2.0` por número, não por texto. */
export const compareVersions = (a: string, b: string): number => a.localeCompare(b, undefined, { numeric: true });
