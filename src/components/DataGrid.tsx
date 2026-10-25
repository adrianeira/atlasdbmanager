import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactElement } from "react";
import { createPortal } from "react-dom";
import { copyToClipboard, readClipboard } from "../clipboard";
import type { MenuEntry } from "../commands";
import { useT } from "../i18n";
import { formatNumber, formatTemporal } from "../format";
import { gridFontCss, useOptions } from "../options";
import { quoteIdent, type CellValue } from "../model";
import { DateTimePopup, type DateTimeKind } from "./DateTimePopup";
import { PopupMenu } from "./PopupMenu";

/*
 * Grid de dados virtualizado do Atlas (ADR-011, alternativa própria).
 * Só as linhas visíveis vão para o DOM; o pai fornece as linhas por página.
 * Aparência calibrada pela grade da referência: cabeçalho em duas linhas (nome em negrito,
 * ícone + tipo em cinza), "(Null)" cinza, BLOB resumido, linha/coluna ativas em azul-claro.
 */

export type TypeKind = "number" | "text" | "time" | "bool" | "json" | "binary" | "uuid" | "other";

export interface GridColumn {
  name: string;
  type: string;
  kind: TypeKind;
  width?: number;
  isKey?: boolean;
  /** NOT NULL: o grid não deixa a célula ficar vazia nem NULL. */
  required?: boolean;
  /** Tem DEFAULT no servidor: num registro novo pode ficar em branco (o banco preenche). */
  hasDefault?: boolean;
  /** Coluna gerada pelo banco (identity ALWAYS ou STORED): bloqueada para edição. */
  generated?: boolean;
  /** Oculta a coluna do grid (botão "Columns" da toolbar); os dados continuam no array de linhas. */
  hidden?: boolean;
  /** Comentário da coluna no banco (dica do cabeçalho). */
  comment?: string;
}

/** Valor que a célula receberia: fora de colunas de texto, vazio significa NULL. */
function normalize(v: CellValue, kind: TypeKind): CellValue {
  return v === "" && kind !== "text" ? null : v;
}

/** Valor em branco numa coluna obrigatória (NULL ou texto vazio). */
const isBlank = (v: CellValue) => v === null || v === "";

export function typeKind(type: string): TypeKind {
  const t = type.toLowerCase();
  if (/^(int|smallint|bigint|serial|numeric|decimal|real|double|float|money)/.test(t)) return "number";
  if (/^(timestamp|date|time|interval)/.test(t)) return "time";
  if (t === "bool" || t === "boolean") return "bool";
  if (t.startsWith("json")) return "json";
  if (t === "bytea" || t.includes("binary") || t === "blob") return "binary";
  if (t === "uuid") return "uuid";
  if (/(char|text|name|citext)/.test(t)) return "text";
  return "other";
}

/* Ícones do tipo no cabeçalho (desenho próprio, 12px, cinza). */
const ring = (
  <>
    <circle cx="4.5" cy="7" r="3" fill="none" stroke="currentColor" strokeWidth="1.1" />
    <circle cx="8.5" cy="7" r="3" fill="none" stroke="currentColor" strokeWidth="1.1" />
  </>
);
const KIND_ICON: Record<TypeKind, ReactElement> = {
  number: <text x="6" y="10.5" textAnchor="middle" fontSize="10" fontFamily="Segoe UI, sans-serif" fill="currentColor">#</text>,
  text: (
    <>
      <text x="-0.5" y="7" fontSize="7.5" fontFamily="Segoe UI, sans-serif" fontWeight="600" fill="currentColor">A</text>
      <text x="4.3" y="7" fontSize="7.5" fontFamily="Segoe UI, sans-serif" fontWeight="600" fill="currentColor">B</text>
      <text x="8.2" y="12.5" fontSize="8.5" fontFamily="Segoe UI, sans-serif" fill="currentColor">c</text>
    </>
  ),
  time: (
    <>
      <circle cx="6.5" cy="7" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.1" />
      <path d="M6.5 4.5V7l1.8 1.2" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinecap="round" />
    </>
  ),
  bool: (
    <>
      <rect x="2" y="2.5" width="9" height="9" rx="1" fill="none" stroke="currentColor" strokeWidth="1.1" />
      <path d="M4 7l2 2 3.5-4" fill="none" stroke="currentColor" strokeWidth="1.1" />
    </>
  ),
  json: <text x="6.5" y="10" textAnchor="middle" fontSize="8.5" fontFamily="Consolas, monospace" fill="currentColor">{"{}"}</text>,
  binary: ring,
  uuid: ring,
  other: ring,
};

function KindIcon({ kind }: { kind: TypeKind }) {
  return (
    <svg className="dg-type-icon" width="13" height="13" viewBox="0 0 13 13" aria-hidden="true">
      {KIND_ICON[kind]}
    </svg>
  );
}

