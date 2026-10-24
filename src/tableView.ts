/** A "visão" de uma tabela aberta (ordenação, filtro, colunas e larguras), lembrada entre aberturas do app. */

export interface TableViewState {
  sort: { column: string; descending?: boolean }[];
  /** Condição WHERE digitada no Filter & Sort (vazia = sem filtro). */
  filter: string;
  /** Colunas escondidas pelo botão Columns. */
  hidden: string[];
  /** Largura de cada coluna que o usuário ajustou, em px. */
  widths: Record<string, number>;
  /** "Limitar registros" ligado e quantos por página. */
  limitOn: boolean;
  pageSize: number;
}

export const prefKey = (viewKey: string) => `tv:${viewKey}`;

/** Chave de uma tabela: conexão, banco, schema e nome. */
export const viewKeyOf = (connectionId: string, database: string, schema: string, table: string) => [connectionId, database, schema, table].join("/");

/** Lê o que veio das preferências, descartando o que não vale mais (coluna que não existe mais, valores fora do intervalo). */
export function sanitizeView(raw: unknown, columns: string[], fallback: { limitOn: boolean; pageSize: number }): TableViewState {
  const known = new Set(columns);
  const r = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const sort = Array.isArray(r.sort)
    ? r.sort.flatMap((s): TableViewState["sort"] => {
        const o = s && typeof s === "object" ? (s as Record<string, unknown>) : {};
        return typeof o.column === "string" && known.has(o.column) ? [{ column: o.column, descending: o.descending === true }] : [];
      })
    : [];
  const hidden = Array.isArray(r.hidden) ? r.hidden.filter((c): c is string => typeof c === "string" && known.has(c)) : [];
  // Nunca esconde todas as colunas: a tabela pareceria vazia.
  const visible = columns.filter((c) => !hidden.includes(c)).length;
  const widths: Record<string, number> = {};
  if (r.widths && typeof r.widths === "object") {
    for (const [name, w] of Object.entries(r.widths as Record<string, unknown>)) {
      if (known.has(name) && typeof w === "number" && Number.isFinite(w)) widths[name] = Math.min(600, Math.max(40, Math.round(w)));
    }
  }
  const size = typeof r.pageSize === "number" && Number.isFinite(r.pageSize) ? Math.min(1_000_000, Math.max(1, Math.floor(r.pageSize))) : fallback.pageSize;
  return {
    sort,
    filter: typeof r.filter === "string" ? r.filter.slice(0, 4000) : "",
    hidden: visible > 0 ? hidden : [],
    widths,
    limitOn: typeof r.limitOn === "boolean" ? r.limitOn : fallback.limitOn,
    pageSize: size,
  };
}

/** `true` se a visão é a padrão (nada para lembrar). */
export const isDefaultView = (v: TableViewState, fallback: { limitOn: boolean; pageSize: number }): boolean =>
  !v.sort.length && !v.filter.trim() && !v.hidden.length && !Object.keys(v.widths).length && v.limitOn === fallback.limitOn && v.pageSize === fallback.pageSize;
