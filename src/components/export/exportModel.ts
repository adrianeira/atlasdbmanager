import type { ExportEncoding, ExportFormat, OrderBy, TableFilter } from "../../backend";
import type { MessageKey } from "../../i18n";
import type { SmallIconName } from "../../icons";

/** Formatos que o assistente sabe gravar. */
export type Kind = "txt" | "csv" | "json" | "xml" | "html" | "sql" | "xlsx" | "xls" | "dbf";

export interface FormatDef {
  kind: Kind;
  label: MessageKey;
  /** Extensões aceitas; a primeira é a sugerida. */
  ext: string[];
  icon: SmallIconName;
}

export const FORMATS: FormatDef[] = [
  { kind: "txt", label: "exp.kind.txt", ext: ["txt"], icon: "fmtTxt" },
  { kind: "csv", label: "exp.kind.csv", ext: ["csv"], icon: "fmtCsv" },
  { kind: "json", label: "exp.kind.json", ext: ["json"], icon: "fmtJson" },
  { kind: "xml", label: "exp.kind.xml", ext: ["xml"], icon: "fmtXml" },
  { kind: "html", label: "exp.kind.html", ext: ["html", "htm"], icon: "fmtHtml" },
  { kind: "sql", label: "exp.kind.sql", ext: ["sql"], icon: "fmtSql" },
  { kind: "xlsx", label: "exp.kind.xlsx", ext: ["xlsx"], icon: "fmtXlsx" },
  { kind: "xls", label: "exp.kind.xls", ext: ["xls"], icon: "fmtXlsx" },
  { kind: "dbf", label: "exp.kind.dbf", ext: ["dbf"], icon: "fmtTxt" },
];

/** Formatos da lista que ainda não existem: aparecem desabilitados. */
export const UNAVAILABLE_FORMATS: { label: MessageKey; icon: SmallIconName }[] = [
  { label: "exp.kind.accdb", icon: "fmtTxt" },
];

export const formatOf = (kind: Kind): FormatDef => FORMATS.find((f) => f.kind === kind) ?? FORMATS[0];
const MANAGED_EXTENSIONS = new Set(FORMATS.flatMap((f) => f.ext));

export interface DelimitedOptions {
  delimiter: string;
  qualifier: string;
  header: boolean;
  crlf: boolean;
  nullText: string;
  encoding: ExportEncoding;
}

export interface Options {
  txt: DelimitedOptions;
  csv: DelimitedOptions;
  json: { pretty: boolean };
  html: { header: boolean };
  /** `tableName`: nome da tabela no `INSERT` ao exportar uma consulta (vazio = o nome da consulta). */
  sql: { rowsPerInsert: number; tableName: string };
  xlsx: { header: boolean; sheetName: string };
  xls: { header: boolean; sheetName: string };
  /** `decimals`: casas dos campos numéricos; `textLength`: largura dos campos de texto (1 a 254). */
  dbf: { decimals: number; textLength: number };
}

export function defaultOptions(): Options {
  // Excel em português espera ";" e UTF-8 com BOM para abrir o CSV sem passar pela importação.
  const pt = typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("pt");
  return {
    txt: { delimiter: "\t", qualifier: "", header: true, crlf: true, nullText: "", encoding: "utf8" },
    csv: { delimiter: pt ? ";" : ",", qualifier: '"', header: true, crlf: true, nullText: "", encoding: pt ? "utf8Bom" : "utf8" },
    json: { pretty: true },
    html: { header: true },
    sql: { rowsPerInsert: 1, tableName: "" },
    xlsx: { header: true, sheetName: "" },
    xls: { header: true, sheetName: "" },
    dbf: { decimals: 4, textLength: 254 },
  };
}

/** Junta opções salvas (possivelmente de uma versão antiga) com os padrões. */
export function mergeOptions(saved: Partial<Options> | null | undefined): Options {
  const base = defaultOptions();
  if (!saved) return base;
  return {
    txt: { ...base.txt, ...saved.txt },
    csv: { ...base.csv, ...saved.csv },
    json: { ...base.json, ...saved.json },
    html: { ...base.html, ...saved.html },
    sql: { ...base.sql, ...saved.sql },
    xlsx: { ...base.xlsx, ...saved.xlsx },
    xls: { ...base.xls, ...saved.xls },
    dbf: { ...base.dbf, ...saved.dbf },
  };
}

