import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { errorText, isDesktop, local, pg, toAtlasError, type ConnectionParams, type ProfileRecord } from "./backend";
import { entries, matchesShortcut, sub, type CommandMap, type MenuDef, type MenuEntry } from "./commands";
import { ConnectionDialog, type ConnectionForm } from "./components/ConnectionDialog";
import { schemaFromServer, sessionPasswords, tableFromServer } from "./live";
import type { TableDataBackend } from "./documents/TableDataDoc";
import type { FkLookupSource } from "./components/FkLookup";
import type { QueryBackend } from "./documents/QueryDoc";
import { InfoPane } from "./components/InfoPane";
import { MainToolbar } from "./components/MainToolbar";
import { MenuBar } from "./components/MenuBar";
import { MessageDialog } from "./components/MessageDialog";
import { connIdOf, NavigationPane } from "./components/NavigationPane";
import { objectsFor, ObjectsView, type ViewMode } from "./components/ObjectsView";
import { PopupMenu } from "./components/PopupMenu";
import { StatusBar, type StatusCell } from "./components/StatusBar";
import { TabBar, type DocTab } from "./components/TabBar";
import { QueryDoc, type QueryActions } from "./documents/QueryDoc";
import { TableDataDoc } from "./documents/TableDataDoc";
import { TableDesignerDoc } from "./documents/TableDesignerDoc";
import { ViewDesignerDoc, type ViewBackend } from "./documents/ViewDesignerDoc";
import { detectLang, I18nContext, makeTranslator, type Lang, type MessageKey } from "./i18n";
import { ProviderIcon, SmallIcon } from "./icons";
import {
  buildVisibleTree, CONNECTION_COLORS, INITIAL_CONNECTIONS, mockRows, quoteIdent, resolveNode, seg,
  type Category, type ConnectionColor, type ConnectionMock, type DbObject, type ProviderId, type TreeNode,
} from "./model";

/** Cores do menu "Cor" (as mesmas do CSS `.color-*`). */
const COLOR_HEX: Record<ConnectionColor, string> = {
  red: "#e5484d", orange: "#f08c2e", yellow: "#e2b714", green: "#2fb36b", blue: "#2f7fd0", purple: "#8c4fe0", gray: "#8c8c8c",
};

/** Limite para "Executar arquivo SQL" (o arquivo é lido inteiro na memória). */
const MAX_SQL_FILE_BYTES = 50 * 1024 * 1024;

type Theme = "system" | "light" | "dark";

interface Layout {
  navWidth: number;
  navVisible: boolean;
  infoVisible: boolean;
  viewMode: ViewMode;
  showCaptions: boolean;
  theme: Theme;
  lang: Lang;
}

const LAYOUT_KEY = "atlas.layout.v1";
const CONNECTIONS_KEY = "atlas.connections.v1";
const DEFAULT_LAYOUT: Layout = {
  navWidth: 384, navVisible: true, infoVisible: false, viewMode: "detail", showCaptions: true, theme: "system", lang: detectLang(),
};

/* ---------- Persistência local do protótipo (no M1: SQLite local + cofre do SO) ---------- */

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? { ...fallback, ...JSON.parse(raw) } : fallback;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* sem armazenamento disponível: segue só em memória */
  }
}

/** Perfil salvo: NUNCA contém senha (spec SEG-001/ADR-007). */
interface SavedProfile {
  id: string; name: string; provider: ProviderId; host: string; port: number; user: string; initialDatabase: string; environment: string;
  sslMode?: ConnectionMock["sslMode"];
  color?: ConnectionColor;
  starred?: boolean;
}

/** No app desktop a conexão é real e os níveis são lidos do servidor sob demanda. */
function profileToConnection(p: SavedProfile): ConnectionMock {
  return {
    ...p, serverVersion: "-", serverVersionNum: 0, hiddenDatabases: 0, roles: [], live: isDesktop,
    databases: isDesktop
      ? []
      : [{ name: p.initialDatabase || "postgres", schemas: [{ name: "public", objects: { table: [], view: [], matview: [], function: [], query: [], backup: [] } }] }],
  };
}

