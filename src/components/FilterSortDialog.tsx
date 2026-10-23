import { useMemo, useState } from "react";
import type { OrderBy, TableFilter } from "../backend";
import { useT } from "../i18n";
import type { ColumnDef } from "../model";
import { quoteIdent } from "../model";
import { Dropdown } from "./Dropdown";
import { SmallIcon } from "../icons";

type Op = "=" | "<>" | "<" | "<=" | ">" | ">=" | "LIKE" | "NOT LIKE" | "IS NULL" | "IS NOT NULL" | "IN" | "BETWEEN";

interface Rule {
  id: number;
  column: string;
  op: Op;
  value: string;
  value2: string; // só para BETWEEN
}

interface Props {
  columns: ColumnDef[];
  initialFilter: TableFilter | null;
  initialSort: OrderBy[];
  /** Coluna a pré-selecionar numa nova regra quando não há filtro ainda (ex.: "Add Filter" do cabeçalho). */
  initialColumn?: string;
  onApply: (filter: TableFilter | null, sort: OrderBy[]) => void;
  onClose: () => void;
}

const OP_LABELS: Record<Op, string> = {
  "=": "=",
  "<>": "≠",
  "<": "<",
  "<=": "≤",
  ">": ">",
  ">=": "≥",
  "LIKE": "LIKE",
  "NOT LIKE": "NOT LIKE",
  "IS NULL": "IS NULL",
  "IS NOT NULL": "IS NOT NULL",
  "IN": "IN",
  "BETWEEN": "BETWEEN",
};
const OPS: Op[] = ["=", "<>", "<", "<=", ">", ">=", "LIKE", "NOT LIKE", "IN", "BETWEEN", "IS NULL", "IS NOT NULL"];
const NO_VALUE: Op[] = ["IS NULL", "IS NOT NULL"];

function pgQuote(v: string): string {
  // Escapa aspas simples duplicando (PostgreSQL com standard_conforming_strings = on).
  return `'${v.replace(/'/g, "''")}'`;
}

function castSuffix(col: ColumnDef): string {
  // Texto não precisa de cast; números/datas se beneficiam.
  const t = (col.castType ?? col.type).toLowerCase();
  if (/(char|text|name|citext)/.test(t)) return "";
  return `::${col.castType ?? col.type}`;
}

function literal(col: ColumnDef, v: string): string {
  return `${pgQuote(v)}${castSuffix(col)}`;
}

function buildExpr(rules: Rule[], columns: ColumnDef[]): string {
  const parts: string[] = [];
  for (const r of rules) {
    const col = columns.find((c) => c.name === r.column);
    if (!col) continue;
    const q = quoteIdent(col.name);
    if (r.op === "IS NULL" || r.op === "IS NOT NULL") {
      parts.push(`${q} ${r.op}`);
    } else if (r.op === "BETWEEN") {
      if (!r.value || !r.value2) continue;
      parts.push(`${q} BETWEEN ${literal(col, r.value)} AND ${literal(col, r.value2)}`);
    } else if (r.op === "IN") {
      const items = r.value.split(",").map((x) => x.trim()).filter(Boolean);
      if (!items.length) continue;
      parts.push(`${q} IN (${items.map((v) => literal(col, v)).join(", ")})`);
    } else if (r.op === "LIKE" || r.op === "NOT LIKE") {
      parts.push(`${q} ${r.op} ${pgQuote(r.value)}`);
    } else {
      if (!r.value) continue;
      parts.push(`${q} ${r.op} ${literal(col, r.value)}`);
    }
  }
  return parts.join(" AND ");
}

