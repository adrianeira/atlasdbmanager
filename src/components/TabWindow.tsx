import { emit, listen } from "@tauri-apps/api/event";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useRef, useState } from "react";
import { pg } from "../backend";
import { liveLookupSource, makeTableBackend } from "../docBackends";
import { QueryDoc, type QueryBackend, type QueryContext } from "../documents/QueryDoc";
import { TableDataDoc } from "../documents/TableDataDoc";
import { useT } from "../i18n";
import { local } from "../backend";
import { schemaFromServer } from "../live";
import type { ConnectionMock } from "../model";
import { TAB_EVENT, type SavedTable, type TabEvent, type TabWindowParams } from "../tabWindow";
import type { SnapshotRef, TabSnapshot } from "../tabState";
import { viewKeyOf } from "../tableView";
import { MessageDialog } from "./MessageDialog";
import { ToolWindow } from "./ToolWindow";
import type { StatusCell } from "./StatusBar";

/** Janela de uma aba de tabela ou de consulta que foi destacada da janela principal. */
export function TabWindow() {
  const params = (window as unknown as { __ATLAS_TAB__: TabWindowParams }).__ATLAS_TAB__;
  return (
    <ToolWindow title={params.title}>
      <TabWindowBody params={params} />
    </ToolWindow>
  );
}

const noop = () => undefined;
const NO_CONNECTING = new Set<string>();

