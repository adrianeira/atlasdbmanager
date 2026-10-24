import type { Completion } from "@codemirror/autocomplete";
import type { SQLNamespace } from "@codemirror/lang-sql";
import type { ColumnDef, DbObject, SchemaMock } from "./model";

/** Sugestão do editor com o texto que aparece no canto direito da lista (o schema, ou a tabela de uma coluna). */
export type AtlasCompletion = Completion & { where?: string };

/** Tipo da sugestão = o ícone da lista (classe `cm-completionIcon-<tipo>`). */
export type CompletionKind = "table" | "view" | "matview" | "function" | "column";

// Tabelas primeiro, depois views, funções e (no fim, pelo padrão do editor) as palavras-chave.
const BOOST: Record<CompletionKind, number> = { column: 40, table: 30, view: 28, matview: 26, function: 10 };

const item = (label: string, type: CompletionKind, extra: { detail?: string; where?: string } = {}): AtlasCompletion =>
  ({ label, type, boost: BOOST[type], ...extra });

const columnsOf = (cols: ColumnDef[] | undefined, table: string): AtlasCompletion[] =>
  (cols ?? []).map((c) => item(c.name, "column", { detail: c.type, where: table }));

const relation = (o: DbObject, type: CompletionKind, schema: string): SQLNamespace => ({
  self: item(o.name, type, { where: schema }),
  children: columnsOf(o.columns, o.name),
});

/**
 * Nomes que o editor sugere para o schema escolhido: tabelas, views, views materializadas e funções, cada um com o seu ícone
 * e o schema no canto direito, e as colunas depois de `tabela.`. As palavras-chave e os tipos vêm do próprio editor.
 */
export function completionNamespace(schema: SchemaMock | undefined, extraTables?: DbObject[]): SQLNamespace {
  const out: Record<string, SQLNamespace> = {};
  const name = schema?.name ?? "";
  for (const o of extraTables ?? schema?.objects.table ?? []) out[o.name] = relation(o, "table", name);
  for (const o of schema?.objects.view ?? []) out[o.name] ??= relation(o, "view", name);
  for (const o of schema?.objects.matview ?? []) out[o.name] ??= relation(o, "matview", name);
  for (const f of schema?.objects.function ?? []) {
    const label = f.fnName ?? f.name.replace(/\(.*$/s, "");
    // Uma sugestão por nome (com sobrecargas vale a primeira assinatura); os argumentos aparecem ao lado do nome.
    out[label] ??= { self: item(label, "function", { detail: `(${f.args ?? ""})`, where: name }), children: [] };
  }
  return out;
}