export function FilterSortDialog({ columns, initialFilter, initialSort, initialColumn, onApply, onClose }: Props) {
  const t = useT();
  const colItems = useMemo(() => columns.map((c) => ({ value: c.name, label: `${c.name}  (${c.type})` })), [columns]);

  const [rules, setRules] = useState<Rule[]>(() => (
    initialFilter?.expr ? [{ id: 0, column: columns[0]?.name ?? "", op: "=", value: "", value2: "" }]
    : initialColumn ? [{ id: 0, column: initialColumn, op: "=", value: "", value2: "" }]
    : []
  ));
  const [sort, setSort] = useState<OrderBy[]>(initialSort);
  const nextId = useMemo(() => ({ v: rules.reduce((m, r) => Math.max(m, r.id), 0) + 1 }), []);

  const addRule = () => {
    setRules((rs) => [...rs, { id: nextId.v++, column: columns[0]?.name ?? "", op: "=", value: "", value2: "" }]);
  };
  const updateRule = (i: number, patch: Partial<Rule>) => setRules((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const removeRule = (i: number) => setRules((rs) => rs.filter((_, j) => j !== i));

  const addSort = () => setSort((s) => [...s, { column: columns[0]?.name ?? "", descending: false }]);
  const updateSort = (i: number, patch: Partial<OrderBy>) => setSort((s) => s.map((o, j) => (j === i ? { ...o, ...patch } : o)));
  const removeSort = (i: number) => setSort((s) => s.filter((_, j) => j !== i));

  const expr = useMemo(() => buildExpr(rules, columns), [rules, columns]);
  const orderPreview = useMemo(() => sort.map((o) => `${quoteIdent(o.column)} ${o.descending ? "DESC" : "ASC"}`).join(", "), [sort]);

  const apply = () => {
    onApply(expr ? { expr } : null, sort.filter((o) => o.column));
    onClose();
  };
  const clearAll = () => { setRules([]); setSort([]); };

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal fs-dialog" role="dialog" aria-label={t("fs.title")}>
        <div className="modal-header"><span className="modal-title">{t("fs.title")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="modal-body">
          <section className="fs-section">
            <div className="fs-section-head">
              <h4>{t("fs.filter")}</h4>
              <button type="button" className="btn" onClick={addRule}>+ {t("fs.addRule")}</button>
            </div>
            {rules.length === 0 && <p className="muted">{t("fs.noRules")}</p>}
            {rules.map((r, i) => {
              const needsValue = !NO_VALUE.includes(r.op);
              return (
                <div key={r.id} className="fs-row">
                  <Dropdown className="fs-col" value={r.column} items={colItems} onChange={(v) => updateRule(i, { column: v })} />
                  <Dropdown className="fs-op" value={r.op} items={OPS.map((op) => ({ value: op, label: OP_LABELS[op] }))} onChange={(v) => updateRule(i, { op: v as Op })} />
                  {needsValue && (
                    <input className="field fs-val" value={r.value} placeholder={r.op === "IN" ? "a, b, c" : ""} onChange={(e) => updateRule(i, { value: e.target.value })} />
                  )}
                  {r.op === "BETWEEN" && (
                    <>
                      <span className="fs-and">{t("fs.and")}</span>
                      <input className="field fs-val" value={r.value2} onChange={(e) => updateRule(i, { value2: e.target.value })} />
                    </>
                  )}
                  <button type="button" className="icon-button" title={t("fs.remove")} onClick={() => removeRule(i)}>−</button>
                </div>
              );
            })}
          </section>

          <section className="fs-section">
            <div className="fs-section-head">
              <h4>{t("fs.sort")}</h4>
              <button type="button" className="btn" onClick={addSort}>+ {t("fs.addSort")}</button>
            </div>
            {sort.length === 0 && <p className="muted">{t("fs.noSort")}</p>}
            {sort.map((o, i) => (
              <div key={i} className="fs-row">
                <Dropdown className="fs-col" value={o.column} items={colItems} onChange={(v) => updateSort(i, { column: v })} />
                <Dropdown
                  className="fs-op"
                  value={o.descending ? "DESC" : "ASC"}
                  items={[{ value: "ASC", label: t("fs.asc") }, { value: "DESC", label: t("fs.desc") }]}
                  onChange={(v) => updateSort(i, { descending: v === "DESC" })}
                />
                <button type="button" className="icon-button" title={t("fs.remove")} onClick={() => removeSort(i)}>−</button>
              </div>
            ))}
          </section>

          <section className="fs-preview">
            <div className="fs-preview-label">{t("fs.preview")}</div>
            <code>
              {expr || <span className="muted">{t("fs.noWhere")}</span>}
              {orderPreview && <> {expr && <br />}ORDER BY {orderPreview}</>}
            </code>
          </section>
        </div>
        <div className="modal-footer">
          <button type="button" className="btn" onClick={clearAll}>{t("fs.clear")}</button>
          <span className="spacer" />
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
          <button type="button" className="btn btn-primary" onClick={apply}>{t("common.apply")}</button>
        </div>
      </div>
    </div>
  );
}
