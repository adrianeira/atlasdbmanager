import { createContext, useContext } from "react";

export type RowShading = "none" | "every2" | "every3";

export const EDITOR_COLOR_KEYS = ["common", "keyword", "string", "number", "comment", "background"] as const;
export type EditorColorKey = (typeof EDITOR_COLOR_KEYS)[number];
/** Cor de cada elemento do editor (`#rrggbb`); vazio = a do tema. */
export type EditorColors = Record<EditorColorKey, string>;
export const NO_EDITOR_COLORS: EditorColors = { common: "", keyword: "", string: "", number: "", comment: "", background: "" };

/** Opções da tela Ferramentas → Opções, guardadas no banco local (preferência `ui.options`). */
export interface AppOptions {
  /** Tamanho da fonte do editor SQL, em px. */
  editorFontSize: number;
  /** Máximo de linhas mostradas por resultado no editor SQL. */
  queryMaxRows: number;
  /** Registros por página ao abrir uma tabela. */
  pageSize: number;
  /** Editor SQL. */
  showLineNumbers: boolean;
  codeFolding: boolean;
  braceHighlight: boolean;
  syntaxHighlight: boolean;
  wordWrap: boolean;
  tabWidth: number;
  insertSpaces: boolean;
  /** Completar código (lista de sugestões ao digitar) e já marcar a primeira. */
  codeCompletion: boolean;
  autoSelectFirst: boolean;
  /** Fonte do editor (vazio = a padrão do tema) e cores. */
  editorFont: string;
  editorColors: EditorColors;
  /** Registros: abrir tabelas com "Limitar registros" ligado e o sombreamento das linhas. */
  limitRecords: boolean;
  rowShading: RowShading;
  /** Fonte da grade (vazio = Segoe UI) e tamanho em px; "Auto begin transaction": abrir tabelas já dentro de uma transação. */
  gridFont: string;
  gridFontSize: number;
  autoBeginTransaction: boolean;
  /** Formato de exibição de data, hora e data/hora (vazio = como vem do servidor) e separadores dos números. */
  displayDate: string;
  displayTime: string;
  displayDateTime: string;
  showThousands: boolean;
  systemLocale: boolean;
  /** Geral: permitir abrir o mesmo objeto em mais de uma aba. */
  multipleForms: boolean;
  /** Geral: perguntar antes de fechar uma consulta com texto não guardado (transação aberta sempre pergunta). */
  confirmCloseQueries: boolean;
  /** Ao iniciar: só a aba Objects, ou as consultas que estavam abertas. */
  startup: "objects" | "continue";
  /** Auto Recovery: guardar as consultas abertas a cada `autoSaveSeconds` segundos. */
  autoSaveQueries: boolean;
  autoSaveSeconds: number;
}

export const DEFAULT_OPTIONS: AppOptions = {
  editorFontSize: 13, queryMaxRows: 1000, pageSize: 1000,
  showLineNumbers: true, codeFolding: true, braceHighlight: true, syntaxHighlight: true, wordWrap: false, tabWidth: 2, insertSpaces: true,
  codeCompletion: true, autoSelectFirst: true,
  editorFont: "", editorColors: NO_EDITOR_COLORS,
  limitRecords: true, rowShading: "every2", gridFont: "", gridFontSize: 12, autoBeginTransaction: false,
  displayDate: "", displayTime: "", displayDateTime: "", showThousands: false, systemLocale: false,
  multipleForms: false, confirmCloseQueries: true,
  startup: "objects", autoSaveQueries: true, autoSaveSeconds: 30,
};

export const OPTION_LIMITS = {
  editorFontSize: [9, 28],
  gridFontSize: [9, 20],
  queryMaxRows: [10, 100_000],
  pageSize: [10, 100_000],
  tabWidth: [1, 16],
  autoSaveSeconds: [5, 3600],
} as const;

type NumericKey = keyof typeof OPTION_LIMITS;

