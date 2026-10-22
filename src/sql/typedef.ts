import type { TypeDetail } from "../backend";
import { quoteIdent } from "../model";

export type TypeKind = "enum" | "composite" | "domain" | "range";
export const CREATABLE_KINDS: TypeKind[] = ["enum", "composite", "domain"];

export interface EnumLabel {
  /** Rótulo que o servidor já tem (ausente = novo). */
  orig?: string;
  value: string;
}
export interface Attr {
  orig?: string;
  name: string;
  type: string;
}
export interface DomainCheck {
  orig?: string;
  /** Vazio = o servidor escolhe o nome. */
  name: string;
  /** Expressão dentro de `CHECK (...)`; o valor testado se chama `VALUE`. */
  expr: string;
}

/** Estado editável de um tipo definido pelo usuário. */
export interface TypeState {
  name: string;
  kind: TypeKind;
  owner: string;
  comment: string;
  labels: EnumLabel[];
  attrs: Attr[];
  baseType: string;
  default: string;
  notNull: boolean;
  checks: DomainCheck[];
  /** Faixa (só leitura): tipo dos elementos. */
  subtype: string;
}

export const emptyType = (kind: TypeKind = "enum"): TypeState => ({
  name: "", kind, owner: "", comment: "", labels: [], attrs: [], baseType: "", default: "", notNull: false, checks: [], subtype: "",
});

/** `CHECK ((VALUE > 0))` → `(VALUE > 0)` (o que fica entre os parênteses do CHECK). */
export function checkExpression(definition: string): string {
  const m = /^\s*CHECK\s*\((.*)\)\s*(NOT VALID)?\s*$/is.exec(definition);
  return m ? m[1].trim() : definition;
}

export function fromDetail(d: TypeDetail): TypeState {
  return {
    name: d.name,
    kind: d.kind,
    owner: d.owner ?? "",
    comment: d.comment ?? "",
    labels: d.labels.map((l) => ({ orig: l, value: l })),
    attrs: d.attributes.map((a) => ({ orig: a.name, name: a.name, type: a.dataType })),
    baseType: d.baseType ?? "",
    default: d.default ?? "",
    notNull: d.notNull,
    checks: d.checks.map((c) => ({ orig: c.name, name: c.name, expr: checkExpression(c.definition) })),
    subtype: d.subtype ?? "",
  };
}

const literal = (s: string) => `'${s.replace(/'/g, "''")}'`;
const target = (schema: string, name: string) => `${quoteIdent(schema)}.${quoteIdent(name)}`;
const kw = (k: TypeKind) => (k === "domain" ? "DOMAIN" : "TYPE");

/** Erro de validação (chave de mensagem) ou `null`. */
export function validateType(s: TypeState): "name" | "labels" | "attrs" | "domain" | null {
  if (!s.name.trim()) return "name";
  if (s.kind === "enum") {
    const values = s.labels.map((l) => l.value);
    if (values.some((v) => v === "") || new Set(values).size !== values.length) return "labels";
  } else if (s.kind === "composite") {
    const names = s.attrs.map((a) => a.name.trim());
    if (names.some((n) => n === "") || new Set(names).size !== names.length || s.attrs.some((a) => !a.type.trim())) return "attrs";
  } else if (s.kind === "domain") {
    if (!s.baseType.trim() || s.checks.some((c) => !c.expr.trim())) return "domain";
  }
  return null;
}

const checkClause = (c: DomainCheck) => `${c.name.trim() ? `CONSTRAINT ${quoteIdent(c.name.trim())} ` : ""}CHECK (${c.expr.trim()})`;

/** DDL de um tipo novo (também a prévia "Salvar como"). */
export function createTypeStatements(schema: string, s: TypeState): string[] {
  const t = target(schema, s.name.trim());
  const out: string[] = [];
  if (s.kind === "enum") out.push(`CREATE TYPE ${t} AS ENUM (${s.labels.map((l) => literal(l.value)).join(", ")});`);
  else if (s.kind === "composite") out.push(`CREATE TYPE ${t} AS (${s.attrs.map((a) => `${quoteIdent(a.name.trim())} ${a.type.trim()}`).join(", ")});`);
  else if (s.kind === "domain") {
    const parts = [`CREATE DOMAIN ${t} AS ${s.baseType.trim()}`];
    if (s.default.trim()) parts.push(`DEFAULT ${s.default.trim()}`);
    if (s.notNull) parts.push("NOT NULL");
    for (const c of s.checks) parts.push(checkClause(c));
    out.push(`${parts.join("\n  ")};`);
  }
  if (s.owner) out.push(`ALTER ${kw(s.kind)} ${t} OWNER TO ${quoteIdent(s.owner)};`);
  if (s.comment) out.push(`COMMENT ON ${kw(s.kind)} ${t} IS ${literal(s.comment)};`);
  return out;
}

