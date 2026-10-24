import type { ImportFormat, ImportMode } from "../../backend";
import type { MessageKey } from "../../i18n";

/** Formatos que o assistente sabe ler. */
export type Kind = "txt" | "csv" | "json" | "xml" | "excel" | "dbf";

export interface FormatDef {
  kind: Kind;
  label: MessageKey;
  /** Extensões aceitas na janela "Adicionar arquivo(s)". */
  ext: string[];
}

export const FORMATS: FormatDef[] = [
  { kind: "txt", label: "imp.kind.txt", ext: ["txt", "text", "tsv"] },
  { kind: "csv", label: "imp.kind.csv", ext: ["csv"] },
  { kind: "json", label: "imp.kind.json", ext: ["json"] },
  { kind: "xml", label: "imp.kind.xml", ext: ["xml"] },
  { kind: "excel", label: "imp.kind.excel", ext: ["xls", "xlsx"] },
];

/** Na lista, mas ainda sem leitor: aparecem desabilitados. */
export const UNAVAILABLE_FORMATS: MessageKey[] = ["imp.kind.access", "imp.kind.odbc", "imp.kind.dbf.na"];

export const DBF_FORMAT: FormatDef = { kind: "dbf", label: "imp.kind.dbf", ext: ["dbf"] };
export const ALL_FORMATS: FormatDef[] = [...FORMATS, DBF_FORMAT];

export const formatOf = (kind: Kind): FormatDef => ALL_FORMATS.find((f) => f.kind === kind) ?? FORMATS[0];

/** Opções do arquivo de origem e dos formatos de data/número (as etapas "Opções da origem" e "Modo"). */
export interface SourceSettings {
  fieldNameRowOn: boolean;
  fieldNameRow: number;
  firstDataRow: number;
  /** 0 = até o fim. */
  lastDataRow: number;
  dateOrder: "DMY" | "MDY" | "YMD";
  dateTimeOrder: "DateTime" | "TimeDate";
  dateDelimiter: string;
  timeDelimiter: string;
  decimalSymbol: string;
  /** Só texto delimitado. */
  delimiter: string;
  qualifier: string;
  encoding: "utf8" | "windows1252";
}

export interface Advanced {
  extendedInsert: boolean;
  limitStatementSize: boolean;
  maxStatementKb: number;
  emptyAsNull: boolean;
  continueOnError: boolean;
}

export const defaultAdvanced = (): Advanced => ({ extendedInsert: true, limitStatementSize: true, maxStatementKb: 1024, emptyAsNull: false, continueOnError: true });

export function defaultSettings(kind: Kind): SourceSettings {
  const pt = typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("pt");
  return {
    fieldNameRowOn: true,
    fieldNameRow: 1,
    firstDataRow: 2,
    lastDataRow: 0,
    dateOrder: "DMY",
    dateTimeOrder: "DateTime",
    dateDelimiter: "/",
    timeDelimiter: ":",
    decimalSymbol: ".",
    delimiter: kind === "txt" ? "\t" : pt ? ";" : ",",
    qualifier: kind === "txt" ? "" : '"',
    encoding: "utf8",
  };
}

/** Formato enviado ao Rust para um arquivo. */
export function buildFormat(kind: Kind, s: SourceSettings): ImportFormat {
  switch (kind) {
    case "txt":
    case "csv": return { kind: "delimited", delimiter: s.delimiter, qualifier: s.qualifier, encoding: s.encoding };
    case "json": return { kind: "json" };
    case "xml": return { kind: "xml" };
    case "excel": return { kind: "excel" };
    case "dbf": return { kind: "dbf" };
  }
}

/** Linhas de cabeçalho/dados da origem (formatos sem grade ignoram). */
export const sourceOptions = (s: SourceSettings) => ({
  fieldNameRow: s.fieldNameRowOn ? Math.max(1, s.fieldNameRow) : 0,
  firstDataRow: Math.max(1, s.firstDataRow),
  lastDataRow: Math.max(0, s.lastDataRow),
});

export const convertOptions = (s: SourceSettings) => ({
  dateOrder: s.dateOrder,
  dateTimeOrder: s.dateTimeOrder,
  dateDelimiter: s.dateDelimiter,
  timeDelimiter: s.timeDelimiter,
  decimalSymbol: s.decimalSymbol,
});

export const hasGrid = (kind: Kind) => kind === "txt" || kind === "csv" || kind === "excel";
export const isText = (kind: Kind) => kind === "txt" || kind === "csv";

