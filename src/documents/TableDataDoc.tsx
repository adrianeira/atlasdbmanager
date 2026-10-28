import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { errorText, local, toAtlasError, type OrderBy, type PlannedStatement, type TableFilter } from "../backend";
import { CellEditorPanel } from "../components/CellEditorPanel";
import { TableProfileDialog } from "../components/TableProfileDialog";
import { ColumnsDialog } from "../components/ColumnsDialog";
import { DataGrid, typeKind, type GridColumn } from "../components/DataGrid";
import { FilterSortDialog } from "../components/FilterSortDialog";
import { FkLookup, type FkLookupSource } from "../components/FkLookup";
import { MessageDialog } from "../components/MessageDialog";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import { ColumnStatistics, DEFAULT_SCOPE, PROFILE_HEADER_HEIGHT, ProfileCell, ProfileSetup, parseSort, type ProfileScope, type Spotlight } from "../components/DataProfiling";
import { DuplicateRecordDialog } from "../components/DuplicateRecordDialog";
import { FullscreenButton } from "../components/FullscreenButton";
import { plannedStatements } from "../live";
import { mockRows, quoteIdent, type CellValue, type DbObject, type ForeignKeyDef } from "../model";
import { columnType, selectSql } from "../sql/ddl";
import { useOptions } from "../options";
import { isDefaultView, prefKey, sanitizeView } from "../tableView";
import type { DataSnapshot, SnapshotRef } from "../tabState";
import { NO_EQUALITY } from "../live";

/** Acesso ao banco real; ausente = modo de demonstração com dados fictícios. */
export interface TableDataBackend {
  /** Banco dos comandos de gravação; ausente = PostgreSQL. */
  dialect?: "sqlite" | "mssql" | "mysql" | "firebird" | "oracle";
  loadDetail: () => Promise<DbObject>;
  /** Opcional: Data Profiling (estatísticas de cada coluna sobre os registros escolhidos). */
  profileData?: (columns: string[], orderBy: OrderBy[], filter: TableFilter | null, limit: number | null) => Promise<import("../backend").DataProfile>;
  /** Opcional: perfil da tabela (botão "Table Profile"). */
  loadProfile?: () => Promise<import("../backend").TableProfile>;
  /** `limit = null`: sem LIMIT ("Limitar registros" desmarcado). */
  loadRows: (orderBy: OrderBy[], filter: TableFilter | null, limit: number | null, offset: number) => Promise<{ sql: string; rows: CellValue[][]; truncated: boolean }>;
  /** Devolve as linhas afetadas por comando (quando o servidor informa). */
  apply: (statements: PlannedStatement[]) => Promise<number[] | void>;
  /** Opcional: cancela a consulta em andamento no servidor (botão "Parar"). */
  cancel?: () => Promise<void>;
  /** Opcional: total de registros (respeitando o filtro) para a navegação de páginas. */
  count?: (filter: TableFilter | null) => Promise<number>;
  /** Opcional: transação manual. Depois de `beginTx`, apply/loadRows/count passam a usar a transação aberta. */
  beginTx?: () => Promise<void>;
  endTx?: (commit: boolean) => Promise<void>;
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
  /** Há alterações não gravadas (a aba pergunta antes de fechar). */
  onDirty?: (dirty: boolean) => void;
  /** Chave da tabela (conexão/banco/schema/nome): com ela a aba lembra ordenação, filtro, colunas e larguras entre aberturas. */
  viewKey?: string;
  /** Estado guardado de uma tela anterior (página, linha, rolagem...) para reconstruir esta igual. */
  initialState?: unknown;
  /** Recebe uma função que devolve o estado atual da tela. */
  stateRef?: SnapshotRef;
  /** Abre o assistente de exportação com o filtro, a ordenação e as colunas visíveis do grid. Só com banco real. */
  onExport?: (scope: { orderBy: OrderBy[]; filter: TableFilter | null; allColumns: string[]; visibleColumns: string[] }) => void;
}

const MAX_PAGE_SIZE = 1_000_000;