/** `ALTER TYPE ... ADD VALUE` não roda em bloco de transação antes do PostgreSQL 12: a interface o executa à parte. */
export const isAddValue = (sql: string) => /^ALTER TYPE .* ADD VALUE /.test(sql);

/**
 * Só o que mudou. Enum: renomear (PostgreSQL 10+) e acrescentar rótulos (o servidor não remove nem reordena).
 * Composto: acrescentar, remover, renomear e trocar o tipo de atributos. Domínio: padrão, `NOT NULL` e restrições
 * `CHECK` (o tipo base não muda).
 */
export function alterTypeStatements(schema: string, before: TypeState, after: TypeState): string[] {
  const out: string[] = [];
  const k = kw(before.kind);
  let name = before.name;
  if (after.name.trim() !== before.name) {
    out.push(`ALTER ${k} ${target(schema, before.name)} RENAME TO ${quoteIdent(after.name.trim())};`);
    name = after.name.trim();
  }
  const t = target(schema, name);

  if (before.kind === "enum") {
    for (const l of after.labels) if (l.orig !== undefined && l.value !== l.orig) out.push(`ALTER TYPE ${t} RENAME VALUE ${literal(l.orig)} TO ${literal(l.value)};`);
    for (const l of after.labels) if (l.orig === undefined) out.push(`ALTER TYPE ${t} ADD VALUE ${literal(l.value)};`);
  } else if (before.kind === "composite") {
    const kept = new Set(after.attrs.filter((a) => a.orig !== undefined).map((a) => a.orig));
    for (const a of before.attrs) if (!kept.has(a.orig)) out.push(`ALTER TYPE ${t} DROP ATTRIBUTE ${quoteIdent(a.name)};`);
    for (const a of after.attrs) {
      const was = before.attrs.find((b) => b.orig === a.orig);
      if (a.orig !== undefined && was && a.name.trim() !== was.name) out.push(`ALTER TYPE ${t} RENAME ATTRIBUTE ${quoteIdent(was.name)} TO ${quoteIdent(a.name.trim())};`);
    }
    for (const a of after.attrs) {
      const was = before.attrs.find((b) => b.orig === a.orig);
      if (a.orig !== undefined && was && a.type.trim() !== was.type) out.push(`ALTER TYPE ${t} ALTER ATTRIBUTE ${quoteIdent(a.name.trim())} TYPE ${a.type.trim()};`);
    }
    for (const a of after.attrs) if (a.orig === undefined) out.push(`ALTER TYPE ${t} ADD ATTRIBUTE ${quoteIdent(a.name.trim())} ${a.type.trim()};`);
  } else if (before.kind === "domain") {
    if (after.default.trim() !== before.default.trim()) out.push(after.default.trim() ? `ALTER DOMAIN ${t} SET DEFAULT ${after.default.trim()};` : `ALTER DOMAIN ${t} DROP DEFAULT;`);
    if (after.notNull !== before.notNull) out.push(`ALTER DOMAIN ${t} ${after.notNull ? "SET" : "DROP"} NOT NULL;`);
    const changed = (c: DomainCheck) => {
      const was = before.checks.find((b) => b.orig === c.orig);
      return !was || was.expr.trim() !== c.expr.trim() || was.name !== c.name.trim();
    };
    const keep = new Set(after.checks.filter((c) => c.orig !== undefined && !changed(c)).map((c) => c.orig));
    for (const c of before.checks) if (!keep.has(c.orig)) out.push(`ALTER DOMAIN ${t} DROP CONSTRAINT ${quoteIdent(c.name)};`);
    for (const c of after.checks) if (c.orig === undefined || changed(c)) out.push(`ALTER DOMAIN ${t} ADD ${checkClause(c)};`);
  }

  if (after.owner && after.owner !== before.owner) out.push(`ALTER ${k} ${t} OWNER TO ${quoteIdent(after.owner)};`);
  if (after.comment !== before.comment) out.push(`COMMENT ON ${k} ${t} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}

export const dropTypeSql = (schema: string, name: string, kind: string): string =>
  `DROP ${kind === "domain" ? "DOMAIN" : "TYPE"} ${target(schema, name)};`;
