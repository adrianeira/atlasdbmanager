import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { DataGrid, typeKind, type GridColumn } from "../components/DataGrid";
import { Dropdown } from "../components/Dropdown";
import { PopupMenu } from "../components/PopupMenu";
import { SqlEditor, type SqlEditorHandle } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { ProviderIcon, SmallIcon, type SmallIconName } from "../icons";
import { mockRows, type CellValue, type ConnectionMock } from "../model";
import { columnType } from "../sql/ddl";
import { beautifySql } from "../sql/format";
import { errorText, toAtlasError } from "../backend";

export interface QueryContext {
  connectionId?: string;
  database?: string;
  schema?: string;
}

interface Props {
  connections: ConnectionMock[];
  openConnections: Set<string>;
  initial: QueryContext;
  initialSql?: string;
  active: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  /** Permite que menus globais (Query/Format) acionem este documento. */
  registerActions: (a: QueryActions | null) => void;
  /** Acesso ao banco real; só é usado quando a conexão escolhida é real (senão, execução simulada). */
  backend?: QueryBackend;
  /** Conexões em processo de abertura (mostra o indicador de carregamento). */
  connecting?: Set<string>;
  /** Escolheu uma conexão real fechada: abre (pedindo a senha se preciso) e lista os bancos. */
  onOpenConnection?: (connectionId: string) => void;
  /** Escolheu um banco ainda sem schemas lidos: busca no servidor. */
  onLoadSchemas?: (connectionId: string, database: string) => void;
}

export interface QueryBackend {
  /** Abre a conexão se necessário (pode pedir a senha). */
  ensureOpen: (connectionId: string) => Promise<void>;
  execute: (connectionId: string, database: string, sql: string, maxRows: number) => Promise<QueryOutcomeLive>;
  cancel: (connectionId: string, database: string) => Promise<void>;
}

type QueryOutcomeLive = import("../backend").QueryOutcome;

/** Linhas máximas por result set no editor (o excedente é sinalizado como truncado). */
const EDITOR_MAX_ROWS = 1000;

function fromLive(sqlText: string, out: QueryOutcomeLive, t: ReturnType<typeof useT>): RunOutcome {
  const results: ResultSet[] = [];
  const lines: string[] = [sqlText.trim()];
  out.statements.forEach((st, i) => {
    if (st.result) {
      results.push({
        columns: st.result.columns.map((c) => ({ name: c.name, type: c.typeName ?? "", kind: typeKind(c.typeName ?? "") })),
        rows: st.result.rows,
      });
      lines.push(`> [${i + 1}] OK — ${t("query.rowsReturned", { n: st.result.rows.length })}${st.result.truncated ? ` (${t("query.truncated", { n: EDITOR_MAX_ROWS })})` : ""}`);
    } else {
      lines.push(`> [${i + 1}] OK — ${t("query.rowsAffected", { n: st.rowsAffected ?? 0 })}`);
    }
  });
  return { sql: sqlText.trim(), ms: out.elapsedMs, results, message: lines.join("\n") };
}

export interface QueryActions {
  run: (selectionOnly: boolean) => void;
  stop: () => void;
  explain: () => void;
  beautify: () => void;
}

interface ResultSet {
  columns: GridColumn[];
  rows: CellValue[][];
}

interface RunOutcome {
  sql: string;
  ms: number;
  results: ResultSet[];
  message: string;
  error?: string;
}

/**
 * Execução SIMULADA do M0: reconhece SELECT simples sobre as tabelas fictícias.
 * Não divide lotes nem classifica SQL — no M1 isso é do parser/driver no núcleo Rust (ADR-016).
 */