/** Formato enviado ao Rust para uma tabela. */
export function buildFormat(kind: Kind, o: Options, table: string): ExportFormat {
  switch (kind) {
    case "txt": return { kind: "delimited", ...o.txt };
    case "csv": return { kind: "delimited", ...o.csv };
    case "json": return { kind: "json", ...o.json };
    case "xml": return { kind: "xml" };
    case "html": return { kind: "html", ...o.html };
    case "sql": return { kind: "sql", rowsPerInsert: Math.max(1, Math.min(1000, Math.floor(o.sql.rowsPerInsert) || 1)) };
    case "xlsx": return { kind: "xlsx", header: o.xlsx.header, sheetName: o.xlsx.sheetName.trim() || table };
    case "xls": return { kind: "xls", header: o.xls.header, sheetName: o.xls.sheetName.trim() || table };
    case "dbf": return {
      kind: "dbf",
      decimals: Math.max(0, Math.min(14, Math.floor(o.dbf.decimals) || 0)),
      textLength: Math.max(1, Math.min(254, Math.floor(o.dbf.textLength) || 254)),
    };
  }
}

/** Troca a extensão do caminho se ela for de outro formato do assistente; senão mantém. */
export function swapExtension(path: string, kind: Kind): string {
  const m = /\.([A-Za-z0-9]+)$/.exec(path);
  if (!m) return path;
  const ext = m[1].toLowerCase();
  const wanted = formatOf(kind).ext;
  if (!MANAGED_EXTENSIONS.has(ext) || wanted.includes(ext)) return path;
  return `${path.slice(0, -m[0].length)}.${wanted[0]}`;
}

/** Caminho padrão: `<pasta>/<tabela>.<extensão>`. */
export function defaultPath(dir: string, table: string, kind: Kind, sep: string): string {
  const file = `${table}.${formatOf(kind).ext[0]}`;
  if (!dir) return file;
  return dir.endsWith("/") || dir.endsWith("\\") ? dir + file : dir + sep + file;
}

/** Troca só a pasta de um caminho, mantendo o nome do arquivo. */
export function withFolder(path: string, dir: string, sep: string): string {
  const name = path.split(/[\\/]/).pop() ?? path;
  return dir.endsWith("/") || dir.endsWith("\\") ? dir + name : dir + sep + name;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** `mm:ss.cc`, como o cronômetro do assistente. */
export function formatClock(ms: number): string {
  const centis = Math.floor(ms / 10);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(Math.floor(centis / 6000))}:${pad(Math.floor(centis / 100) % 60)}.${pad(centis % 100)}`;
}

/** O que exportar: o schema e, quando vem do grid, o filtro, a ordem e as colunas visíveis de uma tabela. */
export interface ExportTarget {
  connId: string;
  database: string;
  schema: string;
  tables: { name: string; view: boolean; query?: boolean }[];
  /** Tabela marcada ao abrir o assistente. */
  initial?: string;
  /** Resultado de uma consulta do editor SQL: a origem é o texto da consulta (reexecutado em streaming). */
  query?: { sql: string; schema?: string; columns: string[]; name: string };
  scope?: { table: string; orderBy: OrderBy[]; filter: TableFilter | null; visibleColumns: string[]; allColumns: string[] };
}

/** Perfil salvo: formato, opções e, por tabela, o arquivo e os campos. */
export interface ExportProfile {
  name: string;
  kind: Kind;
  options: Options;
  items: { table: string; path: string; fields: string[] | null }[];
}

export const PROFILES_KEY = "export.profiles";
const STORAGE_KEY = "atlas.export.wizard";

/** Último formato e opções usados (só deste computador). */
export function loadLast(): { kind: Kind; options: Options } {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as { kind?: string; options?: Partial<Options> } | null;
    const kind = FORMATS.some((f) => f.kind === raw?.kind) ? (raw?.kind as Kind) : "csv";
    return { kind, options: mergeOptions(raw?.options) };
  } catch {
    return { kind: "csv", options: defaultOptions() };
  }
}

export function saveLast(kind: Kind, options: Options): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ kind, options }));
  } catch {
    /* armazenamento indisponível: só não lembra */
  }
}