export function TableDataDoc({ schema, table: tableProp, active, production, onStatus, backend, lookup, isView, onDirty, viewKey, initialState, stateRef, onExport }: Props) {
  const t = useT();
  const { pageSize: DEFAULT_PAGE_SIZE, limitRecords, autoBeginTransaction } = useOptions();
  const [table, setTable] = useState<DbObject>(tableProp);
  const cols = useMemo(() => table.columns ?? [], [table]);
  const keyCols = cols.map((c, i) => ({ c, i })).filter((x) => x.c.key).sort((a, b) => (a.c.key ?? 0) - (b.c.key ?? 0));
  // Sem chave primária a tabela ainda edita: a linha é achada por todos os valores e linhas iguais são todas afetadas (aviso na tela).
  // Só fica somente leitura se nenhuma coluna tem igualdade para servir de condição (ex.: só `json`).
  const keyless = !isView && keyCols.length === 0;
  const keylessUsable = cols.some((c) => !NO_EQUALITY.has(c.type));
  const readOnly = isView || (keyless && !keylessUsable);

  // "Columns" da toolbar: colunas ocultas do grid, por nome (os dados continuam carregados).
  const [hiddenCols, setHiddenCols] = useState<Set<string>>(new Set());
  const [columnsOpen, setColumnsOpen] = useState(false);
  /** Larguras ajustadas pelo usuário, por nome de coluna. */
  const [widths, setWidths] = useState<Record<string, number>>({});
  /** Ordem das colunas depois de arrastar os cabeçalhos (vazia = a da tabela). */
  const [colOrder, setColOrder] = useState<string[]>([]);

  const columns: GridColumn[] = useMemo(
    () => cols.map((c) => ({
      name: c.name,
      type: columnType(c),
      kind: typeKind(c.type),
      isKey: !!c.key,
      required: c.notNull,
      comment: c.comment || undefined,
      // A identidade `BY DEFAULT` (e o AUTO_INCREMENT) numera sozinha quando o valor é omitido: não é campo obrigatório.
      hasDefault: !!c.default || c.identity === "d",
      // Bloqueia edição em colunas geradas pelo banco: GENERATED ALWAYS AS IDENTITY ou STORED.
      generated: c.identity === "a" || c.generated === "s",
      hidden: hiddenCols.has(c.name),
      width: widths[c.name],
    })),
    [cols, hiddenCols, widths],
  );

  const [original, setOriginal] = useState<CellValue[][]>(() => (backend ? [] : mockRows(tableProp)));
  const [rows, setRows] = useState<CellValue[][]>(original);
  const [edited, setEdited] = useState<Set<string>>(new Set());
  const [newRows, setNewRows] = useState<Set<number>>(new Set());
  const [deleted, setDeleted] = useState<Set<number>>(new Set());
  const [activeRow, setActiveRow] = useState(0);
  const restored = (initialState as DataSnapshot | undefined)?.kind === "data" ? (initialState as DataSnapshot) : undefined;
  const [view, setView] = useState<"grid" | "form">(restored?.view ?? "grid");
  const scrollTopRef = useRef(restored?.scrollTop ?? 0);
  const [refreshedAt, setRefreshedAt] = useState(() => new Date());
  /** Confirmação do Aplicar (há registros a excluir ou a conexão é de produção). */
  const [confirmApply, setConfirmApply] = useState(false);
  const [page, setPage] = useState(0);
  const [loading, setLoading] = useState(!!backend);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  /** Data Profiling: fechado, escolhendo o que perfilar ou mostrando as estatísticas. */
  const [profMode, setProfMode] = useState<"off" | "setup" | "on">("off");
  const [profScope, setProfScope] = useState<ProfileScope>(DEFAULT_SCOPE);
  const [prof, setProf] = useState<import("../backend").DataProfile | null>(null);
  const [profBusy, setProfBusy] = useState(false);
  const [profError, setProfError] = useState<string | null>(null);
  const [spot, setSpot] = useState<Spotlight | null>(null);
  const startProfiling = async () => {
    if (!backend?.profileData) return;
    setProfError(null);
    let filter: TableFilter | null = null;
    let order: OrderBy[] = [];
    let limit: number | null = null;
    if (profScope.mode === "filter") {
      filter = profScope.filterText.trim() ? { expr: profScope.filterText } : null;
      const parsed = parseSort(profScope.sortText, cols.map((c) => c.name));
      if (parsed.bad) return setProfError(t("pf.badSort", { name: parsed.bad }));
      order = parsed.sort;
      if (profScope.limited) {
        limit = Number(profScope.limit);
        if (!Number.isInteger(limit) || limit < 1) return setProfError(t("pf.badLimit"));
      }
    }
    setProfBusy(true);
    try {
      setProf(await backend.profileData(cols.map((c) => c.name), order, filter, limit));
      setSpot(null);
      setProfMode("on");
    } catch (e) {
      setProfError(errorText(toAtlasError(e)));
    } finally {
      setProfBusy(false);
    }
  };
  const toggleSpot = (column: string, value: string | null, additive: boolean) => setSpot((cur) => {
    const base = cur && cur.column === column ? new Set(cur.values) : new Set<string | null>();
    if (additive) { if (base.has(value)) base.delete(value); else base.add(value); }
    else if (base.size === 1 && base.has(value)) base.clear();
    else { base.clear(); base.add(value); }
    return base.size ? { column, values: base } : null;
  });
  /** Pula para a próxima (ou anterior) linha com o valor em destaque. */
  const stepSpot = (dir: 1 | -1) => {
    if (!spot) return;
    const ci = cols.findIndex((c) => c.name === spot.column);
    const n = rows.length;
    for (let k = 1; k <= n; k++) {
      const r = (activeRow + dir * k + n * k) % n;
      if (spot.values.has(rows[r][ci])) return setActiveRow(r);
    }
  };
  /** Linha que está sendo duplicada (janela "Duplicar registro"). */
  const [dupRow, setDupRow] = useState<number | null>(null);
  // Transação manual (botão "Iniciar transação"): alterações aplicadas só valem de vez ao confirmar.
  const [inTx, setInTx] = useState(false);
  const [txBusy, setTxBusy] = useState(false);
  const inTxRef = useRef(false);
  inTxRef.current = inTx;
  const [sqlText, setSqlText] = useState(() => selectSql(schema, tableProp.name, DEFAULT_PAGE_SIZE, 0));
  // Engrenagem do rodapé (como na referência): troca a paginação por "Limitar registros".
  const [limitPanel, setLimitPanel] = useState(false);
  const [limitOn, setLimitOn] = useState(limitRecords);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [sizeText, setSizeText] = useState(String(DEFAULT_PAGE_SIZE));
  const [truncated, setTruncated] = useState(false);
  const limit = limitOn ? pageSize : null;
  const [fkOpen, setFkOpen] = useState<{ row: number; fk: ForeignKeyDef; anchor: DOMRect } | null>(null);

  // Barra "Localizar dados" (Ctrl+F): filtra em todas as colunas das linhas carregadas.
  const [findOpen, setFindOpen] = useState(restored?.findOpen ?? false);
  const [findQuery, setFindQuery] = useState(restored?.findQuery ?? "");
  const [findIndex, setFindIndex] = useState(0);
  const [highlightAll, setHighlightAll] = useState(true);
  const [replaceMode, setReplaceMode] = useState(false);
  const [replaceText, setReplaceText] = useState("");
  const findInputRef = useRef<HTMLInputElement>(null);

  const findMatches = useMemo(() => {
    if (!findOpen || !findQuery) return [] as Array<{ row: number; col: number }>;
    const q = findQuery.toLowerCase();
    const out: Array<{ row: number; col: number }> = [];
    for (let r = 0; r < rows.length; r++) {
      const row = rows[r];
      for (let c = 0; c < row.length; c++) {
        const v = row[c];
        if (v !== null && v !== undefined && columns[c]?.kind !== "bool" && String(v).toLowerCase().includes(q)) {
          out.push({ row: r, col: c });
        }
      }
    }
    return out;
  }, [findOpen, findQuery, rows, columns]);

  useEffect(() => {
    if (findIndex >= findMatches.length) setFindIndex(0);
  }, [findMatches, findIndex]);

  const activeFind = findMatches[findIndex] ?? null;
  const stepFind = (delta: number) => {
    if (!findMatches.length) return;
    setFindIndex((i) => (i + delta + findMatches.length) % findMatches.length);
  };
  const closeFind = () => { setFindOpen(false); setFindQuery(""); setReplaceMode(false); setReplaceText(""); };

  // Célula "substituível": edição permitida e tipo compatível com busca em texto.
  const cellReplaceable = (r: number, c: number): boolean => {
    if (readOnly || deleted.has(r)) return false;
    const kind = columns[c]?.kind;
    return kind !== "bool" && kind !== "binary";
  };
  const applyReplace = (r: number, c: number) => {
    const v = rows[r]?.[c];
    if (v === null || v === undefined || !findQuery || !cellReplaceable(r, c)) return false;
    // Primeira ocorrência (case-insensitive), preservando o resto do texto.
    const s = String(v);
    const i = s.toLowerCase().indexOf(findQuery.toLowerCase());
    if (i < 0) return false;
    onEdit(r, c, s.slice(0, i) + replaceText + s.slice(i + findQuery.length));
    return true;
  };
  const replaceCurrent = () => {
    if (!activeFind) return;
    if (applyReplace(activeFind.row, activeFind.col)) {
      // Avança para a próxima ocorrência.
      stepFind(1);
    }
  };
  const replaceAll = () => {
    if (!findMatches.length) return;
    // Trabalha num snapshot: cada match é uma célula distinta, cada edição é independente.
    for (const m of findMatches) applyReplace(m.row, m.col);
  };

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

  // "Valor da FK": mostra a descrição da linha referenciada no lugar da chave (só chaves de uma coluna; só a exibição muda).
  const [profileOpen, setProfileOpen] = useState(false);
  const [fkValues, setFkValues] = useState(() => {
    try { return localStorage.getItem("atlas.fkValues") === "1"; } catch { return false; }
  });
  const toggleFkValues = () => setFkValues((on) => {
    try { localStorage.setItem("atlas.fkValues", on ? "0" : "1"); } catch { /* sem armazenamento: vale só nesta sessão */ }
    return !on;
  });
  const [valueLabels, setValueLabels] = useState<Map<number, Map<string, string>>>(new Map());
  /** Rótulos já lidos por FK (nome da FK → chave → texto), para não repetir a consulta ao paginar. */
  const labelCache = useRef(new Map<string, { column: string | null; map: Map<string, string>; asked: Set<string> }>());
  const canLabel = !!lookup?.labels && [...fkByCol.values()].some((fk) => !fk.fields.includes(","));
  useEffect(() => {
    const labelsOf = lookup?.labels;
    if (!fkValues || !labelsOf) return void setValueLabels((m) => (m.size ? new Map() : m));
    let alive = true;
    void (async () => {
      const next = new Map<number, Map<string, string>>();
      for (const [ci, fk] of fkByCol) {
        if (fk.fields.includes(",")) continue;
        const cacheKey = `${fk.refSchema}.${fk.refTable}.${fk.refFields}`;
        const entry = labelCache.current.get(cacheKey) ?? { column: null, map: new Map<string, string>(), asked: new Set<string>() };
        labelCache.current.set(cacheKey, entry);
        const wanted = [...new Set(rows.map((r) => r[ci]).filter((v): v is string => v !== null && v !== undefined && !entry.asked.has(v)))];
        if (wanted.length) {
          try {
            const got = await labelsOf(fk.refSchema, fk.refTable, fk.refFields.trim(), wanted);
            entry.column = got.labelColumn;
            for (const [k, label] of got.labels) entry.map.set(k, label);
            for (const k of wanted) entry.asked.add(k);
          } catch {
            // Sem permissão na tabela referenciada: a coluna continua mostrando a chave.
          }
        }
        if (entry.map.size) next.set(ci, entry.map);
      }
      if (alive) setValueLabels(next);
    })();
    return () => { alive = false; };
  }, [fkValues, rows, fkByCol, lookup]);
  const fkColumnIndexes = (fk: ForeignKeyDef) => fk.fields.split(",").map((f) => cols.findIndex((c) => c.name === f.trim()));

  const resetEdits = () => {
    setEdited(new Set());
    setNewRows(new Set());
    setDeleted(new Set());
  };

  // Total conhecido (depois de "Última página" ou refresh). null = desconhecido.
  const [totalRows, setTotalRows] = useState<number | null>(null);
  // Texto editável do campo "página", separado do estado real para permitir digitação.
  const [pageText, setPageText] = useState("1");
  // Filter & Sort (botão da toolbar). Vazios = ordenação pela PK, sem WHERE.
  const [sort, setSort] = useState<OrderBy[]>([]);
  const [filter, setFilter] = useState<TableFilter | null>(null);
  const [filterOpen, setFilterOpen] = useState(false);
  // Coluna a pré-selecionar ao abrir via "Add Filter" no menu "⋯" do cabeçalho.
  const [filterColumn, setFilterColumn] = useState<string | undefined>(undefined);
  // Painel "Cell Editor": coluna ativa é relatada pelo grid (linha já é lifted via activeRow).
  const [cellEditorOpen, setCellEditorOpen] = useState(false);
  const [activeCol, setActiveCol] = useState(0);

  /**
   * `keepRow`: linha a manter selecionada após reler (ex.: depois de gravar um registro novo).
   * `overrideSort`/`overrideFilter`: usados quando o chamador acabou de mudar `sort`/`filter` e
   * não pode esperar o próximo render — `setState` é assíncrono, então o fechamento (closure)
   * desta função só veria o valor novo depois; passando-o explicitamente evita aplicar o valor
   * antigo (bug observado: clicar "Crescente" ordenava pelo sort anterior, não o novo).
   */
  const loadPage = useCallback(async (
    p: number, structure: DbObject, lim: number | null, keepRow?: number,
    overrideSort?: OrderBy[], overrideFilter?: TableFilter | null,
  ) => {
    if (!backend) return;
    setLoading(true);
    setLoadError(null);
    try {
      const pk = (structure.columns ?? []).filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0)).map((c) => c.name);
      const effectiveSort = overrideSort ?? sort;
      const effectiveFilter = overrideFilter !== undefined ? overrideFilter : filter;
      const orderBy: OrderBy[] = effectiveSort.length ? effectiveSort : pk.map((c) => ({ column: c, descending: false }));
      const res = await backend.loadRows(orderBy, effectiveFilter, lim, lim === null ? 0 : p * lim);
      setOriginal(res.rows);
      setRows(res.rows);
      setSqlText(res.sql);
      setTruncated(res.truncated);
      resetEdits();
      setPage(p);
      setPageText(String(p + 1));
      setActiveRow(Math.max(0, Math.min(keepRow ?? 0, res.rows.length - 1)));
      setRefreshedAt(new Date());
      return true;
    } catch (e) {
      setLoadError(errorText(toAtlasError(e)));
      return false;
    } finally {
      setLoading(false);
    }
  }, [backend, sort, filter]);

  /** Só grava a visão depois de lida a anterior (senão a padrão a apagaria). */
  const [viewLoaded, setViewLoaded] = useState(false);
  // Grava a visão (com uma pausa, para não gravar a cada pixel arrastado).
  useEffect(() => {
    if (!viewKey || !backend || !viewLoaded) return;
    const timer = window.setTimeout(() => {
      const state = { sort, filter: filter?.expr ?? "", hidden: [...hiddenCols], order: colOrder, widths, limitOn, pageSize };
      void local.setPref(prefKey(viewKey), isDefaultView(state, { limitOn: limitRecords, pageSize: DEFAULT_PAGE_SIZE }) ? null : state).catch(() => undefined);
    }, 500);
    return () => window.clearTimeout(timer);
  }, [viewKey, backend, viewLoaded, sort, filter, hiddenCols, colOrder, widths, limitOn, pageSize, limitRecords, DEFAULT_PAGE_SIZE]);

  // Conexão real: lê a estrutura (colunas/PK) e a primeira página.
  useEffect(() => {
    if (!backend) return;
    let alive = true;
    backend
      .loadDetail()
      .then(async (obj) => {
        if (!alive) return;
        // A visão que o usuário deixou da última vez (ordenação, filtro, colunas, larguras, limite).
        let saved: ReturnType<typeof sanitizeView> | null = null;
        if (viewKey) {
          try {
            const raw = await local.getPref<unknown>(prefKey(viewKey));
            if (raw) saved = sanitizeView(raw, (obj.columns ?? []).map((c) => c.name), { limitOn, pageSize });
          } catch { /* sem preferência: abre com a visão padrão */ }
        }
        if (!alive) return;
        if (!saved) {
          setTable(obj);
          setViewLoaded(true);
          return loadPage(restored?.page ?? 0, obj, limit, restored?.activeRow);
        }
        const savedFilter = saved.filter.trim() ? { expr: saved.filter } : null;
        // As larguras entram junto com as colunas (o grid só as lê quando a estrutura muda).
        setWidths(saved.widths);
        setTable(obj);
        setSort(saved.sort);
        setFilter(savedFilter);
        setHiddenCols(new Set(saved.hidden));
        setColOrder(saved.order);
        setLimitOn(saved.limitOn);
        setPageSize(saved.pageSize);
        setSizeText(String(saved.pageSize));
        const ok = await loadPage(restored?.page ?? 0, obj, saved.limitOn ? saved.pageSize : null, restored?.activeRow, saved.sort, savedFilter);
        // O filtro lembrado pode não valer mais (coluna renomeada, tipo mudado): volta à visão padrão em vez de ficar com erro.
        if (!ok && alive && (saved.sort.length || savedFilter)) {
          setSort([]);
          setFilter(null);
          await loadPage(0, obj, saved.limitOn ? saved.pageSize : null, undefined, [], null);
        }
        setViewLoaded(true);
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

  if (stateRef) {
    stateRef.current = () => ({ kind: "data", page, activeRow, scrollTop: scrollTopRef.current, view, findOpen, findQuery });
  }
  const dirty = edited.size > 0 || newRows.size > 0 || deleted.size > 0;
  useEffect(() => onDirty?.(dirty || inTx), [dirty, inTx, onDirty]);
  // Fechar a aba (ou trocar de tabela) com transação aberta desfaz tudo.
  useEffect(() => () => { if (inTxRef.current) void backend?.endTx?.(false).catch(() => undefined); }, [backend]);

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
    if (newRows.has(r)) return;
    // Voltou ao valor original (ex.: checkbox clicado duas vezes): deixa de ser alteração.
    setEdited((s) => {
      const n = new Set(s);
      if (original[r]?.[c] === v) n.delete(`${r}:${c}`);
      else n.add(`${r}:${c}`);
      return n;
    });
  };

  const addRow = () => {
    if (readOnly) return;
    const idx = rows.length;
    setRows((rs) => [...rs, cols.map(() => null)]);
    setNewRows((s) => new Set(s).add(idx));
    setActiveRow(idx);
  };

  const deleteRow = (row = activeRow) => {
    if (readOnly || !rows.length) return;
    if (newRows.has(row)) {
      setRows((rs) => rs.filter((_, i) => i !== row));
      setNewRows((s) => new Set([...s].filter((i) => i !== row).map((i) => (i > row ? i - 1 : i))));
      return;
    }
    setDeleted((s) => {
      const n = new Set(s);
      if (n.has(row)) n.delete(row);
      else n.add(row);
      return n;
    });
  };

  const discard = () => {
    setRows(original);
    resetEdits();
    setActiveRow((r) => Math.min(r, Math.max(0, original.length - 1)));
  };

  const apply = async () => {
    if (backend) {
      setApplying(true);
      setApplyError(null);
      try {
        const planned = plannedStatements({ dialect: backend.dialect, schema, table: table.name, columns: cols, original, current: rows, edited, newRows, deleted });
        await backend.apply(planned);
      } catch (e) {
        setApplyError(errorText(toAtlasError(e)));
        setApplying(false);
        return;
      }
      setApplying(false);
      setConfirmApply(false);
      await loadPage(page, table, limit);
      return;
    }
    // Demonstração: aplica só em memória.
    const kept = rows.filter((_, i) => !deleted.has(i));
    setOriginal(kept);
    setRows(kept);
    resetEdits();
    setConfirmApply(false);
    setRefreshedAt(new Date());
    setActiveRow((r) => Math.min(r, Math.max(0, kept.length - 1)));
  };

  /* ----- Gravação automática ao mudar de registro (como a referência) ----- */
  // Produção continua só pelo Aplicar (com revisão); exclusões também só pelo Aplicar.
  // Sem chave não grava ao sair da linha: o usuário aplica de propósito, sabendo que linhas iguais são todas afetadas.
  const autoSave = !production && !readOnly && !keyless;
  const rowPending = (r: number) => newRows.has(r) || [...edited].some((k) => k.startsWith(`${r}:`));
  const savingRow = useRef(false);

  /** Grava só as alterações da linha `r`. Devolve false se o banco recusar (erro fica na tela). */
  const saveRow = async (r: number, nextRow: number): Promise<boolean> => {
    const rowEdited = new Set([...edited].filter((k) => k.startsWith(`${r}:`)));
    const isNew = newRows.has(r);
    if ((!rowEdited.size && !isNew) || deleted.has(r)) return true;
    if (backend) {
      setApplying(true);
      try {
        await backend.apply(plannedStatements({
          dialect: backend.dialect, schema, table: table.name, columns: cols, original, current: rows,
          edited: rowEdited, newRows: new Set(isNew ? [r] : []), deleted: new Set(),
        }));
      } catch (e) {
        setApplyError(errorText(toAtlasError(e)));
        setApplying(false);
        return false;
      }
      setApplying(false);
      // Registro novo: relê a página para trazer os valores do servidor (DEFAULT, serial...),
      // desde que não haja outras alterações pendentes que a releitura descartaria.
      const othersPending = deleted.size > 0 || [...newRows].some((x) => x !== r) || [...edited].some((k) => !k.startsWith(`${r}:`));
      if (isNew && !othersPending) {
        await loadPage(page, table, limit, nextRow);
        return true;
      }
    }
    // A linha gravada passa a ser a base das próximas comparações.
    setOriginal((o) => {
      const copy = [...o];
      copy[r] = rows[r];
      return copy;
    });
    setEdited((s) => new Set([...s].filter((k) => !k.startsWith(`${r}:`))));
    setNewRows((s) => new Set([...s].filter((x) => x !== r)));
    setRefreshedAt(new Date());
    return true;
  };

  // Mudou de linha com a anterior alterada: grava a anterior; se o banco recusar, volta para ela.
  const prevRow = useRef(activeRow);
  useEffect(() => {
    const prev = prevRow.current;
    prevRow.current = activeRow;
    if (!autoSave || prev === activeRow || savingRow.current || !rowPending(prev)) return;
    savingRow.current = true;
    void saveRow(prev, activeRow).then((ok) => {
      savingRow.current = false;
      if (!ok) {
        prevRow.current = prev;
        setActiveRow(prev);
      }
    });
    // Reage só à troca de linha; as demais dependências são lidas no momento.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeRow]);

  // Saiu da aba (outra aba ativa): grava a linha em edição.
  useEffect(() => {
    if (active || !autoSave || savingRow.current || !rowPending(activeRow)) return;
    savingRow.current = true;
    void saveRow(activeRow, activeRow).finally(() => { savingRow.current = false; });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  /** Registros existentes com alguma célula alterada (para o resumo da confirmação). */
  const updatedRows = new Set([...edited].map((k) => Number(k.split(":")[0])).filter((r) => !deleted.has(r))).size;

  /**
   * Aplicar: grava direto, como a referência. Só pede confirmação quando há registros
   * marcados para exclusão ou a conexão é de produção.
   */
  const requestApply = () => {
    if (!dirty || applying) return;
    if (deleted.size > 0 || production) setConfirmApply(true);
    else void apply();
  };

  const beginTx = async () => {
    if (!backend?.beginTx || inTx || txBusy) return;
    setTxBusy(true);
    try {
      await backend.beginTx();
      setInTx(true);
    } catch (e) {
      setApplyError(errorText(toAtlasError(e)));
    } finally {
      setTxBusy(false);
    }
  };

  // Opções → Registros → Iniciar transação automaticamente: a tabela já abre dentro de uma transação (uma vez por aba).
  const autoBegun = useRef(false);
  useEffect(() => {
    if (autoBegun.current || !autoBeginTransaction || readOnly || !backend?.beginTx) return;
    autoBegun.current = true;
    void beginTx();
    // `beginTx` lê o estado atual; as colunas (e com elas `readOnly`) podem chegar depois da abertura.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoBeginTransaction, readOnly, backend]);

  /** Confirma (COMMIT) ou desfaz (ROLLBACK) a transação e relê a página. */
  const endTx = async (commit: boolean) => {
    if (!backend?.endTx || !inTx || txBusy || (commit && dirty)) return;
    setTxBusy(true);
    try {
      await backend.endTx(commit);
      setInTx(false);
      setTotalRows(null);
      await loadPage(page, table, limit);
    } catch (e) {
      setInTx(false); // o servidor já encerrou a transação (ex.: COMMIT recusado)
      setApplyError(errorText(toAtlasError(e)));
    } finally {
      setTxBusy(false);
    }
  };

  const refresh = () => {
    if (dirty) return;
    setTotalRows(null); // relê a contagem na próxima vez que precisar
    if (backend) {
      // Relê também a estrutura: uma coluna criada, renomeada ou removida depois de abrir a tabela passa a valer.
      const shape = (t: DbObject) => JSON.stringify((t.columns ?? []).map((c) => [c.name, c.type, c.key, c.identity, c.generated]));
      void backend
        .loadDetail()
        .then((obj) => {
          const next = shape(obj) === shape(table) ? table : obj;
          if (next !== table) setTable(obj);
          return loadPage(page, next, limit);
        })
        .catch(() => loadPage(page, table, limit));
    } else setRefreshedAt(new Date());
  };

  // "Parar" (botão ⏹): cancela a consulta em andamento no servidor.
  const stop = () => {
    if (!backend?.cancel) return;
    void backend.cancel().catch(() => undefined);
  };

  // "Última página": busca o total (uma vez) e salta para a última página do limite atual.
  const goLast = async () => {
    if (!backend || limit === null || !canPage) return;
    try {
      let total = totalRows;
      if (total === null && backend.count) {
        total = await backend.count(filter);
        setTotalRows(total);
      }
      if (total === null) return;
      const last = Math.max(0, Math.ceil(total / limit) - 1);
      await loadPage(last, table, limit);
    } catch (e) {
      setLoadError(errorText(toAtlasError(e)));
    }
  };

  // Aplicação do diálogo Filter & Sort: zera a página e o total conhecido.
  const applyFilterSort = (newFilter: TableFilter | null, newSort: OrderBy[]) => {
    setFilter(newFilter);
    setSort(newSort);
    setTotalRows(null);
    // Passa os valores novos direto (não espera o próximo render) — ver comentário em loadPage.
    if (backend) void loadPage(0, table, limit, undefined, newSort, newFilter);
  };

  // Menu "⋯" do cabeçalho: ordenar só por aquela coluna (substitui a ordenação atual).
  /** Exportar: mesmo filtro e ordem do grid (sem ordenação escolhida, pela chave primária, como a paginação). */
  const openExport = () =>
    onExport?.({
      orderBy: sort.length ? sort : keyCols.map(({ c }) => ({ column: c.name, descending: false })),
      filter,
      allColumns: cols.map((c) => c.name),
      visibleColumns: [...cols].sort((a, b) => (colOrder.indexOf(a.name) + 1 || 1e9) - (colOrder.indexOf(b.name) + 1 || 1e9)).filter((c) => !hiddenCols.has(c.name)).map((c) => c.name),
    });

  const sortByColumn = (col: number, descending: boolean) => {
    const name = cols[col]?.name;
    if (!name) return;
    applyFilterSort(filter, [{ column: name, descending }]);
  };
  // "Remove All Sorts": volta à ordenação padrão (pela chave primária).
  const clearSort = () => applyFilterSort(filter, []);
  // "Restaurar layout padrão": ordem, largura e visibilidade das colunas e a ordenação das linhas voltam ao padrão (o filtro fica).
  const resetLayout = () => {
    setColOrder([]);
    setWidths({});
    setHiddenCols(new Set());
    if (sort.length) applyFilterSort(filter, []);
  };
  // "Add Filter": abre o diálogo já com uma regra para esta coluna.
  const addFilterForColumn = (col: number) => {
    setFilterColumn(cols[col]?.name);
    setFilterOpen(true);
  };

  // Enter no campo de página: valida e navega.
  const commitPage = () => {
    const n = Number(pageText.trim());
    if (!Number.isInteger(n) || n < 1 || !canPage) { setPageText(String(page + 1)); return; }
    const target = n - 1;
    if (target === page) { setPageText(String(page + 1)); return; }
    void loadPage(target, table, limit);
  };

  // F5 (como na referência) relê só os registros desta tabela; o recarregamento da janela é bloqueado no App.
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (document.querySelector(".modal-overlay")) return;
      if (e.key === "F5" && !e.altKey) {
        e.preventDefault();
        refreshRef.current();
        return;
      }
      // Ctrl+F (ou Cmd+F no macOS): abre a barra "Localizar dados" e foca o input.
      if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        setFindOpen(true);
        requestAnimationFrame(() => findInputRef.current?.select());
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active]);

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

  const tool = (icon: SmallIconName, label: MessageKey, onClick?: () => void, active?: boolean) => (
    <button type="button" className={`ot-button${active ? " is-active" : ""}`} disabled={!onClick} onClick={onClick} title={onClick ? t(label) : t("notImplemented")}>
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
        {tool("table", "data.tableProfile", backend?.loadProfile ? () => setProfileOpen(true) : undefined)}
        {tool("transaction", "data.beginTransaction", backend?.beginTx && !readOnly && !inTx && !txBusy ? () => void beginTx() : undefined, inTx)}
        {inTx && (
          <>
            <button type="button" className="ot-button" disabled={txBusy || dirty} onClick={() => void endTx(true)} title={dirty ? t("data.commitBlocked") : t("data.commit")}>
              <SmallIcon name="apply" />
              <span>{t("data.commit")}</span>
            </button>
            <button type="button" className="ot-button" disabled={txBusy} onClick={() => void endTx(false)} title={t("data.rollback")}>
              <SmallIcon name="discard" />
              <span>{t("data.rollback")}</span>
            </button>
          </>
        )}
        {canLabel && tool("link", "data.fkValues", toggleFkValues, fkValues)}
        {tool("cellEditor", "data.cellEditor", () => setCellEditorOpen((v) => !v), cellEditorOpen)}
        {tool("filterSort", "data.filterSort", () => { setFilterColumn(undefined); setFilterOpen(true); }, !!filter || sort.length > 0)}
        {tool("columns", "data.columns", () => setColumnsOpen(true))}
        {tool("profiling", "data.profiling", backend?.profileData ? () => setProfMode((m) => (m === "off" ? "setup" : "off")) : undefined, profMode !== "off")}
        {tool("import", "data.import")}
        {tool("export", "data.export", onExport ? openExport : undefined)}
        {tool("dataGen", "data.generation")}
        <div className="ot-spacer" />
        <FullscreenButton />
      </div>

      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      {!loadError && truncated && <div className="doc-banner is-warn">{t("data.truncatedAll", { n: rows.length.toLocaleString() })}</div>}
      {!loadError && !loading && readOnly && !isView && <div className="doc-banner">{t("data.readOnlyNoKey")}</div>}
      {!loadError && !loading && keyless && !readOnly && <div className="doc-banner is-warn">{t("data.noKeyNote")}</div>}
      {!readOnly && production && <div className="doc-banner is-warn">{t("data.productionBanner")}</div>}

      {profMode === "setup" && <ProfileSetup columns={cols} scope={profScope} onScope={setProfScope} busy={profBusy} error={profError} onStart={() => void startProfiling()} />}
      {profMode === "on" && prof && (
        <div className="pf-toolbar">
          <strong>{t("data.profiling")}</strong>
          <span className="muted">{t("pf.records2", { n: prof.rows })}</span>
          <span className="spacer" />
          {spot && (
            <>
              <span>{t("pf.spotlightedValue")}</span>
              <button type="button" className="icon-button" title={t("pf.prevSpot")} onClick={() => stepSpot(-1)}>▲</button>
              <button type="button" className="icon-button" title={t("pf.nextSpot")} onClick={() => stepSpot(1)}>▼</button>
            </>
          )}
          <button type="button" className="btn" onClick={() => { setProfMode("off"); setSpot(null); }}>{t("pf.exit")}</button>
        </div>
      )}
      <div className="doc-body">
        {loading && !rows.length ? (
          <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>
        ) : view === "grid" ? (
          <DataGrid
            columns={columns}
            onColumnWidth={(name, w) => setWidths((ws) => ({ ...ws, [name]: w }))}
            profile={profMode === "on" && prof ? { height: PROFILE_HEADER_HEIGHT, render: (ci) => { const p = prof.columns.find((x) => x.name === cols[ci]?.name); return p ? <ProfileCell col={p} spotlight={spot} /> : null; } } : undefined}
            spotlight={spot ? { col: cols.findIndex((c) => c.name === spot.column), values: spot.values } : undefined}
            onScrollTop={(top) => { scrollTopRef.current = top; }}
            initialScrollTop={restored?.scrollTop}
            columnOrder={colOrder}
            onColumnOrder={setColOrder}
            rows={rows}
            readOnly={readOnly}
            edited={edited}
            deletedRows={deleted}
            newRows={newRows}
            activeRow={activeRow}
            onActiveRow={setActiveRow}
            onEdit={onEdit}
            sqlTarget={`${quoteIdent(schema)}.${quoteIdent(table.name)}`}
            onDeleteRow={readOnly ? undefined : (r) => { setActiveRow(r); deleteRow(r); }}
            onDuplicateRow={readOnly || !backend || dirty || newRows.has(0) ? undefined : (r) => setDupRow(r)}
            onRefresh={dirty || loading ? undefined : refresh}
            lookupCols={lookup ? lookupCols : undefined}
            valueLabels={fkValues ? valueLabels : undefined}
            onLookup={(row, col, anchor) => {
              const fk = fkByCol.get(col);
              if (fk) setFkOpen({ row, fk, anchor });
            }}
            findQuery={findOpen ? findQuery : ""}
            activeFind={findOpen ? activeFind : null}
            highlightAll={highlightAll}
            onSaveAs={onExport ? openExport : undefined}
            onSortColumn={sortByColumn}
            onClearSort={clearSort}
            onResetLayout={resetLayout}
            onAddFilter={addFilterForColumn}
            onActiveCell={(_, c) => setActiveCol(c)}
            onOpenCellEditor={(r, c) => { setActiveRow(r); setActiveCol(c); setCellEditorOpen(true); }}
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
                  readOnly={readOnly || c.identity === "a" || c.generated === "s"}
                  title={c.identity === "a" || c.generated === "s" ? t("grid.generated", { name: c.name }) : c.notNull ? t("grid.required", { name: c.name }) : undefined}
                  // Obrigatório (NOT NULL) não pode ficar em branco, como no grid.
                  onChange={(e) => { if (!(c.notNull && e.target.value === "")) onEdit(activeRow, i, e.target.value); }}
                />
                <span className="muted">{columnType(c)}</span>
              </div>
            )) : <div className="muted">{t("data.noRecords")}</div>}
          </div>
        )}
      </div>

      {profileOpen && backend?.loadProfile && <TableProfileDialog table={table.name} dialect={backend.dialect === "mysql" || backend.dialect === "firebird" || backend.dialect === "oracle" ? undefined : backend.dialect} load={backend.loadProfile} onClose={() => setProfileOpen(false)} />}

      {cellEditorOpen && view === "grid" && cols[activeCol] && (
        <CellEditorPanel
          row={activeRow}
          col={activeCol}
          columnName={cols[activeCol].name}
          columnType={columns[activeCol]?.type ?? ""}
          kind={columns[activeCol]?.kind ?? "text"}
          value={rows[activeRow]?.[activeCol] ?? null}
          readOnly={readOnly || !!columns[activeCol]?.generated || deleted.has(activeRow) || !rows.length}
          required={!!columns[activeCol]?.required}
          onChange={(v) => onEdit(activeRow, activeCol, v)}
          onClose={() => setCellEditorOpen(false)}
        />
      )}

      {findOpen && (
        <div className="data-find-stack">
          <div className="data-find" role="search">
            <button type="button" className="find-close" title={t("data.find.close")} aria-label={t("data.find.close")} onClick={closeFind}>✕</button>
            <input
              ref={findInputRef}
              type="search"
              className="field find-input"
              placeholder={t("data.find.placeholder")}
              value={findQuery}
              onChange={(e) => { setFindQuery(e.target.value); setFindIndex(0); }}
              onKeyDown={(e) => {
                if (e.key === "Escape") { e.preventDefault(); closeFind(); }
                else if (e.key === "Enter") { e.preventDefault(); stepFind(e.shiftKey ? -1 : 1); }
              }}
            />
            <span className={`find-count${findQuery && !findMatches.length ? " is-none" : ""}`}>
              {findQuery ? (findMatches.length ? t("data.find.count", { i: findIndex + 1, n: findMatches.length }) : t("data.find.none")) : ""}
            </span>
            <button type="button" className="icon-button" title={t("data.find.prev")} disabled={!findMatches.length} onClick={() => stepFind(-1)}><SmallIcon name="prev" /></button>
            <button type="button" className="icon-button" title={t("data.find.next")} disabled={!findMatches.length} onClick={() => stepFind(1)}><SmallIcon name="next" /></button>
            <label className="check-row">
              <input type="checkbox" checked={highlightAll} onChange={(e) => setHighlightAll(e.target.checked)} />
              <span>{t("data.find.highlight")}</span>
            </label>
            <span className="spacer" />
            <label className="check-row" title={readOnly ? t("data.readOnlyNoKey") : undefined}>
              <input type="checkbox" checked={replaceMode} disabled={readOnly} onChange={(e) => setReplaceMode(e.target.checked)} />
              <span>{t("data.find.replace")}</span>
            </label>
          </div>
          {replaceMode && !readOnly && (
            <div className="data-find data-find-replace" role="search">
              <span className="find-close" aria-hidden="true" />
              <input
                type="text"
                className="field find-input"
                placeholder={t("data.find.replaceWith")}
                value={replaceText}
                onChange={(e) => setReplaceText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") { e.preventDefault(); closeFind(); }
                  else if (e.key === "Enter") { e.preventDefault(); replaceCurrent(); }
                }}
              />
              <span className="find-count" />
              <button type="button" className="find-button" disabled={!findMatches.length} onClick={replaceCurrent}>{t("data.find.replace")}</button>
              <button type="button" className="find-button" disabled={!findMatches.length} onClick={replaceAll}>{t("data.find.replaceAll")}</button>
              <span className="spacer" />
            </div>
          )}
        </div>
      )}

      <div className="data-footer">
        <button type="button" className="icon-button" title={t("data.addRecord")} disabled={readOnly || loading} onClick={addRow}><SmallIcon name="addRow" /></button>
        <button type="button" className="icon-button" title={t("data.deleteRecord")} disabled={readOnly || !rows.length} onClick={() => deleteRow()}><SmallIcon name="removeRow" /></button>
        <button type="button" className="icon-button" title={t("data.apply")} disabled={!dirty || applying} onClick={requestApply}><SmallIcon name="apply" /></button>
        <button type="button" className="icon-button" title={t("data.discard")} disabled={!dirty} onClick={discard}><SmallIcon name="discard" /></button>
        <button type="button" className="icon-button" title={t("data.refresh")} disabled={dirty || loading} onClick={refresh}><SmallIcon name="refresh" /></button>
        <button type="button" className="icon-button" title={t("data.stop")} disabled={!backend?.cancel || (!loading && !applying)} onClick={stop}><SmallIcon name="stopSmall" /></button>
        {(loading || applying) && <span className="spinner" />}
        {inTx && <span className="data-pending" title={t("data.inTransaction")}>{t("data.inTransaction")}</span>}
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
            <input
              className="field page-input"
              value={pageText}
              disabled={!canPage}
              inputMode="numeric"
              aria-label={t("data.page")}
              onChange={(e) => setPageText(e.target.value.replace(/\D/g, ""))}
              onFocus={(e) => e.target.select()}
              onBlur={commitPage}
              onKeyDown={(e) => {
                if (e.key === "Enter") { e.preventDefault(); commitPage(); }
                if (e.key === "Escape") { e.preventDefault(); setPageText(String(page + 1)); }
              }}
            />
            <button type="button" className="icon-button" disabled={!canPage || !hasNext} title={t("data.nextPage")} onClick={() => void loadPage(page + 1, table, limit)}><SmallIcon name="next" /></button>
            <button type="button" className="icon-button" disabled={!canPage || !backend?.count || limit === null} title={t("data.lastPage")} onClick={() => void goLast()}><SmallIcon name="last" /></button>
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

      {filterOpen && (
        <FilterSortDialog
          columns={cols}
          initialFilter={filter}
          initialSort={sort}
          initialColumn={filterColumn}
          dialect={backend?.dialect}
          onApply={applyFilterSort}
          onClose={() => setFilterOpen(false)}
        />
      )}

      {columnsOpen && (
        <ColumnsDialog
          columns={cols.map((c) => ({ name: c.name, type: columnType(c) }))}
          hidden={hiddenCols}
          onToggle={(name) => setHiddenCols((s) => {
            const n = new Set(s);
            if (n.has(name)) n.delete(name); else n.add(name);
            // Nunca deixa todas as colunas ocultas: sempre sobra pelo menos uma visível.
            return n.size >= cols.length ? s : n;
          })}
          onShowAll={() => setHiddenCols(new Set())}
          onReset={resetLayout}
          onHideAll={() => setHiddenCols(new Set(cols.slice(1).map((c) => c.name)))}
          onClose={() => setColumnsOpen(false)}
        />
      )}

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

      {/* Erro ao aplicar sem a janela de confirmação: as alterações continuam na tela. */}
      {!confirmApply && applyError && (
        <MessageDialog
          title={t("data.applyFailed")}
          message={applyError}
          danger
          onOk={() => setApplyError(null)}
          onCancel={() => setApplyError(null)}
        />
      )}
      {profMode === "on" && prof && (() => {
        const column = prof.columns.find((x) => x.name === cols[activeCol]?.name) ?? prof.columns[0];
        return column ? <ColumnStatistics col={column} spotlight={spot} onToggle={(v, add) => toggleSpot(column.name, v, add)} /> : null;
      })()}
      {dupRow !== null && rows[dupRow] && backend && (
        <DuplicateRecordDialog
          table={table.name}
          columns={cols}
          row={rows[dupRow]}
          onClose={() => setDupRow(null)}
          onInsert={async (values) => {
            // Um INSERT só com os campos preenchidos (os automáticos o banco gera); o erro do servidor (chave repetida...) volta para a janela.
            await backend.apply(plannedStatements({ dialect: backend.dialect, schema, table: table.name, columns: cols, original: [], current: [values], edited: new Set(), newRows: new Set([0]), deleted: new Set() }));
            setDupRow(null);
            await loadPage(page, table, limit);
          }}
        />
      )}
      {confirmApply && (
        <MessageDialog
          title={t("data.reviewTitle")}
          message={[
            t("data.confirmApply", { table: table.name }),
            t("data.confirmSummary", { ins: newRows.size, upd: updatedRows, del: deleted.size }),
            deleted.size > 0 ? t("data.confirmDeleteWarn") : "",
          ].filter(Boolean).join("\n\n")}
          confirmText={production ? table.name : undefined}
          danger={deleted.size > 0}
          okLabel={t("data.apply")}
          busy={applying}
          error={applyError ?? undefined}
          onOk={() => void apply()}
          onCancel={() => { setApplyError(null); setConfirmApply(false); }}
        />
      )}
    </div>
  );
}