function mockExecute(sqlText: string, conn: ConnectionMock | undefined, database?: string, schema?: string): RunOutcome {
  const started = performance.now();
  const text = sqlText.trim().replace(/;\s*$/, "");
  const ms = () => Math.max(1, Math.round(performance.now() - started + 3));
  if (!conn) return { sql: text, ms: 0, results: [], message: "", error: "no connection selected" };
  const sc = conn.databases.find((d) => d.name === database)?.schemas.find((s) => s.name === schema);

  const fromMatch = /^select\s+([\s\S]+?)\s+from\s+((?:"[^"]+"|\w+)(?:\.(?:"[^"]+"|\w+))?)(?:\s+limit\s+(\d+))?/i.exec(text);
  if (fromMatch) {
    const ident = fromMatch[2].split(".").pop()!.replace(/^"|"$/g, "");
    const table = sc?.objects.table?.find((x) => x.name === ident || x.name.toLowerCase() === ident.toLowerCase());
    if (!table) return { sql: text, ms: ms(), results: [], message: "", error: `ERROR:  relation "${ident}" does not exist\nLINE 1: ${text.slice(0, 80)}` };
    let rows = mockRows(table);
    const limit = fromMatch[3] ? Number(fromMatch[3]) : undefined;
    if (limit !== undefined) rows = rows.slice(0, limit);
    const cols = (table.columns ?? []).map((c) => ({ name: c.name, type: columnType(c), kind: typeKind(c.type) }));
    return { sql: text, ms: ms(), results: [{ columns: cols, rows }], message: `${text}\n> OK\n> ${rows.length} rows` };
  }
  const exprMatch = /^select\s+([\s\S]+)$/i.exec(text);
  if (exprMatch && !/\bfrom\b/i.test(text)) {
    const parts = exprMatch[1].split(",").map((p) => p.trim());
    const cols = parts.map((p, i) => {
      const alias = /\s+as\s+("?)(\w+)\1$/i.exec(p)?.[2] ?? (/^'.*'$/.test(p) ? "?column?" : p.replace(/\(.*$/, "")) ?? `col${i}`;
      return { name: alias, type: /^\d+$/.test(p) ? "int4" : /^'/.test(p) ? "text" : /now\(\)/i.test(p) ? "timestamptz" : /version\(\)/i.test(p) ? "text" : "unknown", kind: typeKind(/^\d+$/.test(p) ? "int4" : "text") };
    });
    const row = parts.map((p) => {
      if (/^\d+$/.test(p)) return p;
      if (/^'(.*)'$/.test(p)) return p.slice(1, -1);
      if (/now\(\)/i.test(p)) return new Date().toISOString().replace("T", " ").replace("Z", "+00");
      if (/version\(\)/i.test(p)) return `PostgreSQL ${conn.serverVersion} (Atlas mock)`;
      if (/^null$/i.test(p)) return null;
      return p;
    });
    return { sql: text, ms: ms(), results: [{ columns: cols, rows: [row] }], message: `${text}\n> OK` };
  }
  return { sql: text, ms: ms(), results: [], message: `${text}\n> [M0] Execução simulada: comando aceito, nada foi enviado ao servidor.` };
}

export function QueryDoc({
  connections, openConnections, initial, initialSql, active, onStatus, onDirty, registerActions, backend: backendProp, connecting, onOpenConnection, onLoadSchemas,
}: Props) {
  const t = useT();
  const editor = useRef<SqlEditorHandle | null>(null);
  const [ctx, setCtx] = useState<QueryContext>(initial);
  const [cursor, setCursor] = useState({ line: 1, col: 1 });
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<RunOutcome | null>(null);
  const [resultTab, setResultTab] = useState(0);
  const [activeRow, setActiveRow] = useState(0);
  const [split, setSplit] = useState(0.55);
  const [runMenu, setRunMenu] = useState(false);
  const runTimer = useRef<number | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);

  const conn = connections.find((c) => c.id === ctx.connectionId);
  const db = conn?.databases.find((d) => d.name === ctx.database);
  const schemaMock = db?.schemas.find((s) => s.name === ctx.schema);
  // Banco real só para conexão real: a escolha atual manda, não a conexão com que a aba foi aberta.
  const backend = conn?.live ? backendProp : undefined;

  /* ----- Seletores em cascata: conexão → banco → schema. Cada nível busca o seguinte no servidor. ----- */
  const requested = useRef(new Set<string>());

  const pickConnection = (id: string) => {
    const c = connections.find((x) => x.id === id);
    setCtx({ connectionId: c?.id });
    // Conexão real fechada: abre e lista os bancos (os efeitos abaixo escolhem o resto).
    if (c?.live && !openConnections.has(c.id)) onOpenConnection?.(c.id);
  };

  const pickDatabase = (name: string) => {
    if (conn) requested.current.delete(`${conn.id}/${name}`); // reescolher o banco tenta de novo se a leitura falhou
    setCtx({ ...ctx, database: name, schema: undefined });
  };

  // Com os bancos conhecidos, escolhe um: o inicial da conexão ou o primeiro.
  useEffect(() => {
    if (!conn || conn.databases.length === 0) return;
    if (ctx.database && conn.databases.some((d) => d.name === ctx.database)) return;
    const pick = conn.databases.find((d) => d.name === conn.initialDatabase) ?? conn.databases[0];
    setCtx((c) => ({ ...c, database: pick.name, schema: undefined }));
  }, [conn, ctx.database]);

  // Banco escolhido sem schemas lidos: busca no servidor (uma vez por abertura).
  useEffect(() => {
    if (!conn?.live || !db) return;
    const key = `${conn.id}/${db.name}`;
    if (db.loaded) {
      requested.current.delete(key);
      return;
    }
    if (requested.current.has(key)) return;
    requested.current.add(key);
    onLoadSchemas?.(conn.id, db.name);
  }, [conn, db, onLoadSchemas]);

  // Com os schemas conhecidos, escolhe um: "public" se existir, senão o primeiro.
  useEffect(() => {
    if (!db || db.schemas.length === 0) return;
    if (ctx.schema && db.schemas.some((s) => s.name === ctx.schema)) return;
    const pick = db.schemas.find((s) => s.name === "public") ?? db.schemas[0];
    setCtx((c) => ({ ...c, schema: pick.name }));
  }, [db, ctx.schema]);

  const loadingCtx = !!conn && (!!connecting?.has(conn.id) || (!!conn.live && !!db && !db.loaded));

  const completionSchema = useMemo(() => {
    const out: Record<string, string[]> = {};
    for (const tb of schemaMock?.objects.table ?? []) out[tb.name] = (tb.columns ?? []).map((c) => c.name);
    return out;
  }, [schemaMock]);

  const run = useCallback((selectionOnly: boolean) => {
    if (running) return;
    // Com seleção executa só a seleção; sem ela, o documento inteiro (sem adivinhar o "statement atual" — ADR-016).
    const sel = editor.current?.getSelection() ?? "";
    const text = (selectionOnly || sel ? sel : "") || (editor.current?.getText() ?? "");
    if (!text.trim()) return;
    setRunning(true);
    if (backend && conn) {
      const database = ctx.database ?? conn.initialDatabase;
      void (async () => {
        try {
          await backend.ensureOpen(conn.id);
          const res = fromLive(text, await backend.execute(conn.id, database, text, EDITOR_MAX_ROWS), t);
          setOutcome(res);
          setResultTab(res.results.length ? 1 : 0);
        } catch (e) {
          const err = toAtlasError(e);
          const pos = err.position ? `\n> ${t("query.errorPosition", { n: err.position })}` : "";
          setOutcome({ sql: text.trim(), ms: 0, results: [], message: "", error: `${errorText(err)}${pos}` });
          setResultTab(0);
        } finally {
          setActiveRow(0);
          setRunning(false);
        }
      })();
      return;
    }
    runTimer.current = window.setTimeout(() => {
      const res = mockExecute(text, conn, ctx.database, ctx.schema);
      setOutcome(res);
      setResultTab(res.results.length ? 1 : 0);
      setActiveRow(0);
      setRunning(false);
    }, 280);
  }, [running, conn, ctx.database, ctx.schema, backend, t]);

  const stop = useCallback(() => {
    if (backend && conn && running) {
      // Pede o cancelamento ao servidor; o resultado (erro 57014) chega pelo próprio run.
      void backend.cancel(conn.id, ctx.database ?? conn.initialDatabase).catch(() => undefined);
      return;
    }
    if (runTimer.current) window.clearTimeout(runTimer.current);
    runTimer.current = null;
    if (running) {
      setRunning(false);
      setOutcome({ sql: "", ms: 0, results: [], message: t("query.cancelled") });
      setResultTab(0);
    }
  }, [running, t, backend, conn, ctx.database]);

  const beautify = useCallback(() => {
    const text = editor.current?.getText() ?? "";
    if (text.trim()) editor.current?.replaceAll(beautifySql(text));
  }, []);

  const explain = useCallback(() => {
    const text = editor.current?.getSelection() || editor.current?.getText() || "";
    if (!text.trim()) return;
    if (backend && conn) {
      // EXPLAIN sem ANALYZE: o servidor só planeja, não executa o comando.
      const sqlText = `EXPLAIN ${text.trim().replace(/;\s*$/, "")}`;
      setRunning(true);
      void (async () => {
        try {
          await backend.ensureOpen(conn.id);
          const res = fromLive(sqlText, await backend.execute(conn.id, ctx.database ?? conn.initialDatabase, sqlText, EDITOR_MAX_ROWS), t);
          setOutcome(res);
          setResultTab(res.results.length ? 1 : 0);
        } catch (e) {
          setOutcome({ sql: sqlText, ms: 0, results: [], message: "", error: errorText(toAtlasError(e)) });
          setResultTab(0);
        } finally {
          setActiveRow(0);
          setRunning(false);
        }
      })();
      return;
    }
    setOutcome({
      sql: text, ms: 1,
      results: [{
        columns: [{ name: "QUERY PLAN", type: "text", kind: "text" }],
        rows: [["Seq Scan on (mock)  (cost=0.00..1.00 rows=1 width=0)"], ["[M0] EXPLAIN simulado; o plano real vem do servidor no M1."]],
      }],
      message: `EXPLAIN ${text.trim()}\n> OK`,
    });
    setResultTab(1);
  }, [backend, conn, ctx.database, t]);

  useEffect(() => {
    registerActions(active ? { run, stop, explain, beautify } : null);
  }, [active, run, stop, explain, beautify, registerActions]);

  const current = outcome && resultTab > 0 ? outcome.results[resultTab - 1] : undefined;

  useEffect(() => {
    if (!active) return;
    const cells: StatusCell[] = [];
    if (outcome) {
      cells.push({ text: outcome.sql.replace(/\s+/g, " ").slice(0, 160), grow: true });
      cells.push({ text: t("query.time", { s: (outcome.ms / 1000).toFixed(3) }), width: 150 });
      if (current) cells.push({ text: t("data.record", { n: current.rows.length ? activeRow + 1 : 0, total: current.rows.length, page: 1 }), width: 200 });
    } else {
      cells.push({ text: t("query.completionReady"), grow: true });
    }
    cells.push({ text: `Ln ${cursor.line}, Col ${cursor.col}`, width: 110 });
    onStatus(cells);
  }, [active, outcome, current, activeRow, cursor, t, onStatus]);

  const startSplit = (e: ReactMouseEvent) => {
    e.preventDefault();
    const rect = bodyRef.current?.getBoundingClientRect();
    if (!rect) return;
    const onMove = (ev: MouseEvent) => setSplit(Math.min(0.85, Math.max(0.15, (ev.clientY - rect.top) / rect.height)));
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.classList.remove("is-resizing-v");
    };
    document.body.classList.add("is-resizing-v");
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const tool = (icon: SmallIconName, label: MessageKey, onClick?: () => void) => (
    <button type="button" className="ot-button" disabled={!onClick} onClick={onClick} title={onClick ? t(label) : t("notImplemented")}>
      <SmallIcon name={icon} />
      <span>{t(label)}</span>
    </button>
  );

  const connOpen = !!conn && openConnections.has(conn.id);

  return (
    <div className="doc">
      <div className="object-toolbar" role="toolbar">
        {tool("save", "query.save")}
        {tool("queryBuilder", "query.builder")}
        {tool("beautify", "query.beautify", beautify)}
        {tool("snippet", "query.snippet")}
        {tool("ai", "query.askAi")}
        <div className="ot-spacer" />
        <button type="button" className="icon-button is-active" title={t("query.layoutStacked")}><SmallIcon name="splitH" /></button>
        <button type="button" className="icon-button" title={t("notImplemented")} disabled><SmallIcon name="splitV" /></button>
        <button type="button" className="icon-button" title={t("notImplemented")} disabled><SmallIcon name="fullscreen" /></button>
      </div>

      <div className="query-context">
        <Dropdown
          className="ctx-dd"
          ariaLabel={t("tb.connection")}
          leftIcon={conn ? <ProviderIcon provider={conn.provider} open={connOpen} /> : <SmallIcon name="myConnections" />}
          value={ctx.connectionId ?? ""}
          items={[{ value: "", label: t("query.noConnection") }, ...connections.map((c) => ({ value: c.id, label: c.name }))]}
          onChange={pickConnection}
        />
        <Dropdown
          className="ctx-dd ctx-dd-db"
          ariaLabel={t("info.database")}
          leftIcon={<SmallIcon name={db ? "database" : "databaseClosed"} />}
          value={ctx.database ?? ""}
          items={(conn?.databases ?? []).map((d) => ({ value: d.name, label: d.name }))}
          disabled={!conn}
          placeholder={conn && conn.databases.length === 0 ? t(connecting?.has(conn.id) ? "nav.connecting" : conn.live ? "query.ctx.closed" : "query.ctx.noDatabases") : undefined}
          onChange={pickDatabase}
        />
        <Dropdown
          className="ctx-dd"
          ariaLabel={t("info.schema")}
          leftIcon={<SmallIcon name={schemaMock ? "schema" : "schemaGray"} />}
          value={ctx.schema ?? ""}
          items={(db?.schemas ?? []).map((s) => ({ value: s.name, label: s.name }))}
          disabled={!db}
          onChange={(v) => setCtx({ ...ctx, schema: v })}
        />
        {loadingCtx && <span className="spinner" aria-label={t("common.loading")} />}
        <div className="split-button">
          <button type="button" className="ot-button run-button" disabled={!conn || running} onClick={() => run(false)} title="Ctrl+R / F5">
            <SmallIcon name="run" />
            <span>{t("query.run")}</span>
          </button>
          <button type="button" className="ot-button split-arrow" disabled={!conn || running} onClick={() => setRunMenu((o) => !o)} aria-label={t("query.run")}>
            <SmallIcon name="chevronDown" size={10} />
          </button>
          {runMenu && (
            <PopupMenu
              commands={{}}
              onClose={() => setRunMenu(false)}
              items={[
                { kind: "item", text: `${t("query.run")}  (Ctrl+R)`, run: () => run(false) },
                { kind: "item", text: `${t("query.runSelected")}  (Ctrl+Enter)`, run: () => run(true) },
              ]}
            />
          )}
        </div>
        <button type="button" className="ot-button" disabled={!running} onClick={stop}><SmallIcon name="stop" /><span>{t("query.stop")}</span></button>
        <button type="button" className="ot-button" disabled={!conn || running} onClick={explain}><SmallIcon name="explain" /><span>{t("query.explain")}</span></button>
        {running && <span className="spinner" />}
      </div>

      <div className="query-body" ref={bodyRef}>
        <div className="query-editor" style={{ flexBasis: outcome ? `${split * 100}%` : "100%" }}>
          <SqlEditor
            value={initialSql ?? ""}
            schema={completionSchema}
            onChange={() => onDirty(true)}
            onCursor={(line, col) => setCursor({ line, col })}
            onRun={run}
            handleRef={(h) => { editor.current = h; }}
          />
        </div>
        {outcome && (
          <>
            <div className="hsplitter" role="separator" aria-orientation="horizontal" onMouseDown={startSplit} />
            <div className="query-results">
              <div className="result-tabs" role="tablist">
                <button type="button" role="tab" aria-selected={resultTab === 0} className={`result-tab${resultTab === 0 ? " is-active" : ""}`} onClick={() => setResultTab(0)}>{t("query.message")}</button>
                {outcome.results.map((_, i) => (
                  <button key={i} type="button" role="tab" aria-selected={resultTab === i + 1} className={`result-tab${resultTab === i + 1 ? " is-active" : ""}`} onClick={() => { setResultTab(i + 1); setActiveRow(0); }}>
                    {t("query.result", { n: i + 1 })}
                  </button>
                ))}
              </div>
              <div className="result-body">
                {resultTab === 0 || !current ? (
                  <pre className={`result-message${outcome.error ? " is-error" : ""}`}>
                    {outcome.error ? `${outcome.sql}\n> ${outcome.error}` : outcome.message}
                    {"\n> "}{t("query.time", { s: (outcome.ms / 1000).toFixed(3) })}
                  </pre>
                ) : (
                  <DataGrid columns={current.columns} rows={current.rows} readOnly activeRow={activeRow} onActiveRow={setActiveRow} />
                )}
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
