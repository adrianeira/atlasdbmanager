import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent } from "react";
import type { CellValue } from "../model";

/*
 * Grid de dados virtualizado do Atlas (ADR-011, alternativa própria).
 * Só as linhas visíveis vão para o DOM; o pai fornece as linhas por página
 * (no M1 elas chegam em streaming do núcleo Rust).
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

const KIND_GLYPH: Record<TypeKind, string> = { number: "#", text: "ᴬᴮc", time: "◷", bool: "☑", json: "{}", binary: "01", uuid: "id", other: "?" };

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
}

export function DataGrid({ columns, rows, readOnly, edited, deletedRows, newRows, activeRow, onActiveRow, onEdit, rowHeight = 23 }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(400);
  const [activeCol, setActiveCol] = useState(0);
  const [editing, setEditing] = useState<{ row: number; col: number; value: string; isNull: boolean } | null>(null);
  const [widths, setWidths] = useState<number[]>([]);

  useEffect(() => {
    setWidths(columns.map((c) => c.width ?? (c.kind === "number" ? 150 : c.kind === "time" ? 200 : c.kind === "uuid" ? 280 : 150)));
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

  const headerHeight = 41;
  const overscan = 8;
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const last = Math.min(rows.length, Math.ceil((scrollTop + viewport) / rowHeight) + overscan);
  const totalWidth = 14 + widths.reduce((a, b) => a + b, 0);

  const ensureVisible = (row: number) => {
    const el = scrollRef.current;
    if (!el) return;
    const top = row * rowHeight;
    const bodyHeight = el.clientHeight - headerHeight;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + rowHeight > el.scrollTop + bodyHeight) el.scrollTop = top + rowHeight - bodyHeight;
  };

  const startEdit = (row: number, col: number, initial?: string) => {
    if (readOnly || !onEdit || deletedRows?.has(row)) return;
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
          <div className="dg-rowhead" />
          {columns.map((c, i) => (
            <div key={c.name + i} className="dg-th" style={{ width: widths[i] }} role="columnheader" title={`${c.name} (${c.type})`}>
              <div className="dg-th-name">{c.name}</div>
              {/* Em lotes com vários comandos o servidor não descreve os tipos: a linha fica vazia. */}
              <div className="dg-th-type">{c.type && <><span className={`dg-glyph is-${c.kind}`}>{KIND_GLYPH[c.kind]}</span>{c.type}</>}</div>
              <span className="dg-resize" onMouseDown={(e) => startResize(i, e)} />
            </div>
          ))}
        </div>

        {rows.slice(first, last).map((r, k) => {
          const ri = first + k;
          const isActive = ri === activeRow;
          const cls = `dg-row${isActive ? " is-active" : ""}${deletedRows?.has(ri) ? " is-deleted" : ""}${newRows?.has(ri) ? " is-new" : ""}`;
          return (
            <div key={ri} className={cls} style={{ top: headerHeight + ri * rowHeight, height: rowHeight }} role="row" aria-rowindex={ri + 1}>
              <div className="dg-rowhead">{isActive ? "▸" : ""}</div>
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
                        placeholder={editing.isNull ? "(NULL)" : ""}
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
                      <span className="dg-null">(NULL)</span>
                    ) : (
                      v
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
