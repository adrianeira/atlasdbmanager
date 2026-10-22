import { useCallback, useEffect, useMemo, useState } from "react";
import { errorText, toAtlasError, type PlannedStatement } from "../backend";
import { DataGrid, typeKind, type GridColumn } from "../components/DataGrid";
import { FkLookup, type FkLookupSource } from "../components/FkLookup";
import { MessageDialog } from "../components/MessageDialog";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import { plannedStatements } from "../live";
import { mockRows, quoteIdent, type CellValue, type DbObject, type ForeignKeyDef } from "../model";
import { columnType, selectSql } from "../sql/ddl";

/** Acesso ao banco real; ausente = modo de demonstração com dados fictícios. */
export interface TableDataBackend {
  loadDetail: () => Promise<DbObject>;
  /** `limit = null`: sem LIMIT ("Limitar registros" desmarcado). */
  loadRows: (orderBy: string[], limit: number | null, offset: number) => Promise<{ sql: string; rows: CellValue[][]; truncated: boolean }>;
  apply: (statements: PlannedStatement[]) => Promise<void>;
}

interface Props {
  schema: string;
  table: DbObject;
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  backend?: TableDataBackend;
  /** Leitura das tabelas referenciadas pelas FKs (botão "…" na célula). */
  lookup?: FkLookupSource;
  /** View: sempre somente leitura. */
  isView?: boolean;
}

const DEFAULT_PAGE_SIZE = 1000;
const MAX_PAGE_SIZE = 1_000_000;

function literal(v: CellValue): string {
  return v === null ? "NULL" : `'${v.replace(/'/g, "''")}'`;
}