export interface MappingRow {
  source: string;
  /** Campo do destino ("" = não importa). Em tabela nova é o nome da coluna a criar. */
  target: string;
  /** Tipo da coluna nova (só tabela nova). */
  type: string;
}

/** Uma tabela do arquivo a importar. */
export interface Job {
  id: string;
  file: string;
  /** Endereço de onde o arquivo foi baixado (a lista mostra este em vez do caminho temporário). */
  origin?: string;
  /** Tabela do arquivo (planilha). */
  sheet: string;
  /** Tabela existente de destino ("" = não escolhida). */
  target: string;
  /** Nome de tabela nova (vale quando `target` está vazio). */
  newTable: string;
  mappings: MappingRow[];
  /** Campos do destino marcados como chave. */
  keys: string[];
}

export const MODES: { mode: ImportMode; label: MessageKey }[] = [
  { mode: "append", label: "imp.mode.append" },
  { mode: "update", label: "imp.mode.update" },
  { mode: "appendUpdate", label: "imp.mode.appendUpdate" },
  { mode: "appendNoUpdate", label: "imp.mode.appendNoUpdate" },
  { mode: "delete", label: "imp.mode.delete" },
  { mode: "copy", label: "imp.mode.copy" },
];

export const modeNeedsKey = (m: ImportMode) => m === "update" || m === "appendUpdate" || m === "appendNoUpdate" || m === "delete";

export interface ImportProfile {
  name: string;
  kind: Kind;
  settings: SourceSettings;
  mode: ImportMode;
  advanced: Advanced;
  /** Arquivos e tabelas escolhidos, com destino e mapeamento. */
  jobs: Job[];
}

export const PROFILES_KEY = "import.profiles";
const STORAGE_KEY = "atlas.import.wizard";

/** Último formato e opções usados (só deste computador). */
export function loadLast(): { kind: Kind; settings: SourceSettings; advanced: Advanced } {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as { kind?: string; settings?: Partial<SourceSettings>; advanced?: Partial<Advanced> } | null;
    const kind = ALL_FORMATS.some((f) => f.kind === raw?.kind) ? (raw?.kind as Kind) : "csv";
    return { kind, settings: { ...defaultSettings(kind), ...raw?.settings }, advanced: { ...defaultAdvanced(), ...raw?.advanced } };
  } catch {
    return { kind: "csv", settings: defaultSettings("csv"), advanced: defaultAdvanced() };
  }
}

export function saveLast(kind: Kind, settings: SourceSettings, advanced: Advanced) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ kind, settings, advanced }));
  } catch {
    // Sem armazenamento: só não lembra.
  }
}

/** Nome de arquivo sem pasta e sem extensão (nome padrão de uma tabela nova). */
export function stemOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? path;
  return name.replace(/\.[^.]*$/, "");
}

/** Nome de tabela/coluna "limpo" para o PostgreSQL (minúsculo, sem espaços nem acentos soltos). */
export function sqlName(name: string): string {
  const base = name.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase().replace(/[^a-z0-9_]+/g, "_").replace(/^_+|_+$/g, "");
  return /^[a-z_]/.test(base) ? base : base ? `c_${base}` : "campo";
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_LONG = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** Exemplos de data/hora no formato escolhido (o bloco "Date Time Example" do assistente). */
export function dateExamples(s: SourceSettings, now = new Date()): string[] {
  const p = (n: number) => String(n).padStart(2, "0");
  const time = `${p(now.getHours())}${s.timeDelimiter}${p(now.getMinutes())}${s.timeDelimiter}${p(now.getSeconds())}`;
  const d = now.getDate(), m = now.getMonth(), y = now.getFullYear();
  const dl = s.dateDelimiter;
  const compose = (day: string, month: string, year: string) => {
    const parts = s.dateOrder === "DMY" ? [day, month, year] : s.dateOrder === "MDY" ? [month, day, year] : [year, month, day];
    return parts.join(dl);
  };
  const variants = [
    compose(p(d), p(m + 1), String(y).slice(2)),
    compose(p(d), p(m + 1), String(y)),
    compose(p(d), MONTHS[m], String(y).slice(2)),
    compose(p(d), MONTHS_LONG[m], String(y)),
  ];
  return variants.map((v) => (s.dateTimeOrder === "DateTime" ? `${v} ${time}` : `${time} ${v}`));
}
