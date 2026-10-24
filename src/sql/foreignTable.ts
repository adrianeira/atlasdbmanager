import type { ForeignTableDetail, OptionPair, TableDetail } from "../backend";
import { quoteIdent } from "../model";
import { optionChanges } from "./foreignServer";

export interface FtColumn {
  name: string;
  type: string;
  notNull: boolean;
  /** Nome que a coluna tem no banco; vazio = coluna nova. */
  orig: string;
  origType: string;
  origNotNull: boolean;
}

/** Estado editável de uma tabela externa (nova ou existente). */
export interface ForeignTableState {
  name: string;
  server: string;
  owner: string;
  columns: FtColumn[];
  options: OptionPair[];
  comment: string;
}

export const emptyForeignTable = (): ForeignTableState => ({ name: "", server: "", owner: "", columns: [], options: [], comment: "" });

export const newColumn = (): FtColumn => ({ name: "", type: "text", notNull: false, orig: "", origType: "", origNotNull: false });

export function fromDetail(d: ForeignTableDetail, t: TableDetail): ForeignTableState {
  return {
    name: d.name, server: d.server, owner: d.owner ?? "",
    columns: t.columns.map((c) => ({ name: c.name, type: c.formattedType, notNull: c.notNull, orig: c.name, origType: c.formattedType, origNotNull: c.notNull })),
    options: d.options.map((o) => ({ ...o })), comment: d.comment ?? "",
  };
}

export type ForeignTableProblem = "name" | "server" | "noColumns" | "columnName" | "columnType" | "columnDuplicate" | "optionKey";

export function validateForeignTable(s: ForeignTableState): ForeignTableProblem | null {
  if (!s.name.trim()) return "name";
  if (!s.server.trim()) return "server";
  if (!s.columns.length) return "noColumns";
  if (s.columns.some((c) => !c.name.trim())) return "columnName";
  if (s.columns.some((c) => !c.type.trim())) return "columnType";
  const names = s.columns.map((c) => c.name.trim());
  if (new Set(names).size !== names.length) return "columnDuplicate";
  if (s.options.some((o) => !o.key.trim())) return "optionKey";
  return null;
}

const literal = (v: string) => `'${v.replace(/'/g, "''")}'`;
const optionList = (options: OptionPair[]) => options.map((o) => `${quoteIdent(o.key.trim())} ${literal(o.value)}`).join(", ");
const target = (schema: string, name: string) => `${quoteIdent(schema)}.${quoteIdent(name.trim())}`;

/** `CREATE FOREIGN TABLE` com as colunas, o servidor e as opções (também a prévia "Salvar como"). */
export function createForeignTableStatements(schema: string, s: ForeignTableState): string[] {
  const cols = s.columns.map((c) => `  ${quoteIdent(c.name.trim())} ${c.type.trim()}${c.notNull ? " NOT NULL" : ""}`).join(",\n");
  const head = [`CREATE FOREIGN TABLE ${target(schema, s.name)} (\n${cols}\n)`, `  SERVER ${quoteIdent(s.server.trim())}`];
  if (s.options.length) head.push(`  OPTIONS (${optionList(s.options)})`);
  const out = [`${head.join("\n")};`];
  if (s.owner.trim()) out.push(`ALTER FOREIGN TABLE ${target(schema, s.name)} OWNER TO ${quoteIdent(s.owner.trim())};`);
  if (s.comment.trim()) out.push(`COMMENT ON FOREIGN TABLE ${target(schema, s.name)} IS ${literal(s.comment)};`);
  return out;
}

/** Só o que mudou: renomear a tabela, depois colunas (remover, renomear, tipo, obrigatoriedade, incluir), opções, dono e comentário. */
export function alterForeignTableStatements(schema: string, before: ForeignTableState, after: ForeignTableState): string[] {
  const out: string[] = [];
  let tbl = target(schema, before.name);
  if (after.name.trim() && after.name !== before.name) {
    out.push(`ALTER FOREIGN TABLE ${tbl} RENAME TO ${quoteIdent(after.name.trim())};`);
    tbl = target(schema, after.name);
  }
  const alter = (action: string) => out.push(`ALTER FOREIGN TABLE ${tbl} ${action};`);
  for (const c of before.columns) if (!after.columns.some((a) => a.orig === c.orig)) alter(`DROP COLUMN ${quoteIdent(c.orig)}`);
  for (const c of after.columns) {
    if (!c.orig) continue;
    const name = quoteIdent(c.name.trim());
    if (c.name.trim() !== c.orig) alter(`RENAME COLUMN ${quoteIdent(c.orig)} TO ${name}`);
    if (c.type.trim() !== c.origType) alter(`ALTER COLUMN ${name} TYPE ${c.type.trim()}`);
    if (c.notNull !== c.origNotNull) alter(`ALTER COLUMN ${name} ${c.notNull ? "SET" : "DROP"} NOT NULL`);
  }
  for (const c of after.columns) if (!c.orig) alter(`ADD COLUMN ${quoteIdent(c.name.trim())} ${c.type.trim()}${c.notNull ? " NOT NULL" : ""}`);
  const opts = optionChanges(before.options, after.options);
  if (opts.length) alter(`OPTIONS (${opts.join(", ")})`);
  if (after.owner.trim() && after.owner !== before.owner) alter(`OWNER TO ${quoteIdent(after.owner.trim())}`);
  if (after.comment !== before.comment) out.push(`COMMENT ON FOREIGN TABLE ${tbl} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}
