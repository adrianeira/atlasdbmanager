import { createContext, useContext } from "react";

/** Opções da tela Ferramentas → Opções, guardadas no banco local (preferência `ui.options`). */
export interface AppOptions {
  /** Tamanho da fonte do editor SQL, em px. */
  editorFontSize: number;
  /** Máximo de linhas mostradas por resultado no editor SQL. */
  queryMaxRows: number;
  /** Registros por página ao abrir uma tabela. */
  pageSize: number;
}

export const DEFAULT_OPTIONS: AppOptions = { editorFontSize: 13, queryMaxRows: 1000, pageSize: 1000 };

export const OPTION_LIMITS = {
  editorFontSize: [9, 28],
  queryMaxRows: [10, 100_000],
  pageSize: [10, 100_000],
} as const;

const clamp = (v: unknown, [min, max]: readonly [number, number], fallback: number): number => {
  const n = Math.floor(Number(v));
  return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback;
};

/** Valores guardados (possivelmente de outra versão, ou editados à mão) viram opções válidas. */
export function sanitizeOptions(raw: Partial<Record<keyof AppOptions, unknown>> | null | undefined): AppOptions {
  return {
    editorFontSize: clamp(raw?.editorFontSize, OPTION_LIMITS.editorFontSize, DEFAULT_OPTIONS.editorFontSize),
    queryMaxRows: clamp(raw?.queryMaxRows, OPTION_LIMITS.queryMaxRows, DEFAULT_OPTIONS.queryMaxRows),
    pageSize: clamp(raw?.pageSize, OPTION_LIMITS.pageSize, DEFAULT_OPTIONS.pageSize),
  };
}

export const OptionsContext = createContext<AppOptions>(DEFAULT_OPTIONS);
export const useOptions = (): AppOptions => useContext(OptionsContext);