function readLegacyProfiles(): SavedProfile[] {
  try {
    const raw = localStorage.getItem(CONNECTIONS_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function loadConnections(): ConnectionMock[] {
  // App desktop: os perfis vêm do atlas.db (carregados de forma assíncrona na abertura).
  if (isDesktop) return [];
  // Modo de demonstração (navegador): dados fictícios + perfis criados nesta máquina.
  return [...INITIAL_CONNECTIONS, ...readLegacyProfiles().map(profileToConnection)];
}

/** Conexão → registro do atlas.db. Opções visuais vão em `options`; a senha nunca. */
function toRecord(c: ConnectionMock, sortOrder: number): ProfileRecord {
  const options: Record<string, unknown> = { sslMode: c.sslMode ?? "disable" };
  if (c.color) options.color = c.color;
  if (c.starred) options.starred = true;
  return {
    id: c.id, name: c.name, provider: c.provider, host: c.host, port: c.port, initialDatabase: c.initialDatabase,
    user: c.user, environment: c.environment ?? "development", options, sortOrder, hasPassword: !!c.hasPassword,
  };
}

function fromRecord(r: ProfileRecord): ConnectionMock {
  const o = r.options;
  return {
    ...profileToConnection({
      id: r.id, name: r.name, provider: r.provider as ProviderId, host: r.host, port: r.port, user: r.user,
      initialDatabase: r.initialDatabase, environment: r.environment,
      sslMode: (o.sslMode as ConnectionMock["sslMode"]) ?? "disable",
      color: o.color as ConnectionColor | undefined,
      starred: o.starred === true,
    }),
    hasPassword: r.hasPassword,
  };
}

/** Identidade do conteúdo persistido (sem `hasPassword`, que é derivado do cofre). */
const profileKey = (r: ProfileRecord) => JSON.stringify({ ...r, hasPassword: undefined });

function saveConnections(list: ConnectionMock[]) {
  const builtIn = new Set(INITIAL_CONNECTIONS.map((c) => c.id));
  const profiles: SavedProfile[] = list
    .filter((c) => !builtIn.has(c.id))
    .map((c) => ({
      id: c.id, name: c.name, provider: c.provider, host: c.host, port: c.port, user: c.user,
      initialDatabase: c.initialDatabase, environment: c.environment ?? "development", sslMode: c.sslMode,
      color: c.color, starred: c.starred,
    }));
  writeJson(CONNECTIONS_KEY, profiles);
}

function paramsFor(c: ConnectionMock): ConnectionParams {
  return {
    host: c.host, port: c.port, database: c.initialDatabase || "postgres", user: c.user,
    password: sessionPasswords.get(c.id) ?? "", sslMode: c.sslMode ?? "disable",
  };
}

const CATEGORY_LABEL: Record<Category, MessageKey> = {
  table: "tb.table", view: "tb.view", matview: "cat.matview", function: "tb.function",
  sequence: "cat.sequence", type: "cat.type", user: "tb.user", query: "tb.query",
  backup: "tb.backup", automation: "tb.automation", model: "tb.model", bi: "tb.bi",
};

const FOLDER_LABEL: Partial<Record<Category, MessageKey>> = {
  table: "folder.table", view: "folder.view", matview: "folder.matview",
  function: "folder.function", query: "folder.query", backup: "folder.backup",
};

const COUNT_LABEL: Record<Category, MessageKey> = {
  table: "status.count.table", view: "status.count.view", matview: "status.count.matview", function: "status.count.function",
  sequence: "status.count.sequence", type: "status.count.type", user: "status.count.user", query: "status.count.query",
  backup: "status.count.backup", automation: "status.count.automation", model: "status.count.model", bi: "status.count.bi",
};

const OBJECTS_TAB: DocTab = { id: "objects", kind: "objects", title: "" };

interface RecentItem { label: string; tab: Omit<DocTab, "id"> }

/** Comando de estrutura disparado pelos menus de contexto: sempre revisado antes de executar. */
interface DdlRequest {
  title: string;
  statements: string[];
  connId: string;
  database: string;
  schema?: string;
  /** O que reler na árvore depois de executar. */
  refresh: "connection" | "database" | "schema";
  danger?: boolean;
  /** `false` para comandos que o PostgreSQL não aceita em transação (CREATE/DROP DATABASE). */
  transactional?: boolean;
  /** Exige digitar este nome para confirmar. */
  confirmText?: string;
  /** Fecha a conexão do Atlas com este banco antes (DROP DATABASE falha se houver sessões nele). */
  closeDbFirst?: string;
  note?: string;
  after?: () => void;
}

type Dialog =
  | { kind: "connection"; editing?: ConnectionMock }
  | { kind: "deleteConnection"; conn: ConnectionMock }
  | { kind: "closeDirty"; tabId: string }
  | { kind: "password"; connId: string; retry?: string }
  | { kind: "error"; title: string; message: string }
  | { kind: "info"; title: string; message: string }
  | { kind: "newDatabase"; connId: string; name?: string; busy?: boolean; error?: string }
  | ({ kind: "ddl"; busy?: boolean; error?: string } & DdlRequest)
  | { kind: "name"; title: string; label: string; initial: string; onSubmit: (value: string) => void }
  | { kind: "execFile"; connId: string; database: string; fileName: string; size: number; sql: string; busy?: boolean; error?: string }
  | { kind: "about" };

export default function App() {
  const [layout, setLayout] = useState<Layout>(() => readJson(LAYOUT_KEY, DEFAULT_LAYOUT));
  const t = useMemo(() => makeTranslator(layout.lang), [layout.lang]);
  const patchLayout = useCallback((p: Partial<Layout>) => setLayout((l) => ({ ...l, ...p })), []);

  useEffect(() => writeJson(LAYOUT_KEY, layout), [layout]);
  useEffect(() => {
    const root = document.documentElement;
    if (layout.theme === "system") delete root.dataset.theme;
    else root.dataset.theme = layout.theme;
    root.lang = layout.lang;
    // A barra de título nativa acompanha o tema que a interface está mostrando
    // (sem isso ela segue o modo de apps do Windows e pode ficar escura com a UI clara).
    if (!isDesktop) return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const effective = layout.theme === "system" ? (media.matches ? "dark" : "light") : layout.theme;
      void getCurrentWindow().setTheme(effective).catch(() => undefined);
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [layout.theme, layout.lang]);

  /* ---------- Conexões e árvore ---------- */
  const [connections, setConnections] = useState<ConnectionMock[]>(loadConnections);
  /* ---------- Perfis no atlas.db (app desktop) ---------- */
  const [profilesLoaded, setProfilesLoaded] = useState(!isDesktop);
  const persistedProfiles = useRef(new Map<string, string>());

  useEffect(() => {
    if (!isDesktop) return;
    void (async () => {
      try {
        const status = await local.status();
        if (status.error) {
          setDialog({ kind: "error", title: t("local.unavailableTitle"), message: `${status.error}\n\n${status.path}` });
          return;
        }
        let records = await local.profiles();
        // Migração única dos perfis antigos (guardados no WebView, sem senha) para o atlas.db.
        const legacy = readLegacyProfiles();
        if (!records.length && legacy.length) {
          for (const [i, p] of legacy.entries()) await local.saveProfile(toRecord(profileToConnection(p), i), null, false);
          records = await local.profiles();
        }
        try {
          localStorage.removeItem(CONNECTIONS_KEY);
        } catch {
          /* sem armazenamento do WebView: nada a limpar */
        }
        const conns = records.map(fromRecord);
        persistedProfiles.current = new Map(conns.map((c, i) => [c.id, profileKey(toRecord(c, i))]));
        setConnections(conns);
        const savedLayout = await local.getPref<Partial<Layout>>("layout");
        if (savedLayout) setLayout((l) => ({ ...l, ...savedLayout }));
      } catch (e) {
        setDialog({ kind: "error", title: t("local.unavailableTitle"), message: errorText(toAtlasError(e)) });
      } finally {
        setProfilesLoaded(true);
      }
    })();
    // Uma vez na abertura do app.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sincroniza mudanças de perfil (nome, cor, estrela, ordem...) com o atlas.db. A senha não
  // passa por aqui: só o diálogo de conexão grava ou remove credenciais.
  useEffect(() => {
    if (!isDesktop) {
      saveConnections(connections);
      return;
    }
    if (!profilesLoaded) return;
    const next = new Map(connections.map((c, i) => [c.id, toRecord(c, i)]));
    for (const [id, rec] of next) {
      const key = profileKey(rec);
      if (persistedProfiles.current.get(id) === key) continue;
      persistedProfiles.current.set(id, key);
      void local.saveProfile(rec, null, true).catch((e) => setDialog({ kind: "error", title: rec.name, message: errorText(toAtlasError(e)) }));
    }
    for (const id of [...persistedProfiles.current.keys()]) {
      if (next.has(id)) continue;
      persistedProfiles.current.delete(id);
      void local.deleteProfile(id).catch((e) => setDialog({ kind: "error", title: t("cmd.deleteConnection"), message: errorText(toAtlasError(e)) }));
    }
  }, [connections, profilesLoaded, t]);

  // Layout também vai para o atlas.db (o localStorage fica só como cópia para abrir sem piscar).
  useEffect(() => {
    if (!isDesktop || !profilesLoaded) return;
    const timer = window.setTimeout(() => void local.setPref("layout", layout).catch(() => undefined), 400);
    return () => window.clearTimeout(timer);
  }, [layout, profilesLoaded]);

  const [selectedNode, setSelectedNode] = useState<string | null>("root");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [openConnections, setOpenConnections] = useState<Set<string>>(new Set());
  const [openDatabases, setOpenDatabases] = useState<Set<string>>(new Set());
  const [connecting, setConnecting] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");
  const [category, setCategory] = useState<Category>("table");
  const [selectedObjects, setSelectedObjects] = useState<string[]>([]);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  const [recents, setRecents] = useState<RecentItem[]>([]);
  const [recentProviders, setRecentProviders] = useState<ProviderId[]>(["postgresql"]);

  const [starredOnly, setStarredOnly] = useState(false);
  const nodes = useMemo(
    () => buildVisibleTree(connections, { expanded, openConnections, openDatabases, filter, starredOnly }, (c) => t(FOLDER_LABEL[c] ?? CATEGORY_LABEL[c]), t("nav.myConnections")),
    [connections, expanded, openConnections, openDatabases, filter, starredOnly, t],
  );

  const add = (set: Set<string>, id: string) => new Set(set).add(id);
  const remove = (set: Set<string>, id: string) => {
    const s = new Set(set);
    s.delete(id);
    return s;
  };

  /* ---------- Banco real: carga preguiçosa por nível ---------- */
  const [loadingNodes, setLoadingNodes] = useState<Set<string>>(new Set());
  const connectionsRef = useRef(connections);
  connectionsRef.current = connections;
  const openRef = useRef(openConnections);
  openRef.current = openConnections;

  const showError = (title: string, e: unknown) => setDialog({ kind: "error", title, message: errorText(toAtlasError(e)) });

  const updateConn = (connId: string, fn: (c: ConnectionMock) => ConnectionMock) =>
    setConnections((list) => list.map((c) => (c.id === connId ? fn(c) : c)));

  const withLoading = async <T,>(nodeId: string, job: () => Promise<T>): Promise<T> => {
    setLoadingNodes((s) => add(s, nodeId));
    try {
      return await job();
    } finally {
      setLoadingNodes((s) => remove(s, nodeId));
    }
  };

  /** Abre a sessão no servidor e lê bancos/roles. Rejeita se a senha ainda não é conhecida. */
  const openSession = async (connId: string) => {
    if (openRef.current.has(connId)) return;
    const conn = connectionsRef.current.find((c) => c.id === connId);
    if (!conn) return;
    // Sem senha digitada nesta sessão, o núcleo usa a do cofre (profileId); ela não vem para a UI.
    const info = await pg.open(connId, paramsFor(conn), conn.id);
    const dbs = await pg.listDatabases(connId);
    const roles = await pg.listRoles(connId).catch(() => []);
    updateConn(connId, (c) => ({
      ...c,
      serverVersion: info.version,
      serverVersionNum: info.versionNum,
      hiddenDatabases: dbs.hidden,
      roles: roles.map((r) => ({ name: r.name, canLogin: r.canLogin, superuser: r.superuser })),
      databases: dbs.databases.map((name) => c.databases.find((d) => d.name === name) ?? { name, schemas: [], loaded: false }),
    }));
    openRef.current = add(openRef.current, connId);
    setOpenConnections((s) => add(s, connId));
  };

  const connect = (connId: string) => {
    if (connecting.has(connId)) return;
    const conn = connections.find((c) => c.id === connId);
    if (conn?.live) {
      if (!sessionPasswords.has(connId) && !conn.hasPassword) {
        setDialog({ kind: "password", connId });
        return;
      }
      setConnecting((s) => add(s, connId));
      void openSession(connId)
        .then(() => setExpanded((s) => add(s, `conn:${seg(connId)}`)))
        .catch((e) => {
          const err = toAtlasError(e);
          // Senha recusada: pede de novo em vez de repetir a tentativa (spec §26.2).
          if (err.kind === "authenticationFailed") {
            const usedSaved = !sessionPasswords.has(connId) && !!conn.hasPassword;
            sessionPasswords.delete(connId);
            setDialog({ kind: "password", connId, retry: usedSaved ? `${errorText(err)}\n\n${t("conn.savedPasswordRejected")}` : errorText(err) });
          } else showError(t("cmd.openConnection"), err);
        })
        .finally(() => setConnecting((s) => remove(s, connId)));
      return;
    }
    // Modo de demonstração.
    setConnecting((s) => add(s, connId));
    window.setTimeout(() => {
      setConnecting((s) => remove(s, connId));
      setOpenConnections((s) => add(s, connId));
      setExpanded((s) => add(s, `conn:${seg(connId)}`));
    }, 450);
  };

  const disconnect = (connId: string) => {
    if (connections.find((c) => c.id === connId)?.live) {
      void pg.close(connId).catch(() => undefined);
      // Ao reabrir, a estrutura é lida de novo do servidor.
      updateConn(connId, (c) => ({ ...c, databases: [] }));
    }
    openRef.current = remove(openRef.current, connId);
    setOpenConnections((s) => remove(s, connId));
    const prefix = `db:${seg(connId)}/`;
    setOpenDatabases((s) => new Set([...s].filter((x) => !x.startsWith(prefix))));
  };

  const loadDatabase = async (connId: string, dbName: string, force = false) => {
    const conn = connectionsRef.current.find((c) => c.id === connId);
    const db = conn?.databases.find((d) => d.name === dbName);
    if (!conn?.live || !db || (db.loaded && !force)) return;
    const names = await withLoading(`db:${seg(connId)}/${seg(dbName)}`, () => pg.listSchemas(connId, dbName));
    updateConn(connId, (c) => ({
      ...c,
      databases: c.databases.map((d) => (d.name !== dbName ? d : {
        ...d, loaded: true,
        schemas: names.map((n) => (force ? undefined : d.schemas.find((s) => s.name === n)) ?? { name: n, objects: {}, loaded: false }),
      })),
    }));
  };

  const loadSchema = async (connId: string, dbName: string, schemaName: string, force = false) => {
    const conn = connectionsRef.current.find((c) => c.id === connId);
    const sc = conn?.databases.find((d) => d.name === dbName)?.schemas.find((s) => s.name === schemaName);
    if (!conn?.live || !sc || (sc.loaded && !force)) return;
    const objs = await withLoading(`schema:${seg(connId)}/${seg(dbName)}/${seg(schemaName)}`, () => pg.listObjects(connId, dbName, schemaName));
    const fresh = schemaFromServer(schemaName, objs);
    updateConn(connId, (c) => ({
      ...c,
      databases: c.databases.map((d) => (d.name !== dbName ? d : { ...d, schemas: d.schemas.map((s) => (s.name === schemaName ? fresh : s)) })),
    }));
  };

  const loadTableColumns = async (connId: string, dbName: string, schemaName: string, table: string) => {
    const detail = await withLoading(`obj:${seg(connId)}/${seg(dbName)}/${seg(schemaName)}/table/${seg(table)}`, () =>
      pg.tableDetail(connId, dbName, schemaName, table),
    );
    updateConn(connId, (c) => ({
      ...c,
      databases: c.databases.map((d) => (d.name !== dbName ? d : {
        ...d,
        schemas: d.schemas.map((s) => (s.name !== schemaName ? s : {
          ...s,
          objects: { ...s.objects, table: (s.objects.table ?? []).map((o) => (o.name === table ? tableFromServer(o, detail) : o)) },
        })),
      })),
    }));
  };

  const toggle = (node: TreeNode) => {
    const c = resolveNode(node.id, connections);
    if (node.kind === "connection") {
      const connId = connIdOf(node.id);
      if (!openConnections.has(connId)) return connect(connId);
    }
    if (node.kind === "database" && !openDatabases.has(node.id)) {
      const open = () => {
        setOpenDatabases((s) => add(s, node.id));
        setExpanded((s) => add(s, node.id));
      };
      if (c.connection?.live && c.database) {
        void loadDatabase(c.connection.id, c.database.name).then(open).catch((e) => showError(t("cmd.openDatabase"), e));
      } else open();
      return;
    }
    const expanding = !expanded.has(node.id);
    if (expanding && c.connection?.live && c.database) {
      if (node.kind === "schema" && c.schema && !c.schema.loaded) {
        void loadSchema(c.connection.id, c.database.name, c.schema.name)
          .then(() => setExpanded((s) => add(s, node.id)))
          .catch((e) => showError(c.schema!.name, e));
        return;
      }
      if (node.kind === "object" && node.category === "table" && c.schema && c.objectName && !c.object?.columns) {
        void loadTableColumns(c.connection.id, c.database.name, c.schema.name, c.objectName)
          .then(() => setExpanded((s) => add(s, node.id)))
          .catch((e) => showError(c.objectName!, e));
        return;
      }
    }
    setExpanded((s) => (s.has(node.id) ? remove(s, node.id) : add(s, node.id)));
  };

  const refreshNode = (node: TreeNode) => {
    const c = resolveNode(node.id, connections);
    if (!c.connection?.live) return;
    const connId = c.connection.id;
    const fail = (e: unknown) => showError(t("cmd.refresh"), e);
    if (node.kind === "connection") {
      void withLoading(node.id, async () => {
        const dbs = await pg.listDatabases(connId);
        updateConn(connId, (cc) => ({
          ...cc, hiddenDatabases: dbs.hidden,
          databases: dbs.databases.map((name) => cc.databases.find((d) => d.name === name) ?? { name, schemas: [], loaded: false }),
        }));
      }).catch(fail);
    } else if (node.kind === "database" && c.database) {
      void loadDatabase(connId, c.database.name, true).catch(fail);
    } else if ((node.kind === "schema" || node.kind === "folder" || node.kind === "object") && c.database && c.schema) {
      void loadSchema(connId, c.database.name, c.schema.name, true).catch(fail);
    }
  };

  const selectNode = (id: string) => {
    setSelectedNode(id);
    const c = resolveNode(id, connections);
    // Selecionar schema/pasta de uma conexão real já carrega a lista de objetos (como a referência).
    if ((id.startsWith("schema:") || id.startsWith("folder:")) && c.connection?.live && c.database && c.schema && !c.schema.loaded) {
      void loadSchema(c.connection.id, c.database.name, c.schema.name).catch((e) => showError(c.schema!.name, e));
    }
    if (id.startsWith("folder:") && c.category) {
      setCategory(c.category);
      setSelectedObjects([]);
    } else if (id.startsWith("obj:") && c.category && c.objectName) {
      setCategory(c.category);
      setSelectedObjects([c.objectName]);
    } else if (id.startsWith("col:") && c.objectName) {
      setCategory("table");
      setSelectedObjects([c.objectName]);
    } else {
      setSelectedObjects([]);
    }
  };

  /* ---------- Documentos/abas ---------- */
  const [tabs, setTabs] = useState<DocTab[]>([OBJECTS_TAB]);
  const [activeTab, setActiveTab] = useState("objects");
  const [docStatus, setDocStatus] = useState<Record<string, StatusCell[]>>({});
  const queryActions = useRef<QueryActions | null>(null);
  const tabSeq = useRef(0);

  const ctx = resolveNode(selectedNode, connections);
  const connectionOpen = !!ctx.connection && openConnections.has(ctx.connection.id);

  const openTab = (tab: Omit<DocTab, "id">, recentLabel?: string) => {
    // Reaproveita a aba se o mesmo objeto já estiver aberto no mesmo modo.
    const existing = tab.object && tabs.find((x) => x.kind === tab.kind && x.connectionId === tab.connectionId && x.database === tab.database && x.schema === tab.schema && x.object === tab.object);
    if (existing) {
      setActiveTab(existing.id);
      return;
    }
    const id = `doc${++tabSeq.current}`;
    setTabs((ts) => [...ts, { ...tab, id }]);
    setActiveTab(id);
    if (recentLabel) setRecents((r) => [{ label: recentLabel, tab }, ...r.filter((x) => x.label !== recentLabel)].slice(0, 10));
  };

  const objectTitle = (name: string, kind: MessageKey, c = ctx) =>
    c.database && c.schema && c.connection ? `${name} @${c.database.name}.${c.schema.name} (${c.connection.name}) - ${t(kind)}` : name;

  /** `forConn`: abre a consulta já apontando para essa conexão, mesmo fechada (abre ao executar). */
  const newQuery = (sqlText?: string, forConn?: ConnectionMock) => {
    if (forConn) {
      openTab({
        kind: "query", title: t("tab.untitledQuery"), windowTitle: t("tab.untitledQuery"), icon: "query",
        connectionId: forConn.id, database: forConn.initialDatabase || "postgres", schema: "public", sql: sqlText,
      });
      return;
    }
    const conn = ctx.connection && openConnections.has(ctx.connection.id) ? ctx.connection : undefined;
    const db = conn ? ctx.database ?? conn.databases[0] : undefined;
    openTab({
      kind: "query", title: t("tab.untitledQuery"), windowTitle: t("tab.untitledQuery"), icon: "query",
      connectionId: conn?.id, database: db?.name, schema: ctx.schema?.name ?? db?.schemas[0]?.name, sql: sqlText,
    });
  };

  /* ---------- Ações do menu da conexão ---------- */
  const duplicateConnection = (conn: ConnectionMock) => {
    const id = `c${Date.now().toString(36)}`;
    const copy: ConnectionMock = {
      ...conn, id, name: t("conn.copyName", { name: conn.name }),
      databases: conn.live ? [] : conn.databases, starred: false,
      // A credencial no cofre é por perfil: a cópia começa sem senha salva.
      hasPassword: false,
    };
    const pw = sessionPasswords.get(conn.id);
    if (pw !== undefined) sessionPasswords.set(id, pw);
    setConnections((list) => {
      const i = list.findIndex((c) => c.id === conn.id);
      return [...list.slice(0, i + 1), copy, ...list.slice(i + 1)];
    });
    setSelectedNode(`conn:${seg(id)}`);
  };

  const sqlFileInput = useRef<HTMLInputElement>(null);
  const sqlFileTarget = useRef<{ connId: string; database: string } | null>(null);
  const pickSqlFile = (connId: string, database: string) => {
    sqlFileTarget.current = { connId, database };
    if (sqlFileInput.current) {
      sqlFileInput.current.value = "";
      sqlFileInput.current.click();
    }
  };
  const onSqlFilePicked = async (file: File | undefined) => {
    const target = sqlFileTarget.current;
    if (!file || !target) return;
    if (file.size > MAX_SQL_FILE_BYTES) {
      setDialog({ kind: "error", title: t("cmd.executeSqlFile"), message: t("msg.execFileTooLarge", { mb: MAX_SQL_FILE_BYTES / 1024 / 1024 }) });
      return;
    }
    setDialog({ kind: "execFile", ...target, fileName: file.name, size: file.size, sql: await file.text() });
  };

  const runSqlFile = async (d: Extract<Dialog, { kind: "execFile" }>) => {
    setDialog({ ...d, busy: true, error: undefined });
    try {
      const out = await pg.execute(d.connId, d.database, d.sql, 1);
      const affected = out.statements.reduce((s, st) => s + (st.result ? 0 : st.rowsAffected ?? 0), 0);
      setDialog({
        kind: "info", title: t("cmd.executeSqlFile"),
        message: t("msg.execFileDone", { n: out.statements.length, rows: affected, s: (out.elapsedMs / 1000).toFixed(2) }),
      });
      const conn = connectionsRef.current.find((c) => c.id === d.connId);
      if (conn?.databases.find((x) => x.name === d.database)?.loaded) void loadDatabase(d.connId, d.database, true).catch(() => undefined);
    } catch (e) {
      // Sem transação explícita no arquivo, comandos anteriores ao erro já foram efetivados.
      setDialog({ ...d, busy: false, error: `${errorText(toAtlasError(e))}\n\n${t("msg.execFilePartial")}` });
    }
  };

  const askDdl = (req: DdlRequest) => setDialog({ kind: "ddl", ...req });
  const askName = (title: string, label: string, initial: string, onSubmit: (value: string) => void) =>
    setDialog({ kind: "name", title, label, initial, onSubmit });

  const runDdl = async (d: Extract<Dialog, { kind: "ddl" }>) => {
    setDialog({ ...d, busy: true, error: undefined });
    try {
      if (d.closeDbFirst) await pg.closeDatabase(d.connId, d.closeDbFirst);
      if (d.transactional === false) {
        for (const sql of d.statements) await pg.execute(d.connId, d.database, sql, 1);
      } else {
        // Vários comandos (ex.: duplicar estrutura + dados) entram numa transação só.
        await pg.apply(d.connId, d.database, d.statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      }
      setDialog(null);
      d.after?.();
      const fail = (e: unknown) => showError(t("cmd.refresh"), e);
      if (d.refresh === "schema" && d.schema) void loadSchema(d.connId, d.database, d.schema, true).catch(fail);
      else if (d.refresh === "database") void loadDatabase(d.connId, d.database, true).catch(fail);
      else {
        const node = nodes.find((n) => n.id === `conn:${seg(d.connId)}`);
        if (node) refreshNode(node);
      }
    } catch (e) {
      setDialog({ ...d, busy: false, error: errorText(toAtlasError(e)) });
    }
  };

  const createDatabase = async (d: Extract<Dialog, { kind: "newDatabase" }>) => {
    const conn = connectionsRef.current.find((c) => c.id === d.connId);
    if (!conn || !d.name) return;
    setDialog({ ...d, busy: true, error: undefined });
    try {
      // CREATE DATABASE não roda dentro de transação: executa isolado.
      await pg.execute(conn.id, conn.initialDatabase || "postgres", `CREATE DATABASE ${quoteIdent(d.name)}`, 1);
      setDialog(null);
      const node = nodes.find((n) => n.id === `conn:${seg(conn.id)}`);
      if (node) refreshNode(node);
    } catch (e) {
      setDialog({ ...d, busy: false, error: errorText(toAtlasError(e)) });
    }
  };

  const newTable = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "tb.table");
    openTab({ kind: "designer", title, windowTitle: title, icon: "tableDesign", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  const newView = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "tb.view");
    openTab({ kind: "viewDesigner", title, windowTitle: title, icon: "view", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  const openObjects = (kind: "open" | "design" | "new", names: string[], cat: Category = category, c = ctx) => {
    if (kind === "new") {
      if (cat === "query") return newQuery();
      if (cat === "table") return newTable();
      if (cat === "view") return newView();
      return openTab({ kind: "placeholder", title: `${t("act.new")} ${t(CATEGORY_LABEL[cat])}`, icon: cat, note: t("placeholder.designer") });
    }
    for (const name of names) {
      const base = { connectionId: c.connection?.id, database: c.database?.name, schema: c.schema?.name, object: name };
      if (cat === "table" && kind === "open") {
        const title = objectTitle(name, "tb.table", c);
        openTab({ ...base, kind: "data", title, windowTitle: title, icon: "table" }, `${t("act.openTable")} - ${name}`);
      } else if (cat === "table" && kind === "design") {
        const title = objectTitle(name, "tb.table", c);
        openTab({ ...base, kind: "designer", title, windowTitle: title, icon: "tableDesign" }, `${t("act.designTable")} - ${name}`);
      } else if (cat === "view" && kind === "open") {
        const title = objectTitle(name, "tb.view", c);
        openTab({ ...base, kind: "data", objectKind: "view", title, windowTitle: title, icon: "view" }, `${t("act.openView")} - ${name}`);
      } else if (cat === "view" && kind === "design") {
        const title = objectTitle(name, "tb.view", c);
        openTab({ ...base, kind: "viewDesigner", title, windowTitle: title, icon: "view" }, `${t("act.designView")} - ${name}`);
      } else {
        const title = objectTitle(name, CATEGORY_LABEL[cat], c);
        openTab({ ...base, kind: "placeholder", title, windowTitle: title, icon: cat, note: kind === "open" && (cat === "view" || cat === "matview") ? t("placeholder.viewData") : t("placeholder.designer") });
      }
    }
  };

  const activate = (node: TreeNode) => {
    if (node.kind === "object" && node.category) {
      const c = resolveNode(node.id, connections);
      if (c.objectName) openObjects(node.category === "function" ? "design" : "open", [c.objectName], node.category, c);
      return;
    }
    if (node.kind === "root" || node.kind === "column") return;
    if (node.kind === "connection" && !openConnections.has(connIdOf(node.id))) return connect(connIdOf(node.id));
    toggle(node);
  };

  const doCloseTab = (id: string) => {
    if (id === "objects") return;
    setTabs((ts) => {
      const idx = ts.findIndex((x) => x.id === id);
      const next = ts.filter((x) => x.id !== id);
      if (activeTab === id) setActiveTab(next[Math.max(0, idx - 1)]?.id ?? "objects");
      return next;
    });
    setDocStatus((m) => {
      const n = { ...m };
      delete n[id];
      return n;
    });
  };

  const closeTab = (id: string) => {
    const tab = tabs.find((x) => x.id === id);
    if (tab?.dirty) setDialog({ kind: "closeDirty", tabId: id });
    else doCloseTab(id);
  };

  const cycleTab = (dir: 1 | -1) => {
    const idx = tabs.findIndex((x) => x.id === activeTab);
    setActiveTab(tabs[(idx + dir + tabs.length) % tabs.length].id);
  };

  const setTabDirty = useCallback((id: string, dirty: boolean) => setTabs((ts) => (ts.some((x) => x.id === id && !!x.dirty !== dirty) ? ts.map((x) => (x.id === id ? { ...x, dirty } : x)) : ts)), []);
  const statusSetters = useRef<Record<string, (cells: StatusCell[]) => void>>({});
  const statusFor = (id: string) => (statusSetters.current[id] ??= (cells: StatusCell[]) => setDocStatus((m) => ({ ...m, [id]: cells })));

  /* ---------- Salvar estrutura de tabela (mock) ---------- */
  const saveTable = (tab: DocTab, obj: DbObject, previousName: string | null) => {
    setConnections((list) => list.map((c) => {
      if (c.id !== tab.connectionId) return c;
      return {
        ...c,
        databases: c.databases.map((d) => d.name !== tab.database ? d : {
          ...d,
          schemas: d.schemas.map((s) => {
            if (s.name !== tab.schema) return s;
            const tables = s.objects.table ?? [];
            const prev = previousName ? tables.find((x) => x.name === previousName) : undefined;
            const merged: DbObject = { ...prev, ...obj, rows: prev?.rows ?? 0 };
            const next = previousName ? tables.map((x) => (x.name === previousName ? merged : x)) : [...tables, merged];
            return { ...s, objects: { ...s.objects, table: next.sort((a, b) => a.name.localeCompare(b.name)) } };
          }),
        }),
      };
    }));
    const c = resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections);
    const title = objectTitle(obj.name, "tb.table", c);
    setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: obj.name, title, windowTitle: title } : x)));
  };

  /** View salva no designer: atualiza a árvore (demonstração) e o título da aba. */
  const saveView = (tab: DocTab, name: string, definition: string, comment: string, owner: string) => {
    setConnections((list) => list.map((c) => {
      if (c.id !== tab.connectionId) return c;
      return {
        ...c,
        databases: c.databases.map((d) => d.name !== tab.database ? d : {
          ...d,
          schemas: d.schemas.map((s) => {
            if (s.name !== tab.schema) return s;
            const views = s.objects.view ?? [];
            const prev = views.find((x) => x.name === name);
            const merged: DbObject = { ...prev, name, definition, comment, owner: owner || prev?.owner };
            const next = prev ? views.map((x) => (x.name === name ? merged : x)) : [...views, merged];
            return { ...s, objects: { ...s.objects, view: next.sort((a, b) => a.name.localeCompare(b.name)) } };
          }),
        }),
      };
    }));
    const c = resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections);
    const title = objectTitle(name, "tb.view", c);
    setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
  };

  /* ---------- Conexões: salvar/excluir ---------- */
  const saveConnection = (form: ConnectionForm, version: number | null, editing?: ConnectionMock) => {
    const base = {
      name: form.name, provider: form.provider, host: form.host, port: Number(form.port) || 0, user: form.user,
      initialDatabase: form.initialDatabase, environment: form.environment,
      sslMode: (form.useSsl ? form.sslMode : "disable") as ConnectionMock["sslMode"],
    };
    const id = editing?.id ?? `c${Date.now().toString(36)}`;
    // "Salvar senha" marcado: vai para o cofre do sistema (app desktop). Desmarcado: só na memória
    // desta sessão, para conectar agora sem gravar em lugar nenhum.
    const keepInMemory = () => {
      if (!isDesktop && form.savePassword && form.password) sessionPasswords.set(id, form.password);
      else if (!form.savePassword && form.password) sessionPasswords.set(id, form.password);
      else sessionPasswords.delete(id);
    };
    const hasPassword = form.savePassword && (!!form.password || !!editing?.hasPassword);
    const apply = () => {
      keepInMemory();
      if (editing) {
        setConnections((list) => list.map((c) => (c.id === id ? { ...c, ...base, hasPassword } : c)));
      } else {
        const conn = { ...profileToConnection({ id, ...base }), hasPassword };
        if (version) {
          conn.serverVersionNum = version;
          conn.serverVersion = `${Math.floor(version / 10000)}.${version % 100}`;
        }
        setConnections((list) => [...list, conn]);
        setSelectedNode(`conn:${seg(id)}`);
      }
      setRecentProviders((r) => [form.provider, ...r.filter((p) => p !== form.provider)].slice(0, 3));
      setDialog(null);
    };
    if (!isDesktop) return apply();

    const current = connections.find((c) => c.id === id);
    const record = toRecord({ ...(current ?? profileToConnection({ id, ...base })), ...base, hasPassword }, current ? connections.indexOf(current) : connections.length);
    // Marca como persistido antes de atualizar o estado, para a sincronização não regravar sem a senha.
    persistedProfiles.current.set(id, profileKey(record));
    void local
      .saveProfile(record, form.savePassword ? form.password || null : null, form.savePassword)
      .then(apply)
      .catch((e) => {
        persistedProfiles.current.delete(id);
        showError(t("cmd.editConnection"), e);
      });
  };

  /* ---------- Menus de contexto ---------- */
  const showContextMenu = (node: TreeNode, x: number, y: number) => {
    const c = resolveNode(node.id, connections);
    const items: MenuEntry[] = [];
    const item = (text: MessageKey, run?: () => void): MenuEntry => ({ kind: "item", text: t(text), run });
    const sep: MenuEntry = { kind: "sep" };
    if (node.kind === "root") items.push(item("cmd.newConnection", () => setDialog({ kind: "connection" })));
    if (node.kind === "connection" && c.connection) {
      const conn = c.connection;
      const isOpen = openConnections.has(conn.id);
      const live = !!conn.live;
      const setColor = (color?: ConnectionColor) => updateConn(conn.id, (x) => ({ ...x, color }));
      // Mesma ordem e agrupamento do menu da referência (CONFIRMADO_EM_CAPTURA).
      items.push(
        isOpen ? item("cmd.closeConnection", () => disconnect(conn.id)) : item("cmd.openConnection", () => connect(conn.id)),
        {
          kind: "sub", label: "cmd.switchProfile",
          items: [{ kind: "item", text: `${t("conn.mainProfile")} (${t("conn.active")})`, checked: true, run: () => undefined }],
        },
        sep,
        item("cmd.editConnection", () => setDialog({ kind: "connection", editing: conn })),
        item("cmd.newConnection", () => setDialog({ kind: "connection" })),
        item("cmd.deleteConnection", () => setDialog({ kind: "deleteConnection", conn })),
        item("cmd.duplicateConnection", () => duplicateConnection(conn)),
        sep,
        item("cmd.newDatabase", isOpen && live ? () => setDialog({ kind: "newDatabase", connId: conn.id }) : undefined),
        item("cmd.newQuery", () => (isOpen ? newQuery() : newQuery(undefined, conn))),
        sep,
        item("cmd.console"),
        item("cmd.executeSqlFile", isOpen && live ? () => pickSqlFile(conn.id, conn.initialDatabase || "postgres") : undefined),
        sep,
        item(conn.starred ? "cmd.removeStar" : "cmd.addStar", () => updateConn(conn.id, (x) => ({ ...x, starred: !x.starred }))),
        {
          kind: "sub", label: "sub.color",
          items: [
            { kind: "item", text: t("color.none"), checked: !conn.color || undefined, run: () => setColor(undefined) },
            ...CONNECTION_COLORS.map<MenuEntry>((col) => ({
              kind: "item", text: t(`color.${col}` as MessageKey), icon: "colorDot", iconColor: COLOR_HEX[col],
              checked: conn.color === col || undefined, run: () => setColor(col),
            })),
          ],
        },
        sep,
        item("cmd.share"),
        sep,
        item("cmd.refresh", isOpen && live ? () => refreshNode(node) : undefined),
      );
    }
    // Menus de banco/schema/tabela: estrutura da linha 17 da referência, ainda sem captura
    // para conferir (DECISÃO_ATLAS) — ordem e nomes serão ajustados quando houver as imagens.
    const live = !!c.connection?.live;
    const prod = c.connection?.environment === "production";
    const dumpSub: MenuEntry = { kind: "sub", label: "sub.dumpSql", items: [{ kind: "item", text: t("cmd.dumpStructData") }, { kind: "item", text: t("cmd.dumpStructOnly") }] };

    if (node.kind === "database" && c.connection && c.database) {
      const conn = c.connection;
      const dbName = c.database.name;
      const isOpen = openDatabases.has(node.id);
      const isInitial = dbName === (conn.initialDatabase || "postgres");
      items.push(
        isOpen
          ? item("cmd.closeDatabase", () => {
              setOpenDatabases((s) => remove(s, node.id));
              if (live) void pg.closeDatabase(conn.id, dbName).catch(() => undefined);
            })
          : item("cmd.openDatabase", () => toggle(node)),
        sep,
        item("cmd.newDatabase", live ? () => setDialog({ kind: "newDatabase", connId: conn.id }) : undefined),
        item("cmd.editDatabase"),
        // O banco inicial da conexão não pode ser excluído por ela mesma (a sessão está nele).
        item("cmd.deleteDatabase", live && !isInitial ? () => askDdl({
          title: t("cmd.deleteDatabase"), statements: [`DROP DATABASE ${quoteIdent(dbName)};`],
          connId: conn.id, database: conn.initialDatabase || "postgres", refresh: "connection",
          danger: true, transactional: false, confirmText: dbName, closeDbFirst: dbName,
          after: () => setOpenDatabases((s) => remove(s, node.id)),
        }) : undefined),
        sep,
        item("cmd.newQuery", () => newQuery()),
        item("cmd.console"),
        item("cmd.executeSqlFile", isOpen && live ? () => pickSqlFile(conn.id, dbName) : undefined),
        dumpSub,
        sep,
        item("cmd.reverseToModel"),
        item("cmd.findInDatabase"),
        sep,
        item("cmd.refresh", isOpen && live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "schema" && c.connection && c.database && c.schema) {
      const conn = c.connection;
      const dbName = c.database.name;
      const scName = c.schema.name;
      items.push(
        item("cmd.newSchema", live ? () => askName(t("cmd.newSchema"), t("msg.schemaName"), "", (name) => askDdl({
          title: t("cmd.newSchema"), statements: [`CREATE SCHEMA ${quoteIdent(name)};`],
          connId: conn.id, database: dbName, refresh: "database",
        })) : undefined),
        item("cmd.editSchema"),
        item("cmd.deleteSchema", live ? () => askDdl({
          title: t("cmd.deleteSchema"), statements: [`DROP SCHEMA ${quoteIdent(scName)};`],
          connId: conn.id, database: dbName, refresh: "database", danger: true, confirmText: prod ? scName : undefined,
          note: t("msg.dropSchemaRestrict"),
        }) : undefined),
        sep,
        item("act.newTable", newTable),
        item("cmd.newQuery", () => newQuery()),
        item("cmd.executeSqlFile", live ? () => pickSqlFile(conn.id, dbName) : undefined),
        dumpSub,
        sep,
        item("cmd.reverseToModel"),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "folder" && node.category === "table") {
      items.push(
        item("act.newTable", newTable),
        item("cmd.newQuery", () => newQuery()),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "object" && node.category === "table" && c.connection && c.database && c.schema && c.objectName) {
      const conn = c.connection;
      const dbName = c.database.name;
      const scName = c.schema.name;
      const name = c.objectName;
      const target = `${quoteIdent(scName)}.${quoteIdent(name)}`;
      const scope = { connId: conn.id, database: dbName, schema: scName, refresh: "schema" as const };
      const duplicate = (withData: boolean) => askName(t("sub.duplicateTable"), t("msg.copyTableName"), `${name}_copy`, (copy) => askDdl({
        ...scope, title: t("sub.duplicateTable"),
        statements: [
          `CREATE TABLE ${quoteIdent(scName)}.${quoteIdent(copy)} (LIKE ${target} INCLUDING ALL);`,
          ...(withData ? [`INSERT INTO ${quoteIdent(scName)}.${quoteIdent(copy)} SELECT * FROM ${target};`] : []),
        ],
        note: t("msg.duplicateSequences"),
      }));
      items.push(
        item("act.openTable", () => openObjects("open", [name], "table", c)),
        item("act.designTable", () => openObjects("design", [name], "table", c)),
        item("act.newTable", newTable),
        item("act.deleteTable", live ? () => askDdl({
          ...scope, title: t("act.deleteTable"), statements: [`DROP TABLE ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        item("cmd.emptyTable", live ? () => askDdl({
          ...scope, title: t("cmd.emptyTable"), statements: [`DELETE FROM ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        item("cmd.truncateTable", live ? () => askDdl({
          ...scope, title: t("cmd.truncateTable"), statements: [`TRUNCATE TABLE ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        {
          kind: "sub", label: "sub.duplicateTable",
          items: [
            { kind: "item", text: t("cmd.dupStructData"), run: live ? () => duplicate(true) : undefined },
            { kind: "item", text: t("cmd.dupStructure"), run: live ? () => duplicate(false) : undefined },
          ],
        },
        sep,
        item("act.importWizard"),
        item("act.exportWizard"),
        dumpSub,
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        item("cmd.renameTable", live ? () => askName(t("cmd.renameTable"), t("msg.newName"), name, (newName) => askDdl({
          ...scope, title: t("cmd.renameTable"), statements: [`ALTER TABLE ${target} RENAME TO ${quoteIdent(newName)};`],
        })) : undefined),
        sep,
        item("cmd.newQuery", () => newQuery(`SELECT * FROM ${target} LIMIT 100;`)),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
      );
    }
    if (node.kind === "folder" && node.category === "view") {
      items.push(
        item("act.newView", newView),
        item("cmd.newQuery", () => newQuery()),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "object" && node.category === "view" && c.connection && c.database && c.schema && c.objectName) {
      const scName = c.schema.name;
      const name = c.objectName;
      const target = `${quoteIdent(scName)}.${quoteIdent(name)}`;
      const scope = { connId: c.connection.id, database: c.database.name, schema: scName, refresh: "schema" as const };
      items.push(
        item("act.openView", () => openObjects("open", [name], "view", c)),
        item("act.designView", () => openObjects("design", [name], "view", c)),
        item("act.newView", newView),
        item("act.deleteView", live ? () => askDdl({
          ...scope, title: t("act.deleteView"), statements: [`DROP VIEW ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("act.exportWizard"),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        item("cmd.renameView", live ? () => askName(t("cmd.renameView"), t("msg.newName"), name, (newName) => askDdl({
          ...scope, title: t("cmd.renameView"), statements: [`ALTER VIEW ${target} RENAME TO ${quoteIdent(newName)};`],
        })) : undefined),
        sep,
        item("cmd.newQuery", () => newQuery(`SELECT * FROM ${target} LIMIT 100;`)),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
      );
    }
    if (items.length) setCtxMenu({ x, y, items });
  };

  useEffect(() => {
    if (!ctxMenu) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof MouseEvent && (e.target as HTMLElement).closest?.(".ctx-menu")) return;
      setCtxMenu(null);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
      window.removeEventListener("resize", close);
    };
  }, [ctxMenu]);

  /* ---------- Comandos ---------- */
  const active = tabs.find((x) => x.id === activeTab) ?? OBJECTS_TAB;
  const hasSchema = !!ctx.schema;
  const selConn = ctx.connection;
  const qa = () => queryActions.current;

  const commands: CommandMap = {
    newProject: { id: "newProject", label: "cmd.newProject" },
    newConnection: { id: "newConnection", label: "cmd.newConnection", run: () => setDialog({ kind: "connection" }) },
    newTableNormal: { id: "newTableNormal", label: "cmd.newTableNormal", run: hasSchema ? newTable : undefined },
    newTableForeign: { id: "newTableForeign", label: "cmd.newTableForeign" },
    newView: { id: "newView", label: "cmd.newView", run: hasSchema ? newView : undefined },
    newMatview: { id: "newMatview", label: "cmd.newMatview" },
    newFunction: { id: "newFunction", label: "cmd.newFunction" },
    newRole: { id: "newRole", label: "cmd.newRole" },
    newTablespace: { id: "newTablespace", label: "cmd.newTablespace" },
    newCast: { id: "newCast", label: "cmd.newCast" },
    newForeignServer: { id: "newForeignServer", label: "cmd.newForeignServer" },
    newLanguage: { id: "newLanguage", label: "cmd.newLanguage" },
    newAggregate: { id: "newAggregate", label: "cmd.newAggregate" },
    newConversion: { id: "newConversion", label: "cmd.newConversion" },
    newDomain: { id: "newDomain", label: "cmd.newDomain" },
    newIndex: { id: "newIndex", label: "cmd.newIndex" },
    newOperator: { id: "newOperator", label: "cmd.newOperator" },
    newOperatorClass: { id: "newOperatorClass", label: "cmd.newOperatorClass" },
    newSequence: { id: "newSequence", label: "cmd.newSequence" },
    newTrigger: { id: "newTrigger", label: "cmd.newTrigger" },
    typeBase: { id: "typeBase", label: "cmd.typeBase" },
    typeComposite: { id: "typeComposite", label: "cmd.typeComposite" },
    typeEnum: { id: "typeEnum", label: "cmd.typeEnum" },
    typeRange: { id: "typeRange", label: "cmd.typeRange" },
    newQuery: { id: "newQuery", label: "cmd.newQuery", shortcut: "Ctrl+Q", run: () => newQuery() },
    newQueryFile: { id: "newQueryFile", label: "cmd.newQueryFile", run: () => newQuery() },
    newBackup: { id: "newBackup", label: "cmd.newBackup" },
    newAutomation: { id: "newAutomation", label: "cmd.newAutomation" },
    newModelWorkspace: { id: "newModelWorkspace", label: "cmd.newModelWorkspace" },
    newBiWorkspace: { id: "newBiWorkspace", label: "cmd.newBiWorkspace" },
    openUri: { id: "openUri", label: "cmd.openUri" },
    openQueryFile: { id: "openQueryFile", label: "cmd.newQueryFile" },
    openModelFile: { id: "openModelFile", label: "cmd.newModelWorkspace" },
    openBiFile: { id: "openBiFile", label: "cmd.newBiWorkspace" },
    clearRecents: { id: "clearRecents", label: "cmd.clearRecents", run: recents.length ? () => setRecents([]) : undefined },
    openConnection: { id: "openConnection", label: "cmd.openConnection", run: selConn && !openConnections.has(selConn.id) ? () => connect(selConn.id) : undefined },
    closeConnection: { id: "closeConnection", label: "cmd.closeConnection", run: selConn && openConnections.has(selConn.id) ? () => disconnect(selConn.id) : undefined },
    importConnections: { id: "importConnections", label: "cmd.importConnections" },
    exportConnections: { id: "exportConnections", label: "cmd.exportConnections" },
    closeWindow: { id: "closeWindow", label: "cmd.closeWindow" },
    closeTab: { id: "closeTab", label: "cmd.closeTab", shortcut: "Ctrl+W", run: activeTab !== "objects" ? () => closeTab(activeTab) : undefined },
    exit: { id: "exit", label: "cmd.exit" },
    undo: { id: "undo", label: "cmd.undo", shortcut: "Ctrl+Z" },
    redo: { id: "redo", label: "cmd.redo", shortcut: "Ctrl+Y" },
    cut: { id: "cut", label: "cmd.cut", shortcut: "Ctrl+X" },
    copy: { id: "copy", label: "cmd.copy", shortcut: "Ctrl+C" },
    paste: { id: "paste", label: "cmd.paste", shortcut: "Ctrl+V" },
    selectAll: { id: "selectAll", label: "cmd.selectAll", shortcut: "Ctrl+A" },
    find: { id: "find", label: "cmd.find", shortcut: "Ctrl+F" },
    navigationPane: { id: "navigationPane", label: "cmd.navigationPane", checked: layout.navVisible, run: () => patchLayout({ navVisible: !layout.navVisible }) },
    informationPane: { id: "informationPane", label: "cmd.informationPane", checked: layout.infoVisible, run: () => patchLayout({ infoVisible: !layout.infoVisible }) },
    toolbarCaptions: { id: "toolbarCaptions", label: "cmd.toolbarCaptions", checked: layout.showCaptions, run: () => patchLayout({ showCaptions: !layout.showCaptions }) },
    viewList: { id: "viewList", label: "cmd.viewList", checked: layout.viewMode === "list", run: () => patchLayout({ viewMode: "list" }) },
    viewDetail: { id: "viewDetail", label: "cmd.viewDetail", checked: layout.viewMode === "detail", run: () => patchLayout({ viewMode: "detail" }) },
    viewIcons: { id: "viewIcons", label: "cmd.viewIcons", checked: layout.viewMode === "icons", run: () => patchLayout({ viewMode: "icons" }) },
    themeSystem: { id: "themeSystem", label: "cmd.themeSystem", checked: layout.theme === "system", run: () => patchLayout({ theme: "system" }) },
    themeLight: { id: "themeLight", label: "cmd.themeLight", checked: layout.theme === "light", run: () => patchLayout({ theme: "light" }) },
    themeDark: { id: "themeDark", label: "cmd.themeDark", checked: layout.theme === "dark", run: () => patchLayout({ theme: "dark" }) },
    langPt: { id: "langPt", label: "cmd.langPt", checked: layout.lang === "pt-BR", run: () => patchLayout({ lang: "pt-BR" }) },
    langEn: { id: "langEn", label: "cmd.langEn", checked: layout.lang === "en-US", run: () => patchLayout({ lang: "en-US" }) },
    resetLayout: { id: "resetLayout", label: "cmd.resetLayout", run: () => setLayout({ ...DEFAULT_LAYOUT, lang: layout.lang, theme: layout.theme }) },
    addFavorite: { id: "addFavorite", label: "cmd.addFavorite" },
    manageFavorites: { id: "manageFavorites", label: "cmd.manageFavorites" },
    dataTransfer: { id: "dataTransfer", label: "cmd.dataTransfer" },
    dataSync: { id: "dataSync", label: "cmd.dataSync" },
    structureSync: { id: "structureSync", label: "cmd.structureSync" },
    serverMonitor: { id: "serverMonitor", label: "cmd.serverMonitor" },
    options: { id: "options", label: "cmd.options" },
    nextTab: { id: "nextTab", label: "cmd.nextTab", shortcut: "Ctrl+Tab", run: () => cycleTab(1) },
    previousTab: { id: "previousTab", label: "cmd.previousTab", shortcut: "Ctrl+Shift+Tab", run: () => cycleTab(-1) },
    closeAllTabs: { id: "closeAllTabs", label: "cmd.closeAllTabs", run: tabs.length > 1 ? () => { setTabs([OBJECTS_TAB]); setActiveTab("objects"); } : undefined },
    documentation: { id: "documentation", label: "cmd.documentation" },
    about: { id: "about", label: "cmd.about", run: () => setDialog({ kind: "about" }) },
    run: { id: "run", label: "cmd.run", shortcut: "Ctrl+R", run: active.kind === "query" ? () => qa()?.run(false) : undefined },
    runSelected: { id: "runSelected", label: "cmd.runSelected", shortcut: "Ctrl+Enter", run: active.kind === "query" ? () => qa()?.run(true) : undefined },
    stop: { id: "stop", label: "cmd.stop", run: active.kind === "query" ? () => qa()?.stop() : undefined },
    explain: { id: "explain", label: "cmd.explain", run: active.kind === "query" ? () => qa()?.explain() : undefined },
    beautify: { id: "beautify", label: "cmd.beautify", shortcut: "Ctrl+Shift+F", run: active.kind === "query" ? () => qa()?.beautify() : undefined },
    queryBuilder: { id: "queryBuilder", label: "cmd.queryBuilder" },
    indent: { id: "indent", label: "cmd.indent", shortcut: "Tab" },
    outdent: { id: "outdent", label: "cmd.outdent", shortcut: "Shift+Tab" },
    toggleComment: { id: "toggleComment", label: "cmd.comment", shortcut: "Ctrl+/" },
    upper: { id: "upper", label: "cmd.upper" },
    lower: { id: "lower", label: "cmd.lower" },
    addRecord: { id: "addRecord", label: "data.addRecord" },
    deleteRecord: { id: "deleteRecord", label: "data.deleteRecord" },
    applyChanges: { id: "applyChanges", label: "data.apply" },
    discardChanges: { id: "discardChanges", label: "data.discard" },
    refreshData: { id: "refreshData", label: "data.refresh" },
  };

  const recentEntries: MenuEntry[] = recents.length
    ? recents.map((r, i) => ({ kind: "item" as const, text: `${i + 1}. ${r.label}`, run: () => openTab(r.tab, r.label) }))
    : [{ kind: "item", text: t("cmd.noRecents") }];

  const fileMenu: MenuDef = {
    label: "menu.file",
    items: entries(
      "newProject", "newConnection",
      sub("sub.new",
        sub("sub.table", "newTableNormal", "newTableForeign"),
        "newView", "newMatview", "newFunction", "newRole",
        sub("sub.others", "newTablespace", "-", "newCast", "newForeignServer", "newLanguage", "-", "newAggregate", "newConversion", "newDomain", "newIndex", "newOperator", "newOperatorClass", "newSequence", "newTrigger", sub("sub.type", "typeBase", "typeComposite", "typeEnum", "typeRange")),
        "-", "newQueryFile", "newBackup", "newAutomation", "newModelWorkspace", "newBiWorkspace",
      ),
      "-", "openUri",
      sub("sub.openExternal", "openQueryFile", "openModelFile", "openBiFile"),
      { kind: "sub", label: "sub.openRecent", items: [...recentEntries, { kind: "sep" }, { kind: "cmd", id: "clearRecents" }] },
      "-", "openConnection", "closeConnection", "importConnections", "exportConnections",
      "-", "closeWindow", "closeTab", "exit",
    ),
  };
  const editMenu: MenuDef = { label: "menu.edit", items: entries("undo", "redo", "-", "cut", "copy", "paste", "selectAll", "-", "find") };
  const viewMenu: MenuDef = {
    label: "menu.view",
    items: entries("navigationPane", "informationPane", "toolbarCaptions", "-", "viewList", "viewDetail", "viewIcons", "-", "themeSystem", "themeLight", "themeDark", "-", "langPt", "langEn", "-", "resetLayout"),
  };
  const queryMenu: MenuDef = { label: "menu.query", items: entries("run", "runSelected", "stop", "-", "explain", "-", "beautify", "queryBuilder") };
  const formatMenu: MenuDef = { label: "menu.format", items: entries("indent", "outdent", "-", "toggleComment", "-", "upper", "lower") };
  const tableMenu: MenuDef = { label: "menu.table", items: entries("addRecord", "deleteRecord", "-", "applyChanges", "discardChanges", "-", "refreshData") };
  const tailMenus: MenuDef[] = [
    { label: "menu.favorites", items: entries("addFavorite", "manageFavorites") },
    { label: "menu.tools", items: entries("dataTransfer", "dataSync", "structureSync", "-", "serverMonitor", "-", "options") },
    { label: "menu.window", items: entries("nextTab", "previousTab", "-", "closeTab", "closeAllTabs") },
    { label: "menu.help", items: entries("documentation", "-", "about") },
  ];
  // A barra de menus muda conforme o documento ativo (CONFIRMADO_EM_CAPTURA: Query/Format e Table).
  const menus: MenuDef[] = [
    fileMenu, editMenu, viewMenu,
    ...(active.kind === "query" ? [queryMenu, formatMenu] : active.kind === "data" ? [tableMenu] : []),
    ...tailMenus,
  ];

  const commandsRef = useRef(commands);
  commandsRef.current = commands;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Atalhos de edição e os do editor SQL ficam com o controle focado.
      const local = new Set(["copy", "cut", "paste", "selectAll", "undo", "redo", "find", "indent", "outdent", "toggleComment", "run", "runSelected", "beautify"]);
      for (const cmd of Object.values(commandsRef.current)) {
        if (cmd.run && cmd.shortcut && !local.has(cmd.id) && matchesShortcut(e, cmd.shortcut)) {
          e.preventDefault();
          cmd.run();
          return;
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /* ---------- Título da janela ---------- */
  useEffect(() => {
    document.title = active.kind === "objects" || !active.windowTitle ? "Atlas DB Manager" : `${active.windowTitle} - Atlas DB Manager`;
  }, [active]);

  /* ---------- Splitter do painel de navegação ---------- */
  const startResize = (e: ReactMouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = layout.navWidth;
    const move = (ev: MouseEvent) => {
      const max = Math.min(640, window.innerWidth - 420);
      patchLayout({ navWidth: Math.max(180, Math.min(max, startW + ev.clientX - startX)) });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.classList.remove("is-resizing");
    };
    document.body.classList.add("is-resizing");
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  /* ---------- Status ---------- */
  const objects = objectsFor(category, ctx, connectionOpen);
  const objectsStatus = (): { left: string; cells: StatusCell[] } => {
    const cells: StatusCell[] = [];
    if (ctx.connection) {
      cells.push({ text: <span className="sb-chip"><ProviderIcon provider={ctx.connection.provider} open={connectionOpen} />{ctx.connection.name}</span> });
      if (ctx.database) cells.push({ text: <span className="sb-chip"><SmallIcon name="database" />{ctx.database.name}</span> });
      if (ctx.schema) cells.push({ text: <span className="sb-chip"><SmallIcon name="schema" />{ctx.schema.name}</span> });
    }
    if (selectedNode?.startsWith("conn:") && ctx.connection && connectionOpen) {
      const n = ctx.connection.databases.length;
      const h = ctx.connection.hiddenDatabases;
      return { left: h ? t("status.databasesHidden", { n, h }) : t("status.databases", { n }), cells };
    }
    if (objects) {
      const left = selectedObjects.length > 1 ? t("status.selected", { n: selectedObjects.length }) : t(COUNT_LABEL[category], { n: objects.length });
      return { left, cells };
    }
    return { left: connections.length === 1 ? t("status.connection") : t("status.connections", { n: connections.length }), cells };
  };
  const status = active.kind === "objects" ? objectsStatus() : { left: "", cells: docStatus[active.id] ?? [] };
  const selectedObject = selectedObjects.length === 1 ? objects?.find((o) => o.name === selectedObjects[0]) ?? null : null;

  const tabsForBar = tabs.map((x) => (x.kind === "objects" ? { ...x, title: t("tab.objects") } : x));
  const highlight: Category = active.kind === "query" ? "query"
    : active.kind === "viewDesigner" || (active.kind === "data" && active.objectKind === "view") ? "view"
    : active.kind === "data" || active.kind === "designer" ? "table" : category;
  const toolbarProvider = ctx.connection?.provider ?? connections.find((c) => c.id === active.connectionId)?.provider;

  /* ---------- Backends reais por documento (estáveis por aba) ---------- */
  const queryBackend = useRef<QueryBackend>({
    ensureOpen: async (connId) => {
      if (openRef.current.has(connId)) return;
      const conn = connectionsRef.current.find((c) => c.id === connId);
      if (!sessionPasswords.has(connId) && !conn?.hasPassword) {
        throw { kind: "sessionNotFound", code: null, message: t("conn.needOpen"), detail: null, position: null };
      }
      await openSession(connId);
    },
    execute: (connId, db, sqlText, max) => pg.execute(connId, db, sqlText, max),
    cancel: (connId, db) => pg.cancel(connId, db),
  }).current;

  const dataBackends = useRef<Record<string, TableDataBackend>>({});
  const tableBackendFor = (tab: DocTab, base: DbObject): TableDataBackend => {
    const connId = tab.connectionId ?? "";
    const db = tab.database ?? "";
    const schema = tab.schema ?? "";
    return (dataBackends.current[tab.id] ??= {
      loadDetail: async () => tableFromServer(base, await pg.tableDetail(connId, db, schema, tab.object ?? base.name)),
      loadRows: async (orderBy, limit, offset) => {
        const r = await pg.tableRows(connId, db, schema, tab.object ?? base.name, orderBy, limit, offset);
        const rs = r.outcome.statements[0]?.result;
        return { sql: r.sql, rows: rs?.rows ?? [], truncated: !!rs?.truncated };
      },
      apply: async (statements) => {
        await pg.apply(connId, db, statements);
      },
    });
  };

  /** Fonte do lookup de FK por aba: banco real (SELECT só de leitura) ou dados de demonstração. */
  const lookupSources = useRef<Record<string, FkLookupSource>>({});
  const lookupSourceFor = (tab: DocTab, conn: ConnectionMock | undefined): FkLookupSource | undefined => {
    if (!conn) return undefined;
    const db = tab.database ?? "";
    return (lookupSources.current[tab.id] ??= conn.live
      ? {
          columns: async (schema, table) =>
            (await pg.tableDetail(conn.id, db, schema, table)).columns.map((c) => ({ name: c.name, type: c.dataType })),
          rows: async (schema, table, columns, orderBy, limit) => {
            // Só identificadores (com aspas) e um limite numérico entram no texto; nenhum valor do usuário.
            const sql = `SELECT ${columns.map(quoteIdent).join(", ")} FROM ${quoteIdent(schema)}.${quoteIdent(table)}`
              + (orderBy.length ? ` ORDER BY ${orderBy.map(quoteIdent).join(", ")}` : "") + ` LIMIT ${limit}`;
            const out = await pg.execute(conn.id, db, sql, limit);
            return out.statements[0]?.result?.rows ?? [];
          },
        }
      : {
          columns: async (schema, table) => {
            const obj = connectionsRef.current.find((c) => c.id === conn.id)?.databases.find((d) => d.name === db)
              ?.schemas.find((s) => s.name === schema)?.objects.table?.find((x) => x.name === table);
            return (obj?.columns ?? []).map((c) => ({ name: c.name, type: c.type }));
          },
          rows: async (schema, table, columns, _orderBy, limit) => {
            const obj = connectionsRef.current.find((c) => c.id === conn.id)?.databases.find((d) => d.name === db)
              ?.schemas.find((s) => s.name === schema)?.objects.table?.find((x) => x.name === table);
            if (!obj) return [];
            const idx = columns.map((name) => (obj.columns ?? []).findIndex((c) => c.name === name));
            return mockRows(obj).slice(0, limit).map((r) => idx.map((i) => (i >= 0 ? r[i] : null)));
          },
        });
  };

  /* ---------- Render dos documentos ---------- */
  const lookupTable = (tab: DocTab): { conn?: ConnectionMock; table?: DbObject; tables: DbObject[] } => {
    const conn = connections.find((c) => c.id === tab.connectionId);
    const sc = conn?.databases.find((d) => d.name === tab.database)?.schemas.find((s) => s.name === tab.schema);
    const tables = sc?.objects.table ?? [];
    const own = tab.objectKind === "view" || tab.kind === "viewDesigner" ? sc?.objects.view ?? [] : tables;
    return { conn, table: own.find((x) => x.name === tab.object), tables };
  };

  const viewBackends = useRef<Record<string, ViewBackend>>({});
  const viewBackendFor = (tab: DocTab, conn: ConnectionMock): ViewBackend => {
    const db = tab.database ?? "";
    return (viewBackends.current[tab.id] ??= {
      loadDetail: () => pg.viewDetail(conn.id, db, tab.schema ?? "", tab.object ?? ""),
      execute: (sqlText, maxRows) => pg.execute(conn.id, db, sqlText, maxRows),
      apply: async (statements) => {
        await pg.apply(conn.id, db, statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      },
    });
  };

  const renderDoc = (tab: DocTab) => {
    const isActive = tab.id === activeTab;
    if (tab.kind === "query") {
      return (
        <QueryDoc
          connections={connections}
          openConnections={openConnections}
          initial={{ connectionId: tab.connectionId, database: tab.database, schema: tab.schema }}
          initialSql={tab.sql}
          active={isActive}
          onStatus={statusFor(tab.id)}
          onDirty={(d) => setTabDirty(tab.id, d)}
          registerActions={(a) => { if (isActive) queryActions.current = a; }}
          backend={connections.find((c) => c.id === tab.connectionId)?.live ? queryBackend : undefined}
        />
      );
    }
    if (tab.kind === "data") {
      const { conn, table } = lookupTable(tab);
      const base = table ?? (conn?.live && tab.object ? { name: tab.object } : undefined);
      if (!base) return <div className="doc-placeholder">{t("placeholder.missing")}</div>;
      return (
        <TableDataDoc
          schema={tab.schema ?? ""}
          table={base}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          backend={conn?.live ? tableBackendFor(tab, base) : undefined}
          lookup={tab.objectKind === "view" ? undefined : lookupSourceFor(tab, conn)}
          isView={tab.objectKind === "view"}
        />
      );
    }
    if (tab.kind === "viewDesigner") {
      const { conn, table: view, tables } = lookupTable(tab);
      const completion: Record<string, string[]> = {};
      for (const tb of tables) completion[tb.name] = (tb.columns ?? []).map((c) => c.name);
      return (
        <ViewDesignerDoc
          schema={tab.schema ?? "public"}
          viewName={tab.object}
          mock={view ? { definition: view.definition ?? "", comment: view.comment ?? "", owner: view.owner ?? "" } : undefined}
          owners={conn?.roles.map((r) => r.name) ?? []}
          serverVersionNum={conn?.serverVersionNum ?? 0}
          active={isActive}
          production={conn?.environment === "production"}
          completionSchema={completion}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(name, s) => {
            saveView(tab, name, s.definition, s.comment, s.owner);
            if (conn?.live && tab.database && tab.schema) void loadSchema(conn.id, tab.database, tab.schema, true).catch(() => undefined);
          }}
          backend={conn?.live ? viewBackendFor(tab, conn) : undefined}
        />
      );
    }
    if (tab.kind === "designer") {
      const { conn, table, tables } = lookupTable(tab);
      const liveBackend = conn?.live ? tableBackendFor(tab, table ?? { name: tab.object ?? "" }) : undefined;
      return (
        <TableDesignerDoc
          schema={tab.schema ?? "public"}
          table={tab.object ? table ?? null : null}
          owners={conn?.roles.map((r) => r.name) ?? []}
          otherTables={tables.map((x) => x.name)}
          serverVersionNum={conn?.serverVersionNum ?? 0}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          onSaved={(obj, prev) => {
            saveTable(tab, obj, prev);
            // Relê o schema do servidor para a árvore refletir o DDL aplicado.
            if (conn?.live && tab.database && tab.schema) void loadSchema(conn.id, tab.database, tab.schema, true).catch(() => undefined);
          }}
          loadDetail={liveBackend && tab.object ? liveBackend.loadDetail : undefined}
          applyDdl={conn?.live ? async (statements) => {
            await pg.apply(conn.id, tab.database ?? "", statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
          } : undefined}
        />
      );
    }
    return (
      <div className="doc-placeholder">
        <div className="doc-placeholder-title">{tab.title}</div>
        <div>{tab.note}</div>
      </div>
    );
  };

  return (
    <I18nContext.Provider value={t}>
      <div className="shell">
        <MenuBar menus={menus} commands={commands} />
        <MainToolbar
          highlight={highlight}
          provider={toolbarProvider}
          showCaptions={layout.showCaptions}
          onCategory={(c) => { setCategory(c); setSelectedObjects([]); setActiveTab("objects"); }}
          onNewConnection={() => setDialog({ kind: "connection" })}
          onNewQuery={() => newQuery()}
        />
        <div className="workspace">
          {layout.navVisible && (
            <>
              <div style={{ width: layout.navWidth }} className="nav-wrap">
                <NavigationPane
                  nodes={nodes}
                  selectedId={selectedNode}
                  expanded={expanded}
                  openConnections={openConnections}
                  openDatabases={openDatabases}
                  connecting={connecting}
                  loading={loadingNodes}
                  filter={filter}
                  onFilter={setFilter}
                  onSelect={selectNode}
                  onActivate={activate}
                  onToggle={toggle}
                  onCollapseAll={() => setExpanded(new Set())}
                  onContextMenu={showContextMenu}
                  starredOnly={starredOnly}
                  onToggleStarredOnly={() => setStarredOnly((s) => !s)}
                />
              </div>
              <div className="splitter" role="separator" aria-orientation="vertical" onMouseDown={startResize} />
            </>
          )}
          <main className="center">
            <TabBar tabs={tabsForBar} activeId={activeTab} onActivate={setActiveTab} onClose={closeTab} />
            <div className="tab-content">
              <div className={`doc-host${activeTab === "objects" ? "" : " is-hidden"}`}>
                <ObjectsView
                  category={category}
                  ctx={ctx}
                  connectionOpen={connectionOpen}
                  viewMode={layout.viewMode}
                  lang={layout.lang}
                  selected={selectedObjects}
                  onSelect={setSelectedObjects}
                  onAction={(kind, names) => openObjects(kind, names)}
                />
              </div>
              {tabs.filter((x) => x.kind !== "objects").map((tab) => (
                <div key={tab.id} className={`doc-host${tab.id === activeTab ? "" : " is-hidden"}`}>{renderDoc(tab)}</div>
              ))}
            </div>
          </main>
          {layout.infoVisible && (
            <InfoPane ctx={ctx} category={category} connectionOpen={connectionOpen} object={selectedObject} lang={layout.lang} />
          )}
        </div>
        <StatusBar
          leftWidth={layout.navVisible ? layout.navWidth + 1 : 300}
          left={status.left}
          cells={status.cells}
          viewMode={active.kind === "objects" ? layout.viewMode : undefined}
          navVisible={layout.navVisible}
          infoVisible={layout.infoVisible}
          onViewMode={(m) => patchLayout({ viewMode: m })}
          onToggleNav={() => patchLayout({ navVisible: !layout.navVisible })}
          onToggleInfo={() => patchLayout({ infoVisible: !layout.infoVisible })}
        />
      </div>

      {ctxMenu && (
        <PopupMenu
          items={ctxMenu.items}
          commands={commands}
          onClose={() => setCtxMenu(null)}
          className="ctx-menu"
          style={{ position: "fixed", left: Math.min(ctxMenu.x, window.innerWidth - 240), top: Math.min(ctxMenu.y, window.innerHeight - 320) }}
        />
      )}

      {dialog?.kind === "connection" && (
        <ConnectionDialog
          editing={dialog.editing}
          knownPassword={dialog.editing ? sessionPasswords.get(dialog.editing.id) : undefined}
          recentProviders={recentProviders}
          onSave={(form, version) => saveConnection(form, version, dialog.editing)}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "deleteConnection" && (
        <MessageDialog
          title={t("cmd.deleteConnection")}
          message={t("msg.deleteConnection", { name: dialog.conn.name })}
          danger
          onOk={() => {
            if (openConnections.has(dialog.conn.id)) disconnect(dialog.conn.id);
            sessionPasswords.delete(dialog.conn.id);
            setConnections((list) => list.filter((c) => c.id !== dialog.conn.id));
            setSelectedNode("root");
            setDialog(null);
          }}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "closeDirty" && (
        <MessageDialog
          title={t("msg.unsavedTitle")}
          message={t("msg.unsavedQuery")}
          okLabel={t("msg.discard")}
          onOk={() => { doCloseTab(dialog.tabId); setDialog(null); }}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "password" && (() => {
        const conn = connections.find((c) => c.id === dialog.connId);
        return (
          <MessageDialog
            title={t("conn.passwordTitle", { name: conn?.name ?? "" })}
            message={dialog.retry ?? t("conn.passwordMessage", { user: conn?.user ?? "", host: `${conn?.host}:${conn?.port}` })}
            prompt={{ label: t("conn.password"), initial: "" }}
            secret
            danger={!!dialog.retry}
            okLabel={t("cmd.openConnection")}
            onOk={(pw) => {
              sessionPasswords.set(dialog.connId, pw ?? "");
              setDialog(null);
              connect(dialog.connId);
            }}
            onCancel={() => setDialog(null)}
          />
        );
      })()}
      {dialog?.kind === "error" && (
        <MessageDialog title={dialog.title} message={dialog.message} danger onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />
      )}
      {dialog?.kind === "name" && (
        <MessageDialog
          title={dialog.title}
          prompt={{ label: dialog.label, initial: dialog.initial }}
          onOk={(value) => value && dialog.onSubmit(value)}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "ddl" && (
        <MessageDialog
          title={dialog.title}
          message={[dialog.danger ? t("msg.ddlDanger") : t("des.reviewMessage"), dialog.note].filter(Boolean).join("\n\n")}
          sql={dialog.statements.join("\n")}
          danger={dialog.danger}
          confirmText={dialog.confirmText}
          okLabel={t("msg.execute")}
          busy={dialog.busy}
          error={dialog.error}
          onOk={() => void runDdl(dialog)}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "info" && (
        <MessageDialog title={dialog.title} message={dialog.message} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />
      )}
      {dialog?.kind === "newDatabase" && !dialog.name && (
        <MessageDialog
          title={t("cmd.newDatabase").replace(/\.\.\.$/, "")}
          prompt={{ label: t("msg.newDatabaseName"), initial: "" }}
          onOk={(name) => name && setDialog({ ...dialog, name })}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "newDatabase" && dialog.name && (
        <MessageDialog
          title={t("des.reviewTitle")}
          message={t("des.reviewMessage")}
          sql={`CREATE DATABASE ${quoteIdent(dialog.name)};`}
          okLabel={t("msg.create")}
          busy={dialog.busy}
          error={dialog.error}
          onOk={() => void createDatabase(dialog)}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "execFile" && (() => {
        const conn = connections.find((c) => c.id === dialog.connId);
        const preview = dialog.sql.length > 6000 ? `${dialog.sql.slice(0, 6000)}\n\n-- … (${t("msg.execFileMore", { n: dialog.sql.length - 6000 })})` : dialog.sql;
        return (
          <MessageDialog
            title={t("cmd.executeSqlFile").replace(/\.\.\.$/, "")}
            message={t("msg.execFileMessage", { file: dialog.fileName, kb: Math.max(1, Math.round(dialog.size / 1024)), db: dialog.database, conn: conn?.name ?? "" })}
            sql={preview}
            danger
            confirmText={conn?.environment === "production" ? dialog.database : undefined}
            okLabel={t("msg.execute")}
            busy={dialog.busy}
            error={dialog.error}
            onOk={() => void runSqlFile(dialog)}
            onCancel={() => setDialog(null)}
          />
        );
      })()}
      <input
        ref={sqlFileInput}
        type="file"
        accept=".sql,.txt,text/plain"
        hidden
        onChange={(e) => void onSqlFilePicked(e.target.files?.[0])}
      />
      {dialog?.kind === "about" && (
        <MessageDialog title={t("cmd.about")} message={t("msg.about")} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />
      )}
    </I18nContext.Provider>
  );
}