export function TableDataDoc({ schema, table: tableProp, active, production, onStatus, backend, lookup, isView }: Props) {
  const t = useT();
  const [table, setTable] = useState<DbObject>(tableProp);
  const cols = useMemo(() => table.columns ?? [], [table]);
  const keyCols = cols.map((c, i) => ({ c, i })).filter((x) => x.c.key).sort((a, b) => (a.c.key ?? 0) - (b.c.key ?? 0));
  const readOnly = isView || keyCols.length === 0;

  const columns: GridColumn[] = useMemo(
    () => cols.map((c) => ({ name: c.name, type: columnType(c), kind: typeKind(c.type), isKey: !!c.key })),
    [cols],
  );

  const [original, setOriginal] = useState<CellValue[][]>(() => (backend ? [] : mockRows(tableProp)));
  const [rows, setRows] = useState<CellValue[][]>(original);
  const [edited, setEdited] = useState<Set<string>>(new Set());
  const [newRows, setNewRows] = useState<Set<number>>(new Set());
  const [deleted, setDeleted] = useState<Set<number>>(new Set());
  const [activeRow, setActiveRow] = useState(0);
  const [view, setView] = useState<"grid" | "form">("grid");
  const [refreshedAt, setRefreshedAt] = useState(() => new Date());
  const [review, setReview] = useState<string | null>(null);
  const [page, setPage] = useState(0);
  const [loading, setLoading] = useState(!!backend);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  const [sqlText, setSqlText] = useState(() => selectSql(schema, tableProp.name, DEFAULT_PAGE_SIZE, 0));
  // Engrenagem do rodapé (como na referência): troca a paginação por "Limitar registros".
  const [limitPanel, setLimitPanel] = useState(false);
  const [limitOn, setLimitOn] = useState(true);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [sizeText, setSizeText] = useState(String(DEFAULT_PAGE_SIZE));
  const [truncated, setTruncated] = useState(false);
  const limit = limitOn ? pageSize : null;
  const [fkOpen, setFkOpen] = useState<{ row: number; fk: ForeignKeyDef; anchor: DOMRect } | null>(null);

  // Coluna → FK a que pertence (para o botão "…" de lookup).
  const fkByCol = useMemo(() => {
    const m = new Map<number, ForeignKeyDef>();
    for (const fk of table.foreignKeys ?? []) {
      for (const f of fk.fields.split(",").map((x) => x.trim())) {
        const i = cols.findIndex((c) => c.name === f);
        if (i >= 0 && !m.has(i)) m.set(i, fk);
      }
    }
    return m;
  }, [table, cols]);
  const lookupCols = useMemo(() => new Set(fkByCol.keys()), [fkByCol]);
  const fkColumnIndexes = (fk: ForeignKeyDef) => fk.fields.split(",").map((f) => cols.findIndex((c) => c.name === f.trim()));

  const resetEdits = () => {
    setEdited(new Set());
    setNewRows(new Set());
    setDeleted(new Set());
  };

  const loadPage = useCallback(async (p: number, structure: DbObject, lim: number | null) => {
    if (!backend) return;
    setLoading(true);
    setLoadError(null);
    try {
      const pk = (structure.columns ?? []).filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0)).map((c) => c.name);
      const res = await backend.loadRows(pk, lim, lim === null ? 0 : p * lim);
      setOriginal(res.rows);
      setRows(res.rows);
      setSqlText(res.sql);
      setTruncated(res.truncated);
      resetEdits();
      setPage(p);
      setActiveRow(0);
      setRefreshedAt(new Date());
    } catch (e) {
      setLoadError(errorText(toAtlasError(e)));
    } finally {
      setLoading(false);
    }
  }, [backend]);

  // Conexão real: lê a estrutura (colunas/PK) e a primeira página.
  useEffect(() => {
    if (!backend) return;
    let alive = true;
    backend
      .loadDetail()
      .then((obj) => {
        if (!alive) return;
        setTable(obj);
        return loadPage(0, obj, limit);
      })
      .catch((e) => {
        if (!alive) return;
        setLoadError(errorText(toAtlasError(e)));
        setLoading(false);
      });
    return () => {
      alive = false;
    };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const dirty = edited.size > 0 || newRows.size > 0 || deleted.size > 0;

  useEffect(() => {
    if (!active) return;
    onStatus([
      { text: sqlText, grow: true },
      { text: rows.length ? t("data.record", { n: activeRow + 1, total: rows.length, page: page + 1 }) : t("data.noRecords"), width: 220 },
      { text: t("data.lastRefresh", { when: refreshedAt.toLocaleTimeString() }), width: 240 },
    ]);
  }, [active, rows.length, activeRow, refreshedAt, sqlText, page, t, onStatus]);

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
    resetEdits();
    setActiveRow((r) => Math.min(r, Math.max(0, original.length - 1)));
  };

  /** Prévia legível para revisão. Na execução real os valores vão como parâmetros, nunca no texto. */
  const buildPreview = (): string => {
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

  const apply = async () => {
    if (backend) {
      setApplying(true);
      setApplyError(null);
      try {
        await backend.apply(plannedStatements({ schema, table: table.name, columns: cols, original, current: rows, edited, newRows, deleted }));
      } catch (e) {
        setApplyError(errorText(toAtlasError(e)));
        setApplying(false);
        return;
      }
      setApplying(false);
      setReview(null);
      await loadPage(page, table, limit);
      return;
    }
    // Demonstração: aplica só em memória.
    const kept = rows.filter((_, i) => !deleted.has(i));
    setOriginal(kept);
    setRows(kept);
    resetEdits();
    setReview(null);
    setRefreshedAt(new Date());
    setActiveRow((r) => Math.min(r, Math.max(0, kept.length - 1)));
  };

  const refresh = () => {
    if (dirty) return;
    if (backend) void loadPage(page, table, limit);
    else setRefreshedAt(new Date());
  };

  /** Novo limite: volta para a primeira página (na referência a mudança vale ao confirmar). */
  const applyLimit = (on: boolean, size: number) => {
    setLimitOn(on);
    setPageSize(size);
    setSizeText(String(size));
    if (on !== limitOn || size !== pageSize) void loadPage(0, table, on ? size : null);
  };

  const commitSize = () => {
    const n = Number(sizeText.trim());
    if (Number.isInteger(n) && n >= 1 && n <= MAX_PAGE_SIZE) applyLimit(limitOn, n);
    else setSizeText(String(pageSize));
  };

  const tool = (icon: SmallIconName, label: MessageKey, onClick?: () => void) => (
    <button type="button" className="ot-button" disabled={!onClick} onClick={onClick} title={onClick ? t(label) : t("notImplemented")}>
      <SmallIcon name={icon} />
      <span>{t(label)}</span>
    </button>
  );

  const record = rows[activeRow];
  const hasNext = !!backend && limit !== null && original.length === limit;
  const canPage = !dirty && !loading;

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

      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      {!loadError && truncated && <div className="doc-banner is-warn">{t("data.truncatedAll", { n: rows.length.toLocaleString() })}</div>}
      {!loadError && !loading && readOnly && !isView && <div className="doc-banner">{t("data.readOnlyNoKey")}</div>}
      {!readOnly && production && <div className="doc-banner is-warn">{t("data.productionBanner")}</div>}

      <div className="doc-body">
        {loading && !rows.length ? (
          <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>
        ) : view === "grid" ? (
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
            lookupCols={lookup ? lookupCols : undefined}
            onLookup={(row, col, anchor) => {
              const fk = fkByCol.get(col);
              if (fk) setFkOpen({ row, fk, anchor });
            }}
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
        <button type="button" className="icon-button" title={t("data.addRecord")} disabled={readOnly || loading} onClick={addRow}><SmallIcon name="addRow" /></button>
        <button type="button" className="icon-button" title={t("data.deleteRecord")} disabled={readOnly || !rows.length} onClick={deleteRow}><SmallIcon name="removeRow" /></button>
        <button type="button" className="icon-button" title={t("data.apply")} disabled={!dirty} onClick={() => setReview(buildPreview())}><SmallIcon name="apply" /></button>
        <button type="button" className="icon-button" title={t("data.discard")} disabled={!dirty} onClick={discard}><SmallIcon name="discard" /></button>
        <button type="button" className="icon-button" title={t("data.refresh")} disabled={dirty || loading} onClick={refresh}><SmallIcon name="refresh" /></button>
        <button type="button" className="icon-button" title={t("data.stop")} disabled><SmallIcon name="stopSmall" /></button>
        {loading && <span className="spinner" />}
        {dirty && <span className="data-pending">{t("data.pending", { n: edited.size + newRows.size + deleted.size })}</span>}
        <span className="spacer" />
        {limitPanel ? (
          <div className="data-limit">
            <label className="check-row">
              <input type="checkbox" checked={limitOn} disabled={!canPage} onChange={(e) => applyLimit(e.target.checked, pageSize)} />
              <span>{t("data.limitRecords")}</span>
            </label>
            <input
              className="field data-limit-size"
              value={sizeText}
              disabled={!canPage || !limitOn}
              inputMode="numeric"
              aria-label={t("data.recordsPerPage")}
              onChange={(e) => setSizeText(e.target.value.replace(/\D/g, ""))}
              onFocus={(e) => e.target.select()}
              onBlur={commitSize}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitSize();
                if (e.key === "Escape") setSizeText(String(pageSize));
              }}
            />
            <span>{t("data.recordsPerPage")}</span>
          </div>
        ) : (
          <>
            <button type="button" className="icon-button" disabled={!canPage || page === 0} title={t("data.firstPage")} onClick={() => void loadPage(0, table, limit)}><SmallIcon name="first" /></button>
            <button type="button" className="icon-button" disabled={!canPage || page === 0} title={t("data.prevPage")} onClick={() => void loadPage(page - 1, table, limit)}><SmallIcon name="prev" /></button>
            <input className="field page-input" value={page + 1} readOnly aria-label={t("data.page")} />
            <button type="button" className="icon-button" disabled={!canPage || !hasNext} title={t("data.nextPage")} onClick={() => void loadPage(page + 1, table, limit)}><SmallIcon name="next" /></button>
            <button type="button" className="icon-button" disabled title={t("data.lastPage")}><SmallIcon name="last" /></button>
          </>
        )}
        <button
          type="button"
          className={`icon-button${limitPanel ? " is-active" : ""}`}
          title={t("data.limitSettings")}
          aria-pressed={limitPanel}
          onClick={() => setLimitPanel((v) => !v)}
        >
          <SmallIcon name="settings" />
        </button>
        <span className="data-gap" />
        <button type="button" className={`sb-button${view === "grid" ? " is-active" : ""}`} title={t("data.gridView")} onClick={() => setView("grid")}><SmallIcon name="gridView" /></button>
        <button type="button" className={`sb-button${view === "form" ? " is-active" : ""}`} title={t("data.formView")} onClick={() => setView("form")}><SmallIcon name="formView" /></button>
      </div>

      {fkOpen && lookup && (
        <FkLookup
          fk={fkOpen.fk}
          table={table.name}
          current={fkColumnIndexes(fkOpen.fk).map((i) => (i >= 0 ? rows[fkOpen.row]?.[i] ?? null : null))}
          anchor={fkOpen.anchor}
          source={lookup}
          onPick={(values) => {
            // FK composta: preenche todas as colunas da chave; só marca as que mudaram.
            fkColumnIndexes(fkOpen.fk).forEach((ci, k) => {
              if (ci >= 0 && rows[fkOpen.row][ci] !== values[k]) onEdit(fkOpen.row, ci, values[k]);
            });
            setFkOpen(null);
          }}
          onClose={() => setFkOpen(null)}
        />
      )}

      {review !== null && (
        <MessageDialog
          title={t("data.reviewTitle")}
          message={t(backend ? "data.reviewMessageLive" : "data.reviewMessage")}
          sql={review}
          confirmText={production ? table.name : undefined}
          danger={deleted.size > 0}
          okLabel={t("data.apply")}
          busy={applying}
          error={applyError ?? undefined}
          onOk={() => void apply()}
          onCancel={() => { setApplyError(null); setReview(null); }}
        />
      )}
    </div>
  );
}
