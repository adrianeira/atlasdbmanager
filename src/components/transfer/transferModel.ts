import type { SchemaObjects, TransferSpec, TransferTableOverride } from "../../backend";

export type Cat = "table" | "view" | "function" | "sequence" | "type";
export const CATS: Cat[] = ["table", "view", "function", "sequence", "type"];

/** Um objeto do schema de origem que pode ser marcado. */
export interface Item {
  cat: Cat;
  /** O que vai para o motor: o nome; nas funções, `nome(argumentos)`. */
  key: string;
  name: string;
  /** Texto na árvore (as funções mostram os argumentos). */
  label: string;
}

export const selKey = (i: Pick<Item, "cat" | "key">) => `${i.cat}:${i.key}`;

export function itemsOf(o: SchemaObjects): Record<Cat, Item[]> {
  const plain = (cat: Cat, list: { name: string }[]): Item[] => list.map((x) => ({ cat, key: x.name, name: x.name, label: x.name }));
  return {
    table: plain("table", o.tables),
    view: plain("view", [...o.views, ...o.matviews]),
    function: o.functions.map((f) => ({ cat: "function" as const, key: `${f.name}(${f.args ?? ""})`, name: f.name, label: `${f.name}(${f.args ?? ""})` })),
    sequence: plain("sequence", o.sequences),
    type: plain("type", o.types),
  };
}

/** Modo "Advanced" de uma tabela (campos, linhas e filtros). */
export interface TableAdv {
  mode: "auto" | "advanced";
  targetName: string;
  customFields: boolean;
  fields: { source: string; target: string }[];
  batchOn: boolean;
  batch: number;
  customSets: boolean;
  sets: string[];
}

export const defaultAdv = (table: string): TableAdv => ({
  mode: "auto", targetName: table, customFields: false, fields: [], batchOn: false, batch: 1000, customSets: false, sets: [""],
});

export interface TransferOptions {
  structure: boolean;
  data: boolean;
}

export interface TransferProfile {
  name: string;
  sourceConn: string;
  sourceDb: string;
  sourceSchema: string;
  target: { kind: "connection" | "file"; conn: string; db: string; schema: string; path: string };
  selected: string[];
  adv: Record<string, TableAdv>;
  dropBefore: boolean;
  continueOnError: boolean;
  options: TransferOptions;
}

export const PROFILES_KEY = "datatransfer.profiles";

export function buildSpec(a: {
  sourceSchema: string;
  targetSchema: string;
  items: Record<Cat, Item[]>;
  selected: Set<string>;
  adv: Record<string, TableAdv>;
  dropBefore: boolean;
  continueOnError: boolean;
  options: TransferOptions;
}): TransferSpec {
  const chosen = (cat: Cat) => a.items[cat].filter((i) => a.selected.has(selKey(i)));
  const overrides: TransferTableOverride[] = [];
  for (const t of chosen("table")) {
    const v = a.adv[t.name];
    if (!v || v.mode !== "advanced") continue;
    const sets = v.sets.map((s) => s.trim()).filter(Boolean);
    const fields = v.fields.filter((f) => f.source);
    overrides.push({
      table: t.name,
      targetName: v.targetName.trim() && v.targetName.trim() !== t.name ? v.targetName.trim() : undefined,
      fields: v.customFields && fields.length ? fields.map((f) => ({ source: f.source, target: f.target.trim() || f.source })) : undefined,
      rowsPerBatch: v.batchOn && v.batch > 0 ? v.batch : undefined,
      recordsets: v.customSets && sets.length ? sets : undefined,
    });
  }
  return {
    sourceSchema: a.sourceSchema,
    targetSchema: a.targetSchema,
    tables: chosen("table").map((i) => i.name),
    views: chosen("view").map((i) => i.name),
    functions: chosen("function").map((i) => i.key),
    sequences: chosen("sequence").map((i) => i.name),
    types: chosen("type").map((i) => i.name),
    overrides,
    dropBeforeCreate: a.dropBefore,
    continueOnError: a.continueOnError,
    structure: a.options.structure,
    data: a.options.data,
  };
}

/** `SELECT` da tabela de origem para um filtro (a "SQL Preview" do modo avançado). */
export function previewSql(schema: string, table: string, adv: TableAdv, columns: string[]): string {
  const q = (s: string) => `"${s.replace(/"/g, '""')}"`;
  const fields = adv.customFields && adv.fields.some((f) => f.source) ? adv.fields.filter((f) => f.source).map((f) => q(f.source)) : columns.map(q);
  const where = adv.customSets ? adv.sets.map((s) => s.trim()).find(Boolean) : undefined;
  return `SELECT ${fields.join(", ") || "*"}\nFROM ${q(schema)}.${q(table)}${where ? `\nWHERE ${where}` : ""};`;
}

export const formatClock = (ms: number) => {
  const centis = Math.floor(ms / 10);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(Math.floor(centis / 6000))}:${pad(Math.floor(centis / 100) % 60)}.${pad(centis % 100)}`;
};