/* Larguras automáticas: maior entre o cabeçalho (nome/tipo) e a base do tipo. */
const BASE_WIDTH: Record<TypeKind, number> = { number: 64, text: 150, time: 150, bool: 64, json: 150, binary: 100, uuid: 150, other: 120 };
let measureCtx: CanvasRenderingContext2D | null = null;
function textWidth(text: string, font: string): number {
  measureCtx ??= document.createElement("canvas").getContext("2d");
  if (!measureCtx) return text.length * 7;
  measureCtx.font = font;
  return measureCtx.measureText(text).width;
}
function autoWidth(c: GridColumn, font = "12px Segoe UI, sans-serif", bold = "600 12px Segoe UI, sans-serif"): number {
  const name = textWidth(c.name, bold) + 18;
  const type = c.type ? textWidth(c.type, font) + 34 : 0;
  return Math.ceil(Math.min(320, Math.max(BASE_WIDTH[c.kind], name, type)));
}

/** Subkind temporal para o picker custom (interval volta a ser editado como texto). */
function dtKind(type: string): DateTimeKind | null {
  const t = type.toLowerCase();
  if (t.startsWith("timestamp")) return "datetime";
  if (t.startsWith("date")) return "date";
  if (t.startsWith("time")) return "time";
  return null;
}

/** Texto exibido na célula (sem alterar o valor): BLOB vira "(BLOB) N bytes" como na referência. */
function display(v: string, kind: TypeKind): string {
  if (kind === "binary" && v.startsWith("\\x")) return `(BLOB) ${(v.length - 2) / 2} bytes`;
  return v;
}

/** Literal SQL para "Copiar como": números sem aspas, texto com aspas simples duplicadas. */
function sqlLiteral(v: CellValue, kind: TypeKind): string {
  if (v === null) return "NULL";
  if (kind === "number" && /^-?\d+(\.\d+)?(e[-+]?\d+)?$/i.test(v)) return v;
  return `'${v.replace(/'/g, "''")}'`;
}

