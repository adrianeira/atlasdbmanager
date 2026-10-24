import type { OptionPair, TablespaceDetail } from "../backend";
import { quoteIdent } from "../model";

/** Estado editável de um tablespace (novo ou existente). */
export interface TablespaceState {
  name: string;
  owner: string;
  /** Pasta no servidor; só vale ao criar (o PostgreSQL não a troca depois). */
  location: string;
  options: OptionPair[];
  comment: string;
}

export const emptyTablespace = (): TablespaceState => ({ name: "", owner: "", location: "", options: [], comment: "" });

export function fromDetail(d: TablespaceDetail): TablespaceState {
  return { name: d.name, owner: d.owner, location: d.location, options: d.options.map((o) => ({ ...o })), comment: d.comment ?? "" };
}

export type TablespaceProblem = "name" | "reserved" | "location" | "optionKey";

/** Os tablespaces `pg_default` e `pg_global` e qualquer nome com prefixo `pg_` são do sistema. */
export const isSystemTablespace = (name: string) => name.toLowerCase().startsWith("pg_");

export function validateTablespace(s: TablespaceState, isNew: boolean): TablespaceProblem | null {
  if (!s.name.trim()) return "name";
  if (isNew && isSystemTablespace(s.name.trim())) return "reserved";
  if (isNew && !s.location.trim()) return "location";
  if (s.options.some((o) => !o.key.trim())) return "optionKey";
  return null;
}

const literal = (v: string) => `'${v.replace(/'/g, "''")}'`;
const NUMBER = /^-?\d+(\.\d+)?$/;
/** As opções de custo são números: vão sem aspas; qualquer outro texto vai como literal e o servidor diz se aceita. */
const optionValue = (v: string) => (NUMBER.test(v.trim()) ? v.trim() : literal(v));
const assignments = (options: OptionPair[]) => options.map((o) => `${o.key.trim()} = ${optionValue(o.value)}`).join(", ");

/** `CREATE TABLESPACE` com dono, pasta e opções, mais o comentário (também a prévia "Salvar como"). */
export function createTablespaceStatements(s: TablespaceState): string[] {
  const name = quoteIdent(s.name.trim());
  const head = [`CREATE TABLESPACE ${name}`];
  if (s.owner.trim()) head.push(`OWNER ${quoteIdent(s.owner.trim())}`);
  head.push(`LOCATION ${literal(s.location.trim())}`);
  if (s.options.length) head.push(`WITH (${assignments(s.options)})`);
  const out = [`${head.join("\n  ")};`];
  if (s.comment.trim()) out.push(`COMMENT ON TABLESPACE ${name} IS ${literal(s.comment)};`);
  return out;
}

/** Só o que mudou: renomear, dono, opções (`SET` e `RESET`) e comentário. */
export function alterTablespaceStatements(before: TablespaceState, after: TablespaceState): string[] {
  const out: string[] = [];
  let name = quoteIdent(before.name);
  if (after.name.trim() && after.name !== before.name) {
    out.push(`ALTER TABLESPACE ${name} RENAME TO ${quoteIdent(after.name.trim())};`);
    name = quoteIdent(after.name.trim());
  }
  if (after.owner.trim() && after.owner !== before.owner) out.push(`ALTER TABLESPACE ${name} OWNER TO ${quoteIdent(after.owner.trim())};`);
  const changed = after.options.filter((o) => before.options.find((b) => b.key === o.key)?.value !== o.value);
  if (changed.length) out.push(`ALTER TABLESPACE ${name} SET (${assignments(changed)});`);
  const removed = before.options.filter((b) => !after.options.some((o) => o.key === b.key));
  if (removed.length) out.push(`ALTER TABLESPACE ${name} RESET (${removed.map((o) => o.key.trim()).join(", ")});`);
  if (after.comment !== before.comment) out.push(`COMMENT ON TABLESPACE ${name} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}

export const dropTablespaceStatement = (name: string) => `DROP TABLESPACE ${quoteIdent(name)};`;