function TabWindowBody({ params }: { params: TabWindowParams }) {
  const t = useT();
  const isQuery = params.entry.kind === "query";
  const entry = useRef<SavedTable>(params.entry);
  // Consulta que já tem texto conta como alterada (fechar a janela pergunta), como a aba da janela principal.
  const dirty = useRef(isQuery && !!params.entry.sql?.trim());
  const txOpen = useRef(false);
  const allow = useRef(false);
  const [dialog, setDialog] = useState<null | "close" | "busy">(null);
  const [cells, setCells] = useState<StatusCell[]>([]);
  // Estado da tela da janela principal (resultados, cursor, página...): lido antes de montar a tela, e a tela entrega o atual ao anexar.
  const stateRef: SnapshotRef = useRef(null);
  const initial = useRef<TabSnapshot | null>(null);
  const [ready, setReady] = useState(!params.entry.stateId);
  useEffect(() => {
    const id = params.entry.stateId;
    if (!id) return;
    entry.current = { ...entry.current, stateId: undefined };
    void local.tabStateTake(id).then((s) => { initial.current = s as TabSnapshot | null; }).catch(() => undefined).finally(() => setReady(true));
    // Uma vez ao abrir a janela.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Avisa a janela principal e fecha esta janela. */
  const finish = async (event: TabEvent) => {
    try { await emit(TAB_EVENT, event); } catch { /* sem janela principal: só fecha */ }
    allow.current = true;
    await getCurrentWindow().close();
  };
  const attach = () => {
    // Edições não gravadas (ou transação aberta) não mudam de janela: gravar ou descartar primeiro.
    const busy = isQuery ? txOpen.current : dirty.current;
    if (busy) return setDialog("busy");
    void (async () => {
      // O estado da tela vai junto: a janela principal reconstrói a aba igual.
      const snap = stateRef.current?.();
      if (snap) {
        const id = `s-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
        try { await local.tabStateSave(id, snap); entry.current = { ...entry.current, stateId: id }; } catch { /* volta sem o estado da tela */ }
      }
      await finish({ type: "attach", id: params.id, entry: entry.current });
    })();
  };

  useEffect(() => {
    let cancelled = false;
    let off: (() => void) | undefined;
    void getCurrentWindow().onCloseRequested((e) => {
      if (allow.current) return;
      e.preventDefault();
      if (dirty.current || txOpen.current) setDialog("close");
      else void finish({ type: "closed", id: params.id });
    }).then((u) => { if (cancelled) u(); else off = u; });
    return () => { cancelled = true; off?.(); };
    // `finish` só usa refs e o id.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Arrastar a janela até a barra de abas da janela principal e soltar devolve a aba (como num navegador).
  // O cursor só conta depois de ter estado fora da barra: logo ao abrir, o cursor ainda está sobre ela.
  const overBar = useRef(false);
  const armed = useRef(false);
  const attachRef = useRef(attach);
  attachRef.current = attach;
  useEffect(() => {
    let cancelled = false;
    let offMove: (() => void) | undefined;
    let offHit: (() => void) | undefined;
    let idle: number | null = null;
    let last = 0;
    void getCurrentWindow().onMoved(() => {
      const now = Date.now();
      if (now - last > 60) {
        last = now;
        void cursorPosition().then((c) => emit(TAB_EVENT, { type: "probe", id: params.id, x: c.x, y: c.y } satisfies TabEvent)).catch(() => undefined);
      }
      if (idle) window.clearTimeout(idle);
      // Parou de mover (soltou) com o cursor sobre a barra: anexa.
      idle = window.setTimeout(() => { if (armed.current && overBar.current) attachRef.current(); }, 450);
    }).then((u) => { if (cancelled) u(); else offMove = u; });
    void listen<TabEvent>(TAB_EVENT, (e) => {
      if (e.payload.type !== "probe-hit" || e.payload.id !== params.id) return;
      overBar.current = e.payload.hit;
      if (!e.payload.hit) armed.current = true;
    }).then((u) => { if (cancelled) u(); else offHit = u; });
    return () => { cancelled = true; offMove?.(); offHit?.(); if (idle) window.clearTimeout(idle); };
    // `params.id` não muda.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A janela principal guarda a última versão (texto da consulta, banco e schema) para a sessão.
  const timer = useRef<number | null>(null);
  const sendUpdate = () => {
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void emit(TAB_EVENT, { type: "update", id: params.id, entry: entry.current } satisfies TabEvent).catch(noop), 600);
  };

  return (
    <div className="tw-root">
      <div className="tw-bar">
        <span className="tw-title" title={params.title}>{params.title}</span>
        <button type="button" className="btn" title={t("tab.attachHint")} onClick={attach}>{t("tab.attach")}</button>
      </div>
      <div className="tw-doc doc-host">
        {!ready ? null : isQuery
          ? <QueryPart params={params} entry={entry} initialState={initial.current} stateRef={stateRef} onStatus={setCells} onDirty={(d) => { dirty.current = d; }} onTx={(o) => { txOpen.current = o; }} onChange={sendUpdate} />
          : <DataPart params={params} initialState={initial.current} stateRef={stateRef} onStatus={setCells} onDirty={(d) => { dirty.current = d; }} />}
      </div>
      <footer className="status-bar">
        {cells.map((c, i) => (
          <div key={i} className={`sb-cell sb-doc${c.grow ? " is-grow" : ""}`} style={c.width ? { width: c.width } : undefined} title={typeof c.text === "string" ? c.text : undefined}>{c.text}</div>
        ))}
        {!cells.some((c) => c.grow) && <div className="sb-cell is-grow" />}
      </footer>
      {dialog === "busy" && (
        <MessageDialog title={t("tab.attach")} message={t(isQuery ? "tab.attachBusyTx" : "tab.attachBusyData")} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />
      )}
      {dialog === "close" && (
        <MessageDialog
          title={t("msg.unsavedTitle")}
          message={t(isQuery ? "msg.unsavedQuery" : "msg.unsavedData")}
          okLabel={t("msg.discard")}
          onOk={() => { setDialog(null); void finish({ type: "closed", id: params.id }); }}
          onCancel={() => setDialog(null)}
        />
      )}
    </div>
  );
}

function DataPart({ params, initialState, stateRef, onStatus, onDirty }: { params: TabWindowParams; initialState: TabSnapshot | null; stateRef: SnapshotRef; onStatus: (c: StatusCell[]) => void; onDirty: (d: boolean) => void }) {
  const e = params.entry;
  const view = e.kind === "view" || e.kind === "matview";
  const base = useMemo(() => ({ name: e.object }), [e.object]);
  const backend = useMemo(() => makeTableBackend(params.conn.id, e.database, e.schema, e.object, e.kind === "query" ? undefined : e.kind, base), [params.conn.id, e, base]);
  const lookup = useMemo(() => (view ? undefined : liveLookupSource(params.conn.id, e.database)), [view, params.conn.id, e.database]);
  return (
    <TableDataDoc
      schema={e.schema}
      table={base}
      active
      production={params.conn.environment === "production"}
      onStatus={onStatus}
      backend={backend}
      lookup={lookup}
      isView={view}
      onDirty={onDirty}
      viewKey={viewKeyOf(params.conn.id, e.database, e.schema, e.object)}
      initialState={initialState ?? undefined}
      stateRef={stateRef}
    />
  );
}

function QueryPart({ params, entry, initialState, stateRef, onStatus, onDirty, onTx, onChange }: {
  params: TabWindowParams; entry: React.MutableRefObject<SavedTable>; initialState: TabSnapshot | null; stateRef: SnapshotRef;
  onStatus: (c: StatusCell[]) => void; onDirty: (d: boolean) => void; onTx: (open: boolean) => void; onChange: () => void;
}) {
  const [conn, setConn] = useState<ConnectionMock>(() => ({ ...params.conn, databases: [], live: true }));
  const update = (fn: (c: ConnectionMock) => ConnectionMock) => setConn(fn);
  const open = useMemo(() => new Set([params.conn.id]), [params.conn.id]);

  const loadDb = async (name: string, refresh = false) => {
    const infos = await pg.listSchemas(params.conn.id, name, false);
    update((c) => ({
      ...c,
      databases: c.databases.map((d) => (d.name !== name ? d : {
        ...d, loaded: true,
        schemas: infos.map((s) => ({ ...((refresh ? d.schemas.find((x) => x.name === s.name) : undefined) ?? d.schemas.find((x) => x.name === s.name) ?? { name: s.name, objects: {}, loaded: false }), comment: s.comment ?? undefined })),
      })),
    }));
  };
  const loadSchema = async (db: string, schema: string) => {
    const fresh = schemaFromServer(schema, await pg.listObjects(params.conn.id, db, schema));
    update((c) => ({ ...c, databases: c.databases.map((d) => (d.name !== db ? d : { ...d, schemas: d.schemas.map((s) => (s.name === schema ? fresh : s)) })) }));
  };

  // Bancos do servidor, depois o banco e o schema da consulta (para a lista de bancos e as sugestões do editor).
  useEffect(() => {
    void (async () => {
      const dbs = await pg.listDatabases(params.conn.id, false);
      update((c) => ({ ...c, databases: dbs.databases.map((name) => ({ name, schemas: [], loaded: false, comment: dbs.comments[name] })) }));
      await loadDb(params.entry.database);
      if (params.entry.schema) await loadSchema(params.entry.database, params.entry.schema);
    })().catch(noop);
    // Uma vez ao abrir a janela.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const backend = useMemo<QueryBackend>(() => ({
    // A sessão é a da janela principal; se ela foi fechada lá, a execução devolve o erro do núcleo.
    ensureOpen: async () => undefined,
    execute: (connId, db, sqlText, max, schema) => pg.execute(connId, db, sqlText, max, schema),
    cancel: (connId, db) => pg.cancel(connId, db),
    beginTx: (connId, db) => pg.txBegin(connId, db),
    executeInTx: (txId, sqlText, max, schema) => pg.txExecute(txId, sqlText, max, schema),
    endTx: (txId, commit) => pg.txEnd(txId, commit),
  }), []);

  const ctxRef = useRef<QueryContext>({ connectionId: params.conn.id, database: params.entry.database, schema: params.entry.schema });
  return (
    <QueryDoc
      connections={[conn]}
      openConnections={open}
      initial={ctxRef.current}
      initialSql={params.entry.sql}
      initialState={initialState ?? undefined}
      stateRef={stateRef}
      active
      onStatus={onStatus}
      onDirty={onDirty}
      onTransaction={onTx}
      onText={(text) => { entry.current = { ...entry.current, sql: text }; onChange(); }}
      onContext={(c) => {
        const changed = c.database !== ctxRef.current.database || c.schema !== ctxRef.current.schema;
        ctxRef.current = c;
        if (!changed || !c.database) return;
        entry.current = { ...entry.current, database: c.database, schema: c.schema ?? entry.current.schema };
        onChange();
        const db = conn.databases.find((d) => d.name === c.database);
        if (db && !db.loaded) void loadDb(c.database).catch(noop);
        if (c.schema) void loadSchema(c.database, c.schema).catch(noop);
      }}
      registerActions={noop}
      backend={backend}
      connecting={NO_CONNECTING}
      onOpenConnection={noop}
      onLoadSchemas={(_connId, dbName) => { void loadDb(dbName).catch(noop); }}
      onRefreshSchemas={(_connId, dbName) => { void loadDb(dbName, true).catch(noop); }}
      onExecuted={(e) => {
        // O histórico fica só no banco local; a preferência "history.enabled" (padrão ligada) o desliga.
        void local.getPref<boolean>("history.enabled").then((on) => {
          if (on === false) return;
          return local.historyAdd({
            connectionId: e.connectionId, connectionName: e.connectionName, database: e.database, schema: e.schema ?? null,
            sql: e.sql, durationMs: e.durationMs, rows: e.rows, error: e.error,
          });
        }).catch(noop);
      }}
    />
  );
}