/** UUID v7 (RFC 9562): 48 bits de milissegundos + aleatório; ordena por tempo de criação. */
function uuidV7(): string {
  const b = crypto.getRandomValues(new Uint8Array(16));
  let ms = Date.now();
  for (let i = 5; i >= 0; i--) {
    b[i] = ms % 256;
    ms = Math.floor(ms / 256);
  }
  b[6] = (b[6] & 0x0f) | 0x70;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = [...b].map((x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

const copyText = (text: string) => void copyToClipboard(text).catch(() => undefined);

interface Props {
  columns: GridColumn[];
  /** Coluna → (valor → texto mostrado no lugar dele); só muda o que se vê (ex.: chave estrangeira mostrada pela descrição). */
  valueLabels?: Map<number, Map<string, string>>;
  /** O usuário terminou de ajustar a largura de uma coluna (arrastando a borda ou com duplo clique). */
  onColumnWidth?: (name: string, width: number) => void;
  /** Rolagem vertical atual (para guardar) e a rolagem com que abrir (de uma tela anterior). */
  onScrollTop?: (top: number) => void;
  initialScrollTop?: number;
  /** Ordem de exibição das colunas, por nome (as que não aparecem vão para o fim); com `onColumnOrder` os cabeçalhos podem ser arrastados. */
  columnOrder?: string[];
  onColumnOrder?: (names: string[]) => void;
  rows: CellValue[][];
  readOnly?: boolean;
  /** Chaves "linha:coluna" de células alteradas e ainda não aplicadas. */
  edited?: Set<string>;
  deletedRows?: Set<number>;
  newRows?: Set<number>;
  activeRow: number;
  onActiveRow: (row: number) => void;
  onEdit?: (row: number, col: number, value: CellValue) => void;
  rowHeight?: number;
  /** Cabeçalho de uma linha só (sem tipo), como na grade do lookup de FK. */
  simpleHeader?: boolean;
  /** Colunas com lookup de chave estrangeira: a célula ativa mostra o botão "…". */
  lookupCols?: Set<number>;
  onLookup?: (row: number, col: number, cell: DOMRect) => void;
  /** `"schema"."tabela"` para "Copiar como INSERT/UPDATE"; ausente (ex.: resultado de consulta) desabilita. */
  sqlTarget?: string;
  onDeleteRow?: (row: number) => void;
  /** "Duplicar registro" no menu da linha (some se não for possível, ex.: há alterações pendentes). */
  onDuplicateRow?: (row: number) => void;
  onRefresh?: () => void;
  /** Barra "Find Data": destaca células que contêm o texto. Vazio = sem busca. */
  findQuery?: string;
  /** Célula da ocorrência atual (destaque mais forte + rolagem). */
  activeFind?: { row: number; col: number } | null;
  /** Marcar todas as ocorrências (desligado = só a atual). */
  highlightAll?: boolean;
  /** Menu "⋯" no cabeçalho (hover): ordenar a coluna. Omitir esconde o botão. */
  /** "Salvar dados como...": abre o assistente de exportação. Omitir desabilita o item. */
  onSaveAs?: () => void;
  onSortColumn?: (col: number, descending: boolean) => void;
  /** Idem: "Remove All Sorts" no menu do cabeçalho. */
  onClearSort?: () => void;
  /** "Restaurar layout padrão" no menu do cabeçalho: ordem, largura e visibilidade das colunas e a ordenação das linhas. */
  onResetLayout?: () => void;
  /** Idem: "Add Filter" no menu do cabeçalho. */
  onAddFilter?: (col: number) => void;
  /** Avisa qual é a célula ativa (linha/coluna) a cada mudança — usado pelo painel "Cell Editor". */
  onActiveCell?: (row: number, col: number) => void;
  /** "Edit in Cell Editor" no menu de contexto da célula: abre o painel inferior nessa célula. */
  onOpenCellEditor?: (row: number, col: number) => void;
}

export function DataGrid({
  columns, onColumnWidth, onScrollTop, initialScrollTop, columnOrder, onColumnOrder, valueLabels, rows, readOnly, edited, deletedRows, newRows, activeRow, onActiveRow, onEdit, rowHeight = 23, simpleHeader, lookupCols, onLookup,
  sqlTarget, onDeleteRow, onDuplicateRow, onRefresh, findQuery, activeFind, highlightAll, onSaveAs, onSortColumn, onClearSort, onResetLayout, onAddFilter, onActiveCell, onOpenCellEditor,
}: Props) {
  const t = useT();
  const opts = useOptions();
  const font = gridFontCss(opts);
  const fontBold = gridFontCss(opts, "600");
  const { rowShading, displayDate, displayTime, displayDateTime, showThousands, systemLocale } = useOptions();
  /** Texto da célula com os formatos de data/hora e de número das Opções. */
  const shown = (v: string, c: GridColumn, ci: number): string => {
    const label = valueLabels?.get(ci)?.get(v);
    if (label !== undefined) return label;
    const dk = dtKind(c.type);
    if (dk) return formatTemporal(v, dk, dk === "date" ? displayDate : dk === "time" ? displayTime : displayDateTime);
    return c.kind === "number" ? formatNumber(v, showThousands, systemLocale) : display(v, c.kind);
  };
  /** Linhas sombreadas (Opções → Registros → Row Shading). */
  const shadeRow = (ri: number) => (rowShading === "every2" ? ri % 2 === 1 : rowShading === "every3" ? ri % 3 === 2 : false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; row: number; col: number } | null>(null);
  const [headerMenu, setHeaderMenu] = useState<{ x: number; y: number; col: number } | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  // Rolagem de uma tela anterior: aplicada quando as linhas já chegaram, uma vez só.
  const scrollRestored = useRef(false);
  useEffect(() => {
    if (scrollRestored.current || !initialScrollTop || !rows.length) return;
    scrollRestored.current = true;
    window.requestAnimationFrame(() => { if (scrollRef.current) scrollRef.current.scrollTop = initialScrollTop; });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows.length]);
  const [viewport, setViewport] = useState(400);
  const [activeCol, setActiveCol] = useState(0);
  const [editing, setEditing] = useState<{ row: number; col: number; value: string; isNull: boolean } | null>(null);
  const [widths, setWidths] = useState<number[]>([]);
  const [editorEl, setEditorEl] = useState<HTMLInputElement | null>(null);

  // Índices reais (em `columns`/`rows`) das colunas visíveis, na ordem — "Columns" da toolbar
  // só afeta exibição/navegação; edição, FK e cópia continuam indexadas pela coluna real.
  /** Todas as colunas na ordem de exibição (índices); as ocultas ficam fora de `visible`. */
  const ordered = useMemo(() => {
    const rank = new Map((columnOrder ?? []).map((n, i) => [n, i]));
    return columns.map((_, i) => i).sort((a, b) => (rank.get(columns[a].name) ?? 1e9 + a) - (rank.get(columns[b].name) ?? 1e9 + b));
  }, [columns, columnOrder]);
  const visible = useMemo(() => ordered.filter((i) => !columns[i]?.hidden), [ordered, columns]);
  /** Cabeçalho sendo arrastado e onde ele cairia (antes ou depois da coluna alvo). */
  const [dragCol, setDragCol] = useState<number | null>(null);
  const [dropAt, setDropAt] = useState<{ col: number; after: boolean } | null>(null);
  const dropColumn = () => {
    if (dragCol === null || !dropAt || dragCol === dropAt.col || !onColumnOrder) return;
    const names = ordered.filter((i) => i !== dragCol).map((i) => columns[i].name);
    const at = names.indexOf(columns[dropAt.col].name) + (dropAt.after ? 1 : 0);
    names.splice(at, 0, columns[dragCol].name);
    onColumnOrder(names);
  };

  // Larguras/posição só são reiniciadas quando a ESTRUTURA muda de verdade (nome/tipo), não a
  // cada mostrar/ocultar coluna — senão o diálogo "Columns" resetaria larguras ajustadas manualmente.
  const structureKey = useMemo(() => columns.map((c) => `${c.name}:${c.type}`).join("|"), [columns]);
  useEffect(() => {
    setWidths(columns.map((c) => c.width ?? autoWidth(c, font, fontBold)));
    const firstVisible = columns.findIndex((c) => !c.hidden);
    setActiveCol(firstVisible >= 0 ? firstVisible : 0);
    setEditing(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [structureKey]);

  // Larguras que chegam de fora depois de montar (a visão lembrada da tabela) valem sobre as automáticas.
  const widthsKey = columns.map((c) => c.width ?? "").join("|");
  useEffect(() => {
    if (columns.length && columns.every((c) => c.width === undefined)) setWidths(columns.map((c) => autoWidth(c, font, fontBold)));
    else setWidths((ws) => ws.map((w, i) => columns[i]?.width ?? w));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [widthsKey]);

  // Se a coluna ativa for ocultada, pula para a primeira coluna visível.
  useEffect(() => {
    if (visible.length && !visible.includes(activeCol)) setActiveCol(visible[0]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  useEffect(() => {
    onActiveCell?.(activeRow, activeCol);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeRow, activeCol]);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewport(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const headerHeight = simpleHeader ? 24 : 41;
  const rowHeadWidth = 12;
  const overscan = 8;
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const last = Math.min(rows.length, Math.ceil((scrollTop + viewport) / rowHeight) + overscan);
  const totalWidth = rowHeadWidth + visible.reduce((a, i) => a + (widths[i] ?? 0), 0);

  const findLower = findQuery ? findQuery.toLowerCase() : "";
  const cellMatches = (v: CellValue, kind: TypeKind): boolean => {
    if (!findLower || v === null) return false;
    // Checkbox/booleano: comparar com "true"/"false"/"t"/"f" não acrescenta nada útil — ignorado.
    if (kind === "bool") return false;
    return display(v, kind).toLowerCase().includes(findLower);
  };

  const ensureVisible = (row: number) => {
    const el = scrollRef.current;
    if (!el) return;
    const top = row * rowHeight;
    const bodyHeight = el.clientHeight - headerHeight;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + rowHeight > el.scrollTop + bodyHeight) el.scrollTop = top + rowHeight - bodyHeight;
  };

  // Ocorrência ativa da busca: foca a célula e rola o grid para dentro da viewport.
  useEffect(() => {
    if (!activeFind) return;
    onActiveRow(activeFind.row);
    setActiveCol(activeFind.col);
    ensureVisible(activeFind.row);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeFind?.row, activeFind?.col]);

  // Linha ativa alterada por fora (ex.: "Adicionar registro" criou linha no fim):
  // garante que ela fique visível na viewport.
  useEffect(() => {
    ensureVisible(activeRow);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeRow]);

  /** Booleano: alterna true/false (NULL vira true), no formato do servidor ("t"/"f"). */
  const toggleBool = (row: number, col: number) => {
    if (readOnly || !onEdit || deletedRows?.has(row) || columns[col]?.generated) return;
    const v = rows[row][col];
    onEdit(row, col, v === "t" || v === "true" ? "f" : "t");
  };

  const startEdit = (row: number, col: number, initial?: string) => {
    // Binários não são editados como texto no grid (editor próprio de BLOB — spec §31.6).
    // Colunas geradas pelo banco (identity ALWAYS / STORED) são sempre bloqueadas.
    if (readOnly || !onEdit || deletedRows?.has(row) || columns[col]?.kind === "binary" || columns[col]?.generated) return;
    // Booleano é checkbox: Enter/F2/espaço alternam em vez de abrir o editor de texto.
    if (columns[col]?.kind === "bool") {
      if (initial === undefined || initial === " ") toggleBool(row, col);
      return;
    }
    const v = rows[row][col];
    setEditing({ row, col, value: initial ?? v ?? "", isNull: initial === undefined && v === null });
  };

  /** Grava a edição. Em coluna obrigatória, valor em branco é recusado: devolve false e a célula fica marcada. */
  const commitEdit = (setNull = false): boolean => {
    if (!editing) return true;
    const col = columns[editing.col];
    const original = rows[editing.row][editing.col];
    const next = setNull || (editing.isNull && editing.value === "") ? null : normalize(editing.value, col.kind);
    if (col.required && isBlank(next)) return false;
    if (next !== original) onEdit?.(editing.row, editing.col, next);
    setEditing(null);
    scrollRef.current?.focus();
    return true;
  };

  /** Sai da edição sem gravar (Esc, ou ao clicar fora com valor inválido). */
  const cancelEdit = () => {
    setEditing(null);
    scrollRef.current?.focus();
  };

  const move = (dr: number, dc: number) => {
    const r = Math.max(0, Math.min(rows.length - 1, activeRow + dr));
    if (dc !== 0 && visible.length) {
      // dc conta passos entre colunas VISÍVEIS (colunas ocultas são puladas na navegação).
      const pos = Math.max(0, visible.indexOf(activeCol));
      const nextPos = Math.max(0, Math.min(visible.length - 1, pos + dc));
      setActiveCol(visible[nextPos]);
    }
    onActiveRow(r);
    ensureVisible(r);
  };

  const onKeyDown = (e: ReactKeyboardEvent) => {
    if (editing || !rows.length) return;
    const page = Math.max(1, Math.floor(viewport / rowHeight) - 2);
    switch (e.key) {
      case "ArrowDown": e.preventDefault(); move(1, 0); break;
      case "ArrowUp": e.preventDefault(); move(-1, 0); break;
      case "ArrowRight": e.preventDefault(); move(0, 1); break;
      case "ArrowLeft": e.preventDefault(); move(0, -1); break;
      case "PageDown": e.preventDefault(); move(page, 0); break;
      case "PageUp": e.preventDefault(); move(-page, 0); break;
      case "Home": e.preventDefault(); e.ctrlKey ? move(-rows.length, -columns.length) : move(0, -columns.length); break;
      case "End": e.preventDefault(); e.ctrlKey ? move(rows.length, columns.length) : move(0, columns.length); break;
      case "Tab": e.preventDefault(); move(0, e.shiftKey ? -1 : 1); break;
      case "F2":
      case "Enter": e.preventDefault(); startEdit(activeRow, activeCol); break;
      default:
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          if (e.key === " ") e.preventDefault();
          startEdit(activeRow, activeCol, e.key);
        }
    }
  };

  const startResize = (i: number, e: ReactMouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startW = widths[i];
    let last = startW;
    const onMove = (ev: MouseEvent) => {
      last = Math.max(40, startW + ev.clientX - startX);
      setWidths((w) => w.map((x, j) => (j === i ? last : x)));
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      if (last !== startW) onColumnWidth?.(columns[i].name, Math.round(last));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  /* ----- Menu de contexto da célula e menu "⋯" do cabeçalho (mesmo padrão de fechamento) ----- */
  useEffect(() => {
    if (!menu && !headerMenu) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof MouseEvent && (e.target as HTMLElement).closest?.(".dg-menu")) return;
      setMenu(null);
      setHeaderMenu(null);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
      window.removeEventListener("resize", close);
    };
  }, [menu, headerMenu]);

  const canEdit = (row: number, col: number) => !readOnly && !!onEdit && !deletedRows?.has(row) && columns[col]?.kind !== "binary";
  const setCell = (row: number, col: number, raw: CellValue) => {
    const v = normalize(raw, columns[col].kind);
    if (columns[col].required && isBlank(v)) return;
    if (rows[row]?.[col] !== v) onEdit?.(row, col, v);
  };

  const insertSql = (row: number) =>
    `INSERT INTO ${sqlTarget} (${columns.map((c) => quoteIdent(c.name)).join(", ")}) VALUES (${columns.map((c, i) => sqlLiteral(rows[row][i], c.kind)).join(", ")});`;
  const updateSql = (row: number) => {
    const keys = columns.map((c, i) => ({ c, i })).filter((x) => x.c.isKey);
    const where = (keys.length ? keys : columns.map((c, i) => ({ c, i })))
      .map(({ c, i }) => (rows[row][i] === null ? `${quoteIdent(c.name)} IS NULL` : `${quoteIdent(c.name)} = ${sqlLiteral(rows[row][i], c.kind)}`))
      .join(" AND ");
    const set = columns.map((c, i) => ({ c, i })).filter((x) => !x.c.isKey)
      .map(({ c, i }) => `${quoteIdent(c.name)} = ${sqlLiteral(rows[row][i], c.kind)}`).join(", ");
    return `UPDATE ${sqlTarget} SET ${set} WHERE ${where};`;
  };
  const tsv = (row: number, names: boolean, data: boolean) =>
    [names ? columns.map((c) => c.name).join("\t") : null, data ? rows[row].map((v) => v ?? "").join("\t") : null].filter((x) => x !== null).join("\r\n");

  const menuItems = (row: number, col: number): MenuEntry[] => {
    const editable = canEdit(row, col);
    const kind = columns[col]?.kind;
    const required = !!columns[col]?.required;
    const item = (key: Parameters<typeof t>[0], run?: () => void): MenuEntry => ({ kind: "item", text: t(key), run });
    const sep: MenuEntry = { kind: "sep" };
    const uuidOk = editable && (kind === "uuid" || kind === "text" || kind === "other");
    return [
      // Campo obrigatório (NOT NULL) não pode ficar vazio nem NULL.
      item("grid.setEmpty", editable && kind === "text" && !required ? () => setCell(row, col, "") : undefined),
      item("grid.setNull", editable && !required ? () => setCell(row, col, null) : undefined),
      {
        kind: "sub", label: "grid.generateUuid",
        items: [
          item("grid.uuidV4", uuidOk ? () => setCell(row, col, crypto.randomUUID()) : undefined),
          item("grid.uuidV7", uuidOk ? () => setCell(row, col, uuidV7()) : undefined),
        ],
      },
      item("grid.cellEditor", onOpenCellEditor ? () => onOpenCellEditor(row, col) : undefined),
      sep,
      item("grid.duplicateRecord", !readOnly && onDuplicateRow ? () => onDuplicateRow(row) : undefined),
      item("grid.deleteRecord", !readOnly && onDeleteRow ? () => onDeleteRow(row) : undefined),
      sep,
      item("grid.copy", () => copyText(rows[row][col] ?? "")),
      item("grid.copyFieldName", () => copyText(columns[col].name)),
      {
        kind: "sub", label: "grid.copyAs",
        items: [
          item("grid.copyInsert", sqlTarget ? () => copyText(insertSql(row)) : undefined),
          item("grid.copyUpdate", sqlTarget ? () => copyText(updateSql(row)) : undefined),
          sep,
          item("grid.copyTsvData", () => copyText(tsv(row, false, true))),
          item("grid.copyTsvNames", () => copyText(tsv(row, true, false))),
          item("grid.copyTsvBoth", () => copyText(tsv(row, true, true))),
        ],
      },
      item("grid.paste", editable ? () => {
        void readClipboard()
          // Colar em branco num obrigatório é ignorado pelo setCell.
          .then((text) => setCell(row, col, text.replace(/\r?\n$/, "")))
          .catch(() => undefined);
      } : undefined),
      item("grid.saveDataAs", onSaveAs),
      sep,
      item("grid.toggleFilter"),
      { kind: "sub", label: "grid.filter", items: [item("grid.filter")] },
      { kind: "sub", label: "grid.sort", items: [item("grid.sortAsc"), item("grid.sortDesc")] },
      item("grid.removeFilters"),
      sep,
      item("grid.refresh", onRefresh),
    ];
  };

  /** Menu "⋯" do cabeçalho: Sort Ascending/Descending, Remove All Sorts, Add Filter. */
  const headerMenuItems = (col: number): MenuEntry[] => {
    const item = (key: Parameters<typeof t>[0], run?: () => void): MenuEntry => ({ kind: "item", text: t(key), run });
    const sep: MenuEntry = { kind: "sep" };
    return [
      item("grid.sortAsc", onSortColumn ? () => onSortColumn(col, false) : undefined),
      item("grid.sortDesc", onSortColumn ? () => onSortColumn(col, true) : undefined),
      item("grid.removeAllSorts", onClearSort ? () => onClearSort() : undefined),
      sep,
      item("grid.addFilter", onAddFilter ? () => onAddFilter(col) : undefined),
      sep,
      item("grid.resetLayout", onResetLayout ? () => onResetLayout() : undefined),
    ];
  };

  const openMenu = (e: ReactMouseEvent, row: number, col: number) => {
    e.preventDefault();
    if (editing) commitEdit();
    onActiveRow(row);
    setActiveCol(col);
    setMenu({ x: e.clientX, y: e.clientY, row, col });
  };

  // Duplo clique na borda do cabeçalho ajusta a coluna ao conteúdo visível.
  const fitColumn = (i: number) => {
    const c = columns[i];
    let w = autoWidth(c, font, fontBold);
    for (const r of rows.slice(first, last)) {
      const v = r[i];
      const t = v === null ? "(Null)" : display(v, c.kind);
      w = Math.max(w, textWidth(t, font) + 14);
    }
    const fitted = Math.min(600, Math.ceil(w));
    setWidths((ws) => ws.map((x, j) => (j === i ? fitted : x)));
    onColumnWidth?.(c.name, fitted);
  };

  return (
    <div
      className="dg"
      style={{ font }}
      ref={scrollRef}
      tabIndex={0}
      role="grid"
      aria-rowcount={rows.length}
      aria-readonly={readOnly}
      onKeyDown={onKeyDown}
      onScroll={(e) => { setScrollTop(e.currentTarget.scrollTop); onScrollTop?.(e.currentTarget.scrollTop); }}
    >
      <div className="dg-inner" style={{ width: totalWidth, height: headerHeight + rows.length * rowHeight }}>
        <div className="dg-header" style={{ height: headerHeight }} role="row">
          <div className="dg-rowhead" style={{ width: rowHeadWidth }} />
          {visible.map((i) => {
            const c = columns[i];
            return (
            <div
              key={c.name + i}
              className={`dg-th${i === activeCol && rows.length ? " is-active-col" : ""}${dropAt?.col === i && dragCol !== i ? (dropAt.after ? " is-drop-after" : " is-drop-before") : ""}`}
              style={{ width: widths[i] }}
              draggable={!!onColumnOrder}
              onDragStart={(e) => {
                setDragCol(i);
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", c.name);
              }}
              onDragOver={(e) => {
                if (dragCol === null) return;
                e.preventDefault();
                const r = e.currentTarget.getBoundingClientRect();
                const after = e.clientX > r.left + r.width / 2;
                if (dropAt?.col !== i || dropAt.after !== after) setDropAt({ col: i, after });
              }}
              onDrop={(e) => { e.preventDefault(); dropColumn(); setDragCol(null); setDropAt(null); }}
              onDragEnd={() => { setDragCol(null); setDropAt(null); }}
              role="columnheader"
              title={`${c.type ? `${c.name} (${c.type})` : c.name}${c.comment ? `\n${c.comment}` : ""}`}
              onMouseDown={() => setActiveCol(i)}
            >
              <div className="dg-th-name">{c.name}</div>
              {/* Em lotes com vários comandos o servidor não descreve os tipos: a linha fica vazia. */}
              {!simpleHeader && <div className="dg-th-type">{c.type && <><KindIcon kind={c.kind} /><span>{c.type}</span></>}</div>}
              {/* "⋯" aparece no hover do cabeçalho: Sort Ascending/Descending, Remove All Sorts, Add Filter. */}
              {(onSortColumn || onClearSort || onAddFilter) && (
                <button
                  type="button"
                  className="dg-th-menu"
                  aria-label={t("grid.columnMenu")}
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    const r = e.currentTarget.getBoundingClientRect();
                    setHeaderMenu({ x: r.left, y: r.bottom, col: i });
                  }}
                >
                  ⋯
                </button>
              )}
              <span className="dg-resize" onMouseDown={(e) => startResize(i, e)} onDoubleClick={() => fitColumn(i)} />
            </div>
            );
          })}
        </div>

        {rows.slice(first, last).map((r, k) => {
          const ri = first + k;
          const isActive = ri === activeRow;
          // Linhas zebradas (alternadas), como na referência.
          const cls = `dg-row${shadeRow(ri) ? " is-alt" : ""}${isActive ? " is-active" : ""}${deletedRows?.has(ri) ? " is-deleted" : ""}${newRows?.has(ri) ? " is-new" : ""}`;
          return (
            <div key={ri} className={cls} style={{ top: headerHeight + ri * rowHeight, height: rowHeight }} role="row" aria-rowindex={ri + 1}>
              <div className="dg-rowhead" style={{ width: rowHeadWidth }} />
              {visible.map((ci) => {
                const c = columns[ci];
                const v = r[ci];
                const isEditing = editing?.row === ri && editing.col === ci;
                // Obrigatório em branco: no editor enquanto digita, ou num registro novo ainda sem valor.
                const editorInvalid = isEditing && !!c.required && editing.value === "";
                const missing = !isEditing && !!c.required && !c.hasDefault && v === null && !!newRows?.has(ri);
                const isFindActive = !!activeFind && activeFind.row === ri && activeFind.col === ci;
                const isFindMatch = isFindActive || (highlightAll !== false && cellMatches(v, c.kind));
                const cellCls = `dg-td${c.kind === "number" ? " is-num" : c.kind === "bool" ? " is-bool" : ""}${isActive && ci === activeCol ? " is-focus" : ""}${edited?.has(`${ri}:${ci}`) ? " is-edited" : ""}${missing || editorInvalid ? " is-required" : ""}${isFindMatch ? " is-find-match" : ""}${isFindActive ? " is-find-active" : ""}${c.generated ? " is-generated" : ""}`;
                return (
                  <div
                    key={ci}
                    className={cellCls}
                    style={{ width: widths[ci] }}
                    role="gridcell"
                    title={missing || editorInvalid ? t("grid.required", { name: c.name }) : c.generated ? t("grid.generated", { name: c.name }) : undefined}
                    onMouseDown={() => { onActiveRow(ri); setActiveCol(ci); if (editing && !isEditing && !commitEdit()) cancelEdit(); }}
                    onDoubleClick={() => startEdit(ri, ci)}
                    onContextMenu={isEditing ? undefined : (e) => openMenu(e, ri, ci)}
                  >
                    {isEditing ? (
                      <input
                        className="dg-editor"
                        autoFocus
                        value={editing.value}
                        placeholder={editing.isNull ? "(Null)" : ""}
                        ref={setEditorEl}
                        onChange={(e) => setEditing({ ...editing, value: e.target.value, isNull: false })}
                        // Em data/hora, o blur é tratado pelo próprio popup (ignora para não cancelar ao clicar nele).
                        onBlur={dtKind(c.type) ? undefined : () => { if (!commitEdit()) cancelEdit(); }}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Enter") { if (commitEdit()) move(1, 0); }
                          else if (e.key === "Escape") cancelEdit();
                          else if (e.key === "Tab") { e.preventDefault(); if (commitEdit()) move(0, e.shiftKey ? -1 : 1); }
                          else if (e.key === "0" && e.ctrlKey) { e.preventDefault(); commitEdit(true); }
                        }}
                      />
                    ) : c.kind === "bool" ? (
                      // Checkbox: marcado = true; desmarcado = false ou NULL (a dica diz "(Null)").
                      // Clicar grava true/false; para voltar a NULL, "Definir como NULL" no menu.
                      <input
                        type="checkbox"
                        className="dg-check"
                        checked={v === "t" || v === "true"}
                        title={v === null ? "(Null)" : undefined}
                        aria-readonly={readOnly || !onEdit}
                        onMouseDown={(e) => { onActiveRow(ri); setActiveCol(ci); e.stopPropagation(); }}
                        // Um clique = uma troca. O duplo clique não chega à célula (que também alternaria).
                        onDoubleClick={(e) => e.stopPropagation()}
                        onChange={() => toggleBool(ri, ci)}
                      />
                    ) : v === null ? (
                      <span className="dg-null">(Null)</span>
                    ) : (
                      shown(v, c, ci)
                    )}
                    {/* Ícone discreto à direita em colunas de data/hora/timestamp (como na referência). */}
                    {!isEditing && dtKind(c.type) && (
                      <span className="dg-dt-icon" aria-hidden="true">
                        {dtKind(c.type) === "time" ? (
                          <svg width="12" height="12" viewBox="0 0 12 12"><circle cx="6" cy="6" r="4.5" fill="none" stroke="currentColor" strokeWidth="1" /><path d="M6 3.5V6l1.8 1.1" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="round" /></svg>
                        ) : (
                          <svg width="12" height="12" viewBox="0 0 12 12"><rect x="1.5" y="2.5" width="9" height="8" rx="1" fill="none" stroke="currentColor" strokeWidth="1" /><path d="M1.5 5h9M4 1.5v2M8 1.5v2" stroke="currentColor" strokeWidth="1" strokeLinecap="round" fill="none" /></svg>
                        )}
                      </span>
                    )}
                    {/* Lookup de FK: botão "…" na célula ativa (como a referência). */}
                    {!isEditing && isActive && ci === activeCol && lookupCols?.has(ci) && onLookup && !readOnly && !deletedRows?.has(ri) && (
                      <button
                        type="button"
                        className="dg-lookup"
                        aria-label={t("grid.fkLookup")}
                        onMouseDown={(e) => e.stopPropagation()}
                        onClick={(e) => onLookup(ri, ci, (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect())}
                      >
                        …
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>
      {menu && rows[menu.row] && createPortal(
        <PopupMenu
          items={menuItems(menu.row, menu.col)}
          commands={{}}
          onClose={() => { setMenu(null); scrollRef.current?.focus(); }}
          className="ctx-menu dg-menu"
          style={{ position: "fixed", left: Math.min(menu.x, window.innerWidth - 300), top: Math.min(menu.y, window.innerHeight - 470) }}
        />,
        document.body,
      )}
      {headerMenu && createPortal(
        <PopupMenu
          items={headerMenuItems(headerMenu.col)}
          commands={{}}
          onClose={() => { setHeaderMenu(null); scrollRef.current?.focus(); }}
          className="ctx-menu dg-menu"
          style={{ position: "fixed", left: Math.min(headerMenu.x, window.innerWidth - 220), top: Math.min(headerMenu.y, window.innerHeight - 180) }}
        />,
        document.body,
      )}
      {editing && editorEl && (() => {
        const kind = dtKind(columns[editing.col]?.type ?? "");
        if (!kind) return null;
        const anchor = editorEl.getBoundingClientRect();
        return (
          <DateTimePopup
            kind={kind}
            value={editing.value}
            anchor={anchor}
            onPick={(v) => {
              setEditing({ ...editing, value: v, isNull: false });
              // Commit imediato: picker é escolha explícita, não precisa confirmar.
              const col = columns[editing.col];
              const next = v === "" ? null : v;
              if (col.required && (next === null || next === "")) { setEditing(null); scrollRef.current?.focus(); return; }
              if (next !== rows[editing.row][editing.col]) onEdit?.(editing.row, editing.col, next);
              setEditing(null);
              scrollRef.current?.focus();
            }}
            onClose={() => { setEditing(null); scrollRef.current?.focus(); }}
          />
        );
      })()}
    </div>
  );
}
