import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactElement } from "react";
import type { CellValue } from "../model";

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
}

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
function autoWidth(c: GridColumn): number {
  const name = textWidth(c.name, "600 12px Segoe UI, sans-serif") + 18;
  const type = c.type ? textWidth(c.type, "12px Segoe UI, sans-serif") + 34 : 0;
  return Math.ceil(Math.min(320, Math.max(BASE_WIDTH[c.kind], name, type)));
}

/** Texto exibido na célula (sem alterar o valor): BLOB vira "(BLOB) N bytes" como na referência. */
function display(v: string, kind: TypeKind): string {
  if (kind === "binary" && v.startsWith("\\x")) return `(BLOB) ${(v.length - 2) / 2} bytes`;
  return v;
}

interface Props {
  columns: GridColumn[];
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
}

export function DataGrid({
  columns, rows, readOnly, edited, deletedRows, newRows, activeRow, onActiveRow, onEdit, rowHeight = 23, simpleHeader, lookupCols, onLookup,
}: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(400);
  const [activeCol, setActiveCol] = useState(0);
  const [editing, setEditing] = useState<{ row: number; col: number; value: string; isNull: boolean } | null>(null);
  const [widths, setWidths] = useState<number[]>([]);

  useEffect(() => {
    setWidths(columns.map((c) => c.width ?? autoWidth(c)));
    setActiveCol(0);
    setEditing(null);
  }, [columns]);

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
  const totalWidth = rowHeadWidth + widths.reduce((a, b) => a + b, 0);

  const ensureVisible = (row: number) => {
    const el = scrollRef.current;
    if (!el) return;
    const top = row * rowHeight;
    const bodyHeight = el.clientHeight - headerHeight;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + rowHeight > el.scrollTop + bodyHeight) el.scrollTop = top + rowHeight - bodyHeight;
  };

  const startEdit = (row: number, col: number, initial?: string) => {
    // Binários não são editados como texto no grid (editor próprio de BLOB — spec §31.6).
    if (readOnly || !onEdit || deletedRows?.has(row) || columns[col]?.kind === "binary") return;
    const v = rows[row][col];
    setEditing({ row, col, value: initial ?? v ?? "", isNull: initial === undefined && v === null });
  };

  const commitEdit = (setNull = false) => {
    if (!editing) return;
    const original = rows[editing.row][editing.col];
    const next = setNull || (editing.isNull && editing.value === "") ? null : editing.value;
    if (next !== original) onEdit?.(editing.row, editing.col, next);
    setEditing(null);
    scrollRef.current?.focus();
  };

  const move = (dr: number, dc: number) => {
    const r = Math.max(0, Math.min(rows.length - 1, activeRow + dr));
    const c = Math.max(0, Math.min(columns.length - 1, activeCol + dc));
    onActiveRow(r);
    setActiveCol(c);
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
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) startEdit(activeRow, activeCol, e.key);
    }
  };

  const startResize = (i: number, e: ReactMouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startW = widths[i];
    const onMove = (ev: MouseEvent) => setWidths((w) => w.map((x, j) => (j === i ? Math.max(40, startW + ev.clientX - startX) : x)));
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  // Duplo clique na borda do cabeçalho ajusta a coluna ao conteúdo visível.
  const fitColumn = (i: number) => {
    const c = columns[i];
    let w = autoWidth(c);
    for (const r of rows.slice(first, last)) {
      const v = r[i];
      const t = v === null ? "(Null)" : display(v, c.kind);
      w = Math.max(w, textWidth(t, "12px Segoe UI, sans-serif") + 14);
    }
    setWidths((ws) => ws.map((x, j) => (j === i ? Math.min(600, Math.ceil(w)) : x)));
  };

  return (
    <div
      className="dg"
      ref={scrollRef}
      tabIndex={0}
      role="grid"
      aria-rowcount={rows.length}
      aria-readonly={readOnly}
      onKeyDown={onKeyDown}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
    >
      <div className="dg-inner" style={{ width: totalWidth, height: headerHeight + rows.length * rowHeight }}>
        <div className="dg-header" style={{ height: headerHeight }} role="row">
          <div className="dg-rowhead" style={{ width: rowHeadWidth }} />
          {columns.map((c, i) => (
            <div
              key={c.name + i}
              className={`dg-th${i === activeCol && rows.length ? " is-active-col" : ""}`}
              style={{ width: widths[i] }}
              role="columnheader"
              title={c.type ? `${c.name} (${c.type})` : c.name}
              onMouseDown={() => setActiveCol(i)}
            >
              <div className="dg-th-name">{c.name}</div>
              {/* Em lotes com vários comandos o servidor não descreve os tipos: a linha fica vazia. */}
              {!simpleHeader && <div className="dg-th-type">{c.type && <><KindIcon kind={c.kind} /><span>{c.type}</span></>}</div>}
              <span className="dg-resize" onMouseDown={(e) => startResize(i, e)} onDoubleClick={() => fitColumn(i)} />
            </div>
          ))}
        </div>

        {rows.slice(first, last).map((r, k) => {
          const ri = first + k;
          const isActive = ri === activeRow;
          // Linhas zebradas (alternadas), como na referência.
          const cls = `dg-row${ri % 2 === 1 ? " is-alt" : ""}${isActive ? " is-active" : ""}${deletedRows?.has(ri) ? " is-deleted" : ""}${newRows?.has(ri) ? " is-new" : ""}`;
          return (
            <div key={ri} className={cls} style={{ top: headerHeight + ri * rowHeight, height: rowHeight }} role="row" aria-rowindex={ri + 1}>
              <div className="dg-rowhead" style={{ width: rowHeadWidth }} />
              {columns.map((c, ci) => {
                const v = r[ci];
                const isEditing = editing?.row === ri && editing.col === ci;
                const cellCls = `dg-td${c.kind === "number" ? " is-num" : ""}${isActive && ci === activeCol ? " is-focus" : ""}${edited?.has(`${ri}:${ci}`) ? " is-edited" : ""}`;
                return (
                  <div
                    key={ci}
                    className={cellCls}
                    style={{ width: widths[ci] }}
                    role="gridcell"
                    onMouseDown={() => { onActiveRow(ri); setActiveCol(ci); if (editing && !isEditing) commitEdit(); }}
                    onDoubleClick={() => startEdit(ri, ci)}
                  >
                    {isEditing ? (
                      <input
                        className="dg-editor"
                        autoFocus
                        value={editing.value}
                        placeholder={editing.isNull ? "(Null)" : ""}
                        onChange={(e) => setEditing({ ...editing, value: e.target.value, isNull: false })}
                        onBlur={() => commitEdit()}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Enter") { commitEdit(); move(1, 0); }
                          else if (e.key === "Escape") { setEditing(null); scrollRef.current?.focus(); }
                          else if (e.key === "Tab") { e.preventDefault(); commitEdit(); move(0, e.shiftKey ? -1 : 1); }
                          // Ctrl+0 grava NULL explicitamente (NULL ≠ texto vazio).
                          else if (e.key === "0" && e.ctrlKey) { e.preventDefault(); commitEdit(true); }
                        }}
                      />
                    ) : v === null ? (
                      <span className="dg-null">(Null)</span>
                    ) : (
                      display(v, c.kind)
                    )}
                    {/* Lookup de FK: botão "…" na célula ativa (como a referência). */}
                    {!isEditing && isActive && ci === activeCol && lookupCols?.has(ci) && onLookup && !readOnly && !deletedRows?.has(ri) && (
                      <button
                        type="button"
                        className="dg-lookup"
                        title="…"
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
    </div>
  );
}