const clamp = (v: unknown, [min, max]: readonly [number, number], fallback: number): number => {
  const n = Math.floor(Number(v));
  return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback;
};

const bool = (v: unknown, fallback: boolean) => (typeof v === "boolean" ? v : fallback);

/** Valores guardados (possivelmente de outra versão, ou editados à mão) viram opções válidas. */
export function sanitizeOptions(raw: Partial<Record<keyof AppOptions, unknown>> | null | undefined): AppOptions {
  const d = DEFAULT_OPTIONS;
  const num = (k: NumericKey) => clamp(raw?.[k], OPTION_LIMITS[k], d[k]);
  return {
    editorFontSize: num("editorFontSize"),
    gridFontSize: num("gridFontSize"),
    gridFont: typeof raw?.gridFont === "string" ? raw.gridFont.replace(/[;{}"\\]/g, "").slice(0, 80) : d.gridFont,
    autoBeginTransaction: bool(raw?.autoBeginTransaction, d.autoBeginTransaction),
    queryMaxRows: num("queryMaxRows"),
    pageSize: num("pageSize"),
    tabWidth: num("tabWidth"),
    autoSaveSeconds: num("autoSaveSeconds"),
    startup: raw?.startup === "continue" ? "continue" : "objects",
    autoSaveQueries: bool(raw?.autoSaveQueries, d.autoSaveQueries),
    showLineNumbers: bool(raw?.showLineNumbers, d.showLineNumbers),
    codeFolding: bool(raw?.codeFolding, d.codeFolding),
    braceHighlight: bool(raw?.braceHighlight, d.braceHighlight),
    syntaxHighlight: bool(raw?.syntaxHighlight, d.syntaxHighlight),
    wordWrap: bool(raw?.wordWrap, d.wordWrap),
    insertSpaces: bool(raw?.insertSpaces, d.insertSpaces),
    codeCompletion: bool(raw?.codeCompletion, d.codeCompletion),
    autoSelectFirst: bool(raw?.autoSelectFirst, d.autoSelectFirst),
    editorFont: typeof raw?.editorFont === "string" ? raw.editorFont.replace(/[;{}"\\]/g, "").slice(0, 80) : d.editorFont,
    editorColors: sanitizeColors(raw?.editorColors),
    limitRecords: bool(raw?.limitRecords, d.limitRecords),
    rowShading: raw?.rowShading === "none" || raw?.rowShading === "every2" || raw?.rowShading === "every3" ? raw.rowShading : d.rowShading,
    multipleForms: bool(raw?.multipleForms, d.multipleForms),
    confirmCloseQueries: bool(raw?.confirmCloseQueries, d.confirmCloseQueries),
    displayDate: pattern(raw?.displayDate),
    displayTime: pattern(raw?.displayTime),
    displayDateTime: pattern(raw?.displayDateTime),
    showThousands: bool(raw?.showThousands, d.showThousands),
    systemLocale: bool(raw?.systemLocale, d.systemLocale),
  };
}

const pattern = (v: unknown): string => (typeof v === "string" ? v.slice(0, 40) : "");

function sanitizeColors(raw: unknown): EditorColors {
  const out = { ...NO_EDITOR_COLORS };
  if (raw && typeof raw === "object") {
    for (const k of EDITOR_COLOR_KEYS) {
      const v = (raw as Record<string, unknown>)[k];
      if (typeof v === "string" && /^#[0-9a-fA-F]{6}$/.test(v)) out[k] = v.toLowerCase();
    }
  }
  return out;
}

/** Valor da propriedade CSS `font` da grade (também usado para medir a largura das colunas). */
export const gridFontCss = (o: Pick<AppOptions, "gridFont" | "gridFontSize">, weight = ""): string =>
  `${weight ? weight + " " : ""}${o.gridFontSize}px ${o.gridFont ? `"${o.gridFont}", ` : ""}Segoe UI, sans-serif`;

export const OptionsContext = createContext<AppOptions>(DEFAULT_OPTIONS);
export const useOptions = (): AppOptions => useContext(OptionsContext);
