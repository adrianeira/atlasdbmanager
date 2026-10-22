import { useEffect, useMemo, useState } from "react";
import { DataGrid, typeKind, type GridColumn } from "../components/DataGrid";
import { MessageDialog } from "../components/MessageDialog";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import { mockRows, quoteIdent, type CellValue, type DbObject } from "../model";
import { columnType, selectSql } from "../sql/ddl";

interface Props {
  schema: string;
  table: DbObject;
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
}

const PAGE_SIZE = 1000;

function literal(v: CellValue): string {
  return v === null ? "NULL" : `'${v.replace(/'/g, "''")}'`;
}

export function TableDataDoc({ schema, table, active, production, onStatus }: Props) {
  const t = useT();
  const cols = table.columns ?? [];
  const keyCols = cols.map((c, i) => ({ c, i })).filter((x) => x.c.key).sort((a, b) => (a.c.key ?? 0) - (b.c.key ?? 0));
  const readOnly = keyCols.length === 0;

  const columns: GridColumn[] = useMemo(
    () => cols.map((c) => ({ name: c.name, type: columnType(c), kind: typeKind(c.type), isKey: !!c.key })),
    [cols],
  );

  const [original, setOriginal] = useState<CellValue[][]>(() => mockRows(table));
  const [rows, setRows] = useState<CellValue[][]>(original);
  const [edited, setEdited] = useState<Set<string>>(new Set());
  const [newRows, setNewRows] = useState<Set<number>>(new Set());
  const [deleted, setDeleted] = useState<Set<number>>(new Set());
  const [activeRow, setActiveRow] = useState(0);
  const [view, setView] = useState<"grid" | "form">("grid");
  const [refreshedAt, setRefreshedAt] = useState(() => new Date());
  const [review, setReview] = useState<string | null>(null);

  const dirty = edited.size > 0 || newRows.size > 0 || deleted.size > 0;
  const sqlText = selectSql(schema, table.name, PAGE_SIZE, 0);

  useEffect(() => {
    if (!active) return;
    onStatus([
      { text: sqlText, grow: true },
      { text: rows.length ? t("data.record", { n: activeRow + 1, total: rows.length, page: 1 }) : t("data.noRecords"), width: 200 },
      { text: t("data.lastRefresh", { when: refreshedAt.toLocaleTimeString() }), width: 240 },
    ]);
  }, [active, rows.length, activeRow, refreshedAt, sqlText, t, onStatus]);

  const onEdit = (r: number, c: number, v: CellValue) => {
    setRows((rs) => rs.map((row, i) => (i === r ? row.map((x, j) => (j === c ? v : x)) : row)));
    if (!newRows.has(r)) setEdited((s) => new Set(s).add(`${r}:${c}`));
  };

  const addRow = () => {
    if (readOnly) return;
    const idx = rows.length;
    setRows((rs) => [...rs, cols.map(() => null)]);
    setNewRows((s) => new Set(s).add(idx));
    setActiveRow(idx);
  };

  const deleteRow = () => {
    if (readOnly || !rows.length) return;
    if (newRows.has(activeRow)) {
      setRows((rs) => rs.filter((_, i) => i !== activeRow));
      setNewRows((s) => new Set([...s].filter((i) => i !== activeRow).map((i) => (i > activeRow ? i - 1 : i))));
      return;
    }
    setDeleted((s) => {
      const n = new Set(s);
      if (n.has(activeRow)) n.delete(activeRow);
      else n.add(activeRow);
      return n;
    });
  };

  const discard = () => {
    setRows(original);
    setEdited(new Set());
    setNewRows(new Set());
    setDeleted(new Set());
    setActiveRow((r) => Math.min(r, Math.max(0, original.length - 1)));
  };

  /** SQL gerado para revisão. Prévia legível; na execução real os valores vão como parâmetros. */
  const buildSql = (): string => {
    const target = `${quoteIdent(schema)}.${quoteIdent(table.name)}`;
    const where = (orig: CellValue[], extra: number[]) => {
      const conds = keyCols.map(({ c, i }) => `${quoteIdent(c.name)} = ${literal(orig[i])}`);
      // Concorrência otimista (ADR-027): compara também o valor original das colunas editadas.
      for (const i of extra) {
        if (keyCols.some((k) => k.i === i)) continue;
        conds.push(orig[i] === null ? `${quoteIdent(cols[i].name)} IS NULL` : `${quoteIdent(cols[i].name)} = ${literal(orig[i])}`);
      }
      return conds.join(" AND ");
    };
    const out: string[] = [];
    const byRow = new Map<number, number[]>();
    for (const k of edited) {
      const [r, c] = k.split(":").map(Number);
      if (deleted.has(r)) continue;
      byRow.set(r, [...(byRow.get(r) ?? []), c]);
    }
    for (const [r, cs] of byRow) {
      const set = cs.map((c) => `${quoteIdent(cols[c].name)} = ${literal(rows[r][c])}`).join(", ");
      out.push(`UPDATE ${target}\n   SET ${set}\n WHERE ${where(original[r], cs)};`);
    }
    for (const r of newRows) {
      const filled = cols.map((c, i) => ({ c, v: rows[r][i] })).filter((x) => x.v !== null);
      if (!filled.length) out.push(`INSERT INTO ${target} DEFAULT VALUES;`);
      else out.push(`INSERT INTO ${target} (${filled.map((x) => quoteIdent(x.c.name)).join(", ")})\nVALUES (${filled.map((x) => literal(x.v)).join(", ")});`);
    }
    for (const r of deleted) out.push(`DELETE FROM ${target}\n WHERE ${where(original[r], [])};`);
    return out.join("\n\n");
  };

  const apply = () => {
    // Mock: aplica localmente. No M1: transação no servidor, affected=1 por linha, senão conflito.
    const kept = rows.filter((_, i) => !deleted.has(i));
    setOriginal(kept);
    setRows(kept);
    setEdited(new Set());
    setNewRows(new Set());
    setDeleted(new Set());
    setReview(null);
    setRefreshedAt(new Date());
    setActiveRow((r) => Math.min(r, Math.max(0, kept.length - 1)));
  };

  const refresh = () => {
    if (dirty) return;
    setRefreshedAt(new Date());
  };

  const tool = (icon: SmallIconName, label: MessageKey, onClick?: () => void, extra = "") => (
    <button type="button" className={`ot-button${extra}`} disabled={!onClick} onClick={onClick} title={onClick ? t(label) : t("notImplemented")}>
      <SmallIcon name={icon} />
      <span>{t(label)}</span>
    </button>
  );

  const record = rows[activeRow];

  return (
    <div className="doc">
      <div className="object-toolbar" role="toolbar">
        {tool("table", "data.tableProfile")}
        {tool("transaction", "data.beginTransaction")}
        {tool("cellEditor", "data.cellEditor")}
        {tool("filterSort", "data.filterSort")}
        {tool("columns", "data.columns")}
        {tool("profiling", "data.profiling")}
        {tool("import", "data.import")}
        {tool("export", "data.export")}
        {tool("dataGen", "data.generation")}
        {tool("biCreate", "data.createBi")}
        <div className="ot-spacer" />
        <button type="button" className="icon-button" title={t("notImplemented")} disabled><SmallIcon name="fullscreen" /></button>
      </div>

      {readOnly && <div className="doc-banner">{t("data.readOnlyNoKey")}</div>}
      {!readOnly && production && <div className="doc-banner is-warn">{t("data.productionBanner")}</div>}

      <div className="doc-body">
        {view === "grid" ? (
          <DataGrid
            columns={columns}
            rows={rows}
            readOnly={readOnly}
            edited={edited}
            deletedRows={deleted}
            newRows={newRows}
            activeRow={activeRow}
            onActiveRow={setActiveRow}
            onEdit={onEdit}
          />
        ) : (
          <div className="form-view">
            {record ? cols.map((c, i) => (
              <div key={c.name} className="form-view-row">
                <label>{c.name}</label>
                <input
                  className="field"
                  value={record[i] ?? ""}
                  placeholder={record[i] === null ? "(NULL)" : ""}
                  readOnly={readOnly}
                  onChange={(e) => onEdit(activeRow, i, e.target.value)}
                />
                <span className="muted">{columnType(c)}</span>
              </div>
            )) : <div className="muted">{t("data.noRecords")}</div>}
          </div>
        )}
      </div>

      <div className="data-footer">
        <button type="button" className="icon-button" title={t("data.addRecord")} disabled={readOnly} onClick={addRow}><SmallIcon name="addRow" /></button>
        <button type="button" className="icon-button" title={t("data.deleteRecord")} disabled={readOnly || !rows.length} onClick={deleteRow}><SmallIcon name="removeRow" /></button>
        <button type="button" className="icon-button" title={t("data.apply")} disabled={!dirty} onClick={() => setReview(buildSql())}><SmallIcon name="apply" /></button>
        <button type="button" className="icon-button" title={t("data.discard")} disabled={!dirty} onClick={discard}><SmallIcon name="discard" /></button>
        <button type="button" className="icon-button" title={t("data.refresh")} disabled={dirty} onClick={refresh}><SmallIcon name="refresh" /></button>
        <button type="button" className="icon-button" title={t("data.stop")} disabled><SmallIcon name="stopSmall" /></button>
        {dirty && <span className="data-pending">{t("data.pending", { n: edited.size + newRows.size + deleted.size })}</span>}
        <span className="spacer" />
        <button type="button" className="icon-button" disabled title={t("data.firstPage")}><SmallIcon name="first" /></button>
        <button type="button" className="icon-button" disabled title={t("data.prevPage")}><SmallIcon name="prev" /></button>
        <input className="field page-input" value="1" readOnly aria-label={t("data.page")} />
        <button type="button" className="icon-button" disabled title={t("data.nextPage")}><SmallIcon name="next" /></button>
        <button type="button" className="icon-button" disabled title={t("data.lastPage")}><SmallIcon name="last" /></button>
        <button type="button" className="icon-button" disabled title={t("notImplemented")}><SmallIcon name="settings" /></button>
        <span className="data-gap" />
        <button type="button" className={`sb-button${view === "grid" ? " is-active" : ""}`} title={t("data.gridView")} onClick={() => setView("grid")}><SmallIcon name="gridView" /></button>
        <button type="button" className={`sb-button${view === "form" ? " is-active" : ""}`} title={t("data.formView")} onClick={() => setView("form")}><SmallIcon name="formView" /></button>
      </div>

      {review !== null && (
        <MessageDialog
          title={t("data.reviewTitle")}
          message={t("data.reviewMessage")}
          sql={review}
          confirmText={production ? table.name : undefined}
          danger={deleted.size > 0}
          okLabel={t("data.apply")}
          onOk={apply}
          onCancel={() => setReview(null)}
        />
      )}
    </div>
  );
}
