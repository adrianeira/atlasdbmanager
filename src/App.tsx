import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  appReady, chooseExportPath, chooseOpenFile, errorText, isDesktop, local, onOpenScript, onSessionOpened, openDataDictionary, openDataGen, openDataSync, openDataTransfer, openStructSync, pg, toAtlasError, type OpenScriptRequest,
  type ConnectionParams, type DatabaseDetail, type DumpScope, type ProfileRecord, type SchemaDetail,
} from "./backend";
import { entries, matchesShortcut, sub, type CommandMap, type MenuDef, type MenuEntry } from "./commands";
import { ConnectionDialog, type ConnectionForm } from "./components/ConnectionDialog";
import { ExportWizard, type ExportTarget } from "./components/export/ExportWizard";
import { ImportWizard, type ImportTarget } from "./components/import/ImportWizard";
import { schemaFromServer, sessionPasswords, tableFromServer } from "./live";
import type { TableDataBackend } from "./documents/TableDataDoc";
import type { FkLookupSource } from "./components/FkLookup";
import type { QueryBackend } from "./documents/QueryDoc";
import { InfoPane } from "./components/InfoPane";
import { MainToolbar } from "./components/MainToolbar";
import { MenuBar } from "./components/MenuBar";
import { MessageDialog } from "./components/MessageDialog";
import { connIdOf, NavigationPane } from "./components/NavigationPane";
import { objectColumns, objectsFor, ObjectsView, type ViewMode } from "./components/ObjectsView";
import { PopupMenu } from "./components/PopupMenu";
import { StatusBar, type StatusCell } from "./components/StatusBar";
import { TabBar, type DocTab } from "./components/TabBar";
import { QueryDoc, type QueryActions } from "./documents/QueryDoc";
import { TableDataDoc } from "./documents/TableDataDoc";
import { TableDesignerDoc } from "./documents/TableDesignerDoc";
import { ViewDesignerDoc, type ViewBackend } from "./documents/ViewDesignerDoc";
import { refreshMatviewSql } from "./sql/view";
import { dropFuncSql } from "./sql/func";
import { dropSeqSql } from "./sql/sequence";
import { dropExtSql } from "./sql/extension";
import { dropServerStatements } from "./sql/foreignServer";
import { ExtensionDesignerDoc, type ExtensionBackend } from "./documents/ExtensionDesignerDoc";
import { ConsoleDoc } from "./documents/ConsoleDoc";
import { ForeignServerDesignerDoc, type ForeignServerBackend } from "./documents/ForeignServerDesignerDoc";
import { DocFullscreenContext } from "./components/FullscreenButton";
import { FindDoc } from "./documents/FindDoc";
import { ForeignTableDesignerDoc, type ForeignTableBackend } from "./documents/ForeignTableDesignerDoc";
import { TablespaceDesignerDoc, type TablespaceBackend } from "./documents/TablespaceDesignerDoc";
import { dropTablespaceStatement } from "./sql/tablespace";
import { TasksDialog } from "./components/TasksDialog";
import { viewKeyOf } from "./tableView";
import { runTask } from "./taskRun";
import { isDue, type ScheduledTask } from "./tasks";
import { HistoryLogDoc } from "./documents/HistoryLogDoc";
import { ServerMonitorDoc } from "./documents/ServerMonitorDoc";
import { dropTypeSql } from "./sql/typedef";
import { dropRoleSql } from "./sql/role";
import { RoleDesignerDoc, type RoleBackend } from "./documents/RoleDesignerDoc";
import { ColumnsDialog } from "./components/ColumnsDialog";
import { EditDatabaseDialog } from "./components/EditDatabaseDialog";
import { EditSchemaDialog } from "./components/EditSchemaDialog";
import { databaseStatements, schemaStatements, type DatabaseEditState, type SchemaEditState } from "./sql/dbedit";
import { BackupDialog, type BackupTarget } from "./components/BackupDialog";
import { RestoreDialog, type RestoreTarget } from "./components/RestoreDialog";
import { DumpDialog, type DumpTarget } from "./components/DumpDialog";
import { OptionsDialog } from "./components/OptionsDialog";
import { DEFAULT_OPTIONS, OptionsContext, sanitizeOptions, type AppOptions } from "./options";
import { TypeDesignerDoc, type TypeBackend } from "./documents/TypeDesignerDoc";
import { SequenceDesignerDoc, type SequenceBackend } from "./documents/SequenceDesignerDoc";
import { FunctionDesignerDoc, type FunctionBackend } from "./documents/FunctionDesignerDoc";
import { detectLang, I18nContext, makeTranslator, type Lang, type MessageKey } from "./i18n";
import { ProviderIcon, SmallIcon } from "./icons";
import {
  buildVisibleTree, DEFAULT_FILTER_OPTIONS, databaseVisible, makeMatcher, schemaVisible, type FilterOptions, CONNECTION_COLORS, INITIAL_CONNECTIONS, mockRows, quoteIdent, resolveNode, seg,
  type Category, type ConnectionColor, type ConnectionMock, type DatabaseMock, type DbObject, type NodeContext, type ProviderId, type TreeNode,
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
  /** Opções do diagrama ER (menu Exibir). */
  erShowGrid: boolean;
  erSnapGrid: boolean;
  erShowFkNames: boolean;
  /** Ordenação da lista de objetos (menu Exibir → Ordenar) e colunas ocultas por categoria (Escolher colunas). */
  objSortKey: string;
  objSortDir: 1 | -1;
  hiddenObjCols: Record<string, string[]>;
  /** "Mostrar itens ocultos": bancos que a conexão oculta e schemas de sistema. */
  showHidden: boolean;
  /** Exibir → Flatten Object List. */
  flatten: boolean;
  showCaptions: boolean;
  theme: Theme;
  lang: Lang;
}

const LAYOUT_KEY = "atlas.layout.v1";
const CONNECTIONS_KEY = "atlas.connections.v1";
const DEFAULT_LAYOUT: Layout = {
  navWidth: 384, navVisible: true, infoVisible: false, viewMode: "detail", erShowGrid: true, erSnapGrid: true, erShowFkNames: true, objSortKey: "name", objSortDir: 1, hiddenObjCols: {}, showHidden: false, flatten: false, showCaptions: true, theme: "system", lang: detectLang(),
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
  if (c.customDbs) options.customDbs = c.customDbs;
  if (c.remarks) options.remarks = c.remarks;
  if (c.ssh) options.ssh = c.ssh;
  if (c.group) options.group = c.group;
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
    customDbs: Array.isArray(o.customDbs) ? (o.customDbs as { name: string; autoOpen: boolean }[]).filter((d) => d && typeof d.name === "string").map((d) => ({ name: d.name, autoOpen: d.autoOpen === true })) : undefined,
    remarks: typeof o.remarks === "string" ? o.remarks : undefined,
    group: typeof o.group === "string" && o.group ? o.group : undefined,
    ssh: o.ssh && typeof o.ssh === "object" && typeof (o.ssh as { host?: unknown }).host === "string" ? (o.ssh as NonNullable<ConnectionMock["ssh"]>) : undefined,
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
    // Sem senha digitada nesta sessão, o núcleo usa a do cofre (`<id>:ssh`).
    ssh: c.ssh ? { host: c.ssh.host, port: c.ssh.port, user: c.ssh.user, password: sessionPasswords.get(`${c.id}:ssh`) ?? "", keyPath: c.ssh.auth === "publickey" ? c.ssh.keyPath : "" } : undefined,
  };
}

const CATEGORY_LABEL: Record<Category, MessageKey> = {
  table: "tb.table", view: "tb.view", matview: "cat.matview", function: "tb.function",
  sequence: "cat.sequence", type: "cat.type", extension: "cat.extension", foreignServer: "cat.foreignServer", tablespace: "cat.tablespace", user: "tb.user", query: "tb.query",
  backup: "tb.backup", automation: "tb.automation", model: "tb.model", bi: "tb.bi",
};

const FOLDER_LABEL: Partial<Record<Category, MessageKey>> = {
  table: "folder.table", view: "folder.view", matview: "folder.matview",
  function: "folder.function", query: "folder.query", backup: "folder.backup",
};

const COUNT_LABEL: Record<Category, MessageKey> = {
  table: "status.count.table", view: "status.count.view", matview: "status.count.matview", function: "status.count.function",
  sequence: "status.count.sequence", type: "status.count.type", extension: "status.count.extension", foreignServer: "status.count.foreignServer", tablespace: "status.count.tablespace", user: "status.count.user", query: "status.count.query",
  backup: "status.count.backup", automation: "status.count.automation", model: "status.count.model", bi: "status.count.bi",
};

/** Dados de Ajuda → Sobre. A versão vem do `package.json` (via Vite); autor e repositório são fixos do projeto. */
declare const __APP_VERSION__: string;
const APP_VERSION = typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "0.1.0";
const APP_AUTHOR = "Adriano Alves";
const APP_REPO = "https://github.com/adrianeira/atlasdbmanager";

const OBJECTS_TAB: DocTab = { id: "objects", kind: "objects", title: "" };

interface RecentItem { label: string; tab: Omit<DocTab, "id"> }

/**
 * Comando de estrutura disparado pelos menus de contexto. Só pergunta antes (sem mostrar o SQL, como na referência)
 * quando é destrutivo (`danger`) ou exige digitar o nome (`confirmText`); o resto executa direto.
 */
interface DdlRequest {
  title: string;
  /** Pergunta da confirmação, com o nome do objeto (ex.: `Excluir tabela "x"?`). */
  message?: string;
  /** Pergunta antes de executar mesmo sem ser destrutivo (ex.: renomear um banco que outros clientes usam). */
  confirm?: boolean;
  statements: string[];
  connId: string;
  database: string;
  schema?: string;
  /** O que reler na árvore depois de executar. */
  refresh: "connection" | "database" | "schema" | "roles" | "extensions";
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
  | { kind: "connection"; editing?: ConnectionMock; group?: string }
  | { kind: "deleteConnection"; conn: ConnectionMock }
  | { kind: "closeDirty"; tabId: string }
  | { kind: "closeDbDirty"; run: () => void }
  | { kind: "password"; connId: string; retry?: string }
  | { kind: "error"; title: string; message: string }
  | { kind: "info"; title: string; message: string }
  | { kind: "tasks" }
  | { kind: "newDatabase"; connId: string; busy?: boolean; error?: string }
  | ({ kind: "ddl"; busy?: boolean; error?: string } & DdlRequest)
  | { kind: "name"; title: string; label: string; initial: string; onSubmit: (value: string) => void }
  | { kind: "execFile"; connId: string; database: string; fileName: string; size: number; sql: string; busy?: boolean; error?: string }
  | { kind: "options" }
  | { kind: "chooseColumns" }
  | { kind: "editDatabase"; connId: string; /** Id do nó do banco na árvore (para fechá-lo se for renomeado). */ nodeId: string; detail: DatabaseDetail }
  | { kind: "editSchema"; connId: string; database: string; detail: SchemaDetail }
  | ({ kind: "dump" } & DumpTarget)
  | ({ kind: "backup" } & BackupTarget)
  | ({ kind: "restore" } & RestoreTarget)
  | { kind: "deleteBackup"; connId: string; database: string; schema: string; names: string[]; files: { path: string; metaPath: string }[] }
  | { kind: "about" };

/** Aba de tabela, view ou consulta ligada a um banco, na forma em que fica gravada (e guardada ao fechar o banco). */
function savedFromTab(x: DocTab, all: DocTab[], activeId: string, texts: Map<string, string>): SavedTable {
  const base = { connectionId: x.connectionId!, database: x.database!, schema: x.schema ?? "", title: x.title, active: x.id === activeId, pos: all.indexOf(x) };
  if (x.kind === "query") return { ...base, object: "", kind: "query", sql: texts.get(x.id) ?? x.sql ?? "" };
  return { ...base, object: x.object!, kind: (x.objectKind === "view" || x.objectKind === "matview" ? x.objectKind : "table") as SavedTable["kind"] };
}

/** Tabela, view, view materializada ou consulta aberta na sessão anterior. */
interface SavedTable {
  connectionId: string;
  database: string;
  schema: string;
  object: string;
  kind: "table" | "view" | "matview" | "query";
  /** Consulta: o texto do editor. */
  sql?: string;
  title: string;
  /** Era a aba aberta ao fechar. */
  active?: boolean;
  /** Posição na barra de abas (0 é Objects). */
  pos?: number;
}

export default function App() {
  const [layout, setLayout] = useState<Layout>(() => readJson(LAYOUT_KEY, DEFAULT_LAYOUT));
  const t = useMemo(() => makeTranslator(layout.lang), [layout.lang]);
  // Opções da tela Ferramentas → Opções (guardadas no banco local; sem ele valem os padrões).
  const [appOptions, setAppOptions] = useState<AppOptions>(DEFAULT_OPTIONS);
  const [optionsLoaded, setOptionsLoaded] = useState(!isDesktop);
  useEffect(() => {
    if (!isDesktop) return;
    void local.getPref<Partial<AppOptions>>("ui.options")
      .then((v) => setAppOptions(sanitizeOptions(v)))
      .catch(() => undefined)
      .finally(() => setOptionsLoaded(true));
  }, []);
  useEffect(() => {
    document.documentElement.style.setProperty("--editor-font-size", `${appOptions.editorFontSize}px`);
  }, [appOptions.editorFontSize]);
  // Fonte e cores do editor (Opções → SQL Editor); vazio volta ao padrão do tema.
  useEffect(() => {
    const root = document.documentElement.style;
    const set = (name: string, value: string) => (value ? root.setProperty(name, value) : root.removeProperty(name));
    const c = appOptions.editorColors;
    set("--editor-font", appOptions.editorFont ? `"${appOptions.editorFont}", var(--font-mono)` : "");
    set("--editor-fg", c.common);
    set("--editor-bg", c.background);
    set("--sql-keyword", c.keyword);
    set("--sql-type", c.keyword);
    set("--sql-string", c.string);
    set("--sql-number", c.number);
    set("--sql-comment", c.comment);
  }, [appOptions.editorFont, appOptions.editorColors]);
  const patchLayout = useCallback((p: Partial<Layout>) => setLayout((l) => ({ ...l, ...p })), []);
  /** Valor atual de "Mostrar itens ocultos", lido pelas funções que listam bancos e schemas (sempre o mais recente). */
  const showHiddenRef = useRef(layout.showHidden ?? false);
  showHiddenRef.current = layout.showHidden ?? false;

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
  // Conexões lidas: avisa o núcleo para trocar a tela de abertura pela janela principal (depois de pintar).
  useEffect(() => {
    if (!isDesktop || !profilesLoaded) return;
    const id = requestAnimationFrame(() => requestAnimationFrame(() => void appReady().catch(() => undefined)));
    return () => cancelAnimationFrame(id);
  }, [profilesLoaded]);
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
        const savedGroups = await local.getPref<string[]>("conn.groups");
        if (Array.isArray(savedGroups)) setGroups(savedGroups.filter((g) => typeof g === "string" && g));
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
  /** Grupos (pastas) de conexões da árvore, na ordem de criação; os vazios também ficam guardados (preferência `conn.groups`). */
  const [groups, setGroups] = useState<string[]>([]);
  const allGroups = useMemo(() => {
    const all = [...groups];
    for (const c of connections) if (c.group && !all.includes(c.group)) all.push(c.group);
    return all;
  }, [groups, connections]);
  const [connecting, setConnecting] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");
  const [category, setCategory] = useState<Category>("table");
  const [selectedObjects, setSelectedObjects] = useState<string[]>([]);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [exportTarget, setExportTarget] = useState<ExportTarget | null>(null);
  const [importTarget, setImportTarget] = useState<ImportTarget | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  const [recents, setRecents] = useState<RecentItem[]>([]);
  const [recentProviders, setRecentProviders] = useState<ProviderId[]>(["postgresql"]);

  const [starredOnly, setStarredOnly] = useState(false);
  // Opções do funil da busca da árvore (lembradas entre sessões).
  const [filterOptions, setFilterOptions] = useState<FilterOptions>(() => {
    try {
      const raw = JSON.parse(localStorage.getItem("atlas.nav.filter") ?? "null") as Partial<FilterOptions> | null;
      return { mode: raw?.mode === "starts" ? "starts" : "contains", caseSensitive: !!raw?.caseSensitive };
    } catch {
      return DEFAULT_FILTER_OPTIONS;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem("atlas.nav.filter", JSON.stringify(filterOptions));
    } catch {
      // Sem armazenamento: as opções só valem nesta sessão.
    }
  }, [filterOptions]);
  const nodes = useMemo(
    () => buildVisibleTree(connections, { expanded, openConnections, openDatabases, filter, filterOptions, starredOnly, groups: allGroups, flatten: layout.flatten ?? false }, (c) => t(FOLDER_LABEL[c] ?? CATEGORY_LABEL[c]), t("nav.myConnections")),
    [connections, expanded, openConnections, openDatabases, filter, filterOptions, starredOnly, allGroups, layout.flatten, t],
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

  /** Relê a lista de papéis do servidor (depois de criar, alterar ou excluir um). */
  const reloadRoles = async (connId: string) => {
    const roles = await pg.listRoles(connId);
    updateConn(connId, (c) => ({ ...c, roles: roles.map((r) => ({ name: r.name, canLogin: r.canLogin, superuser: r.superuser })) }));
  };

  /** Lista de bancos do servidor sobre a atual: mantém o que já foi lido de cada banco e atualiza o comentário. */
  const mergeDatabases = (current: DatabaseMock[], dbs: { databases: string[]; comments: Record<string, string> }): DatabaseMock[] =>
    dbs.databases.map((name) => ({
      ...(current.find((d) => d.name === name) ?? { name, schemas: [], loaded: false }),
      comment: dbs.comments[name],
    }));

  /** Bancos do servidor; com lista personalizada (aba Databases) só os da lista aparecem. */
  const fetchDatabases = async (connId: string, includeHidden: boolean) => {
    const dbs = await pg.listDatabases(connId, includeHidden);
    const custom = connectionsRef.current.find((c) => c.id === connId)?.customDbs;
    if (!custom) return dbs;
    const wanted = new Set(custom.map((d) => d.name));
    return { ...dbs, databases: dbs.databases.filter((n) => wanted.has(n)), hidden: 0 };
  };

  /** Abre a sessão no servidor e lê bancos/roles. Rejeita se a senha ainda não é conhecida. */
  /** Conexões que esta janela está abrindo agora (o aviso `session-opened` da própria abertura não deve ser tratado de novo). */
  const openingRef = useRef(new Set<string>());
  const openSession = async (connId: string) => {
    if (openRef.current.has(connId)) return;
    const conn = connectionsRef.current.find((c) => c.id === connId);
    if (!conn) return;
    openingRef.current.add(connId);
    try {
      // Sem senha digitada nesta sessão, o núcleo usa a do cofre (profileId); ela não vem para a UI.
      const info = await pg.open(connId, paramsFor(conn), conn.id);
      await adoptSession(connId, info);
    } finally {
      openingRef.current.delete(connId);
    }
  };

  /** Marca a conexão como aberta e lê bancos e papéis (a sessão já existe no servidor). */
  const adoptSession = async (connId: string, info: { version: string; versionNum: number }) => {
    const dbs = await fetchDatabases(connId, showHiddenRef.current);
    const roles = await pg.listRoles(connId).catch(() => []);
    updateConn(connId, (c) => ({
      ...c,
      serverVersion: info.version,
      serverVersionNum: info.versionNum,
      hiddenDatabases: dbs.hidden,
      roles: roles.map((r) => ({ name: r.name, canLogin: r.canLogin, superuser: r.superuser })),
      databases: mergeDatabases(c.databases, dbs),
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
        .then(() => {
          setExpanded((s) => add(s, `conn:${seg(connId)}`));
          // Bancos marcados em "Auto Open" (aba Databases) abrem ao conectar. A lista de bancos acabou de entrar no estado:
          // espera o React refletir (até 3 s) para `loadDatabase` achá-los.
          for (const d of conn.customDbs ?? []) {
            if (!d.autoOpen) continue;
            void (async () => {
              for (let i = 0; i < 30; i++) {
                if (connectionsRef.current.find((c) => c.id === connId)?.databases.some((x) => x.name === d.name)) break;
                await new Promise((r) => window.setTimeout(r, 100));
              }
              await loadDatabase(connId, d.name);
              const dbId = `db:${seg(connId)}/${seg(d.name)}`;
              setOpenDatabases((s) => add(s, dbId));
              setExpanded((s) => add(s, dbId));
            })().catch(() => undefined);
          }
        })
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

  const disconnect = (connId: string) => parkTables(connId, undefined, () => {
    if (connections.find((c) => c.id === connId)?.live) {
      void pg.close(connId).catch(() => undefined);
      // Ao reabrir, a estrutura é lida de novo do servidor.
      updateConn(connId, (c) => ({ ...c, databases: [] }));
    }
    openRef.current = remove(openRef.current, connId);
    setOpenConnections((s) => remove(s, connId));
    const prefix = `db:${seg(connId)}/`;
    setOpenDatabases((s) => new Set([...s].filter((x) => !x.startsWith(prefix))));
  });

  /** `refresh`: relê só a lista de schemas, sem indicador de carregamento e mantendo o que já foi lido de cada um. */
  const loadDatabase = async (connId: string, dbName: string, force = false, refresh = false) => {
    const conn = connectionsRef.current.find((c) => c.id === connId);
    const db = conn?.databases.find((d) => d.name === dbName);
    if (!conn?.live || !db || (db.loaded && !force && !refresh)) return;
    const infos = refresh
      ? await pg.listSchemas(connId, dbName, showHiddenRef.current)
      : await withLoading(`db:${seg(connId)}/${seg(dbName)}`, () => pg.listSchemas(connId, dbName, showHiddenRef.current));
    updateConn(connId, (c) => ({
      ...c,
      databases: c.databases.map((d) => (d.name !== dbName ? d : {
        ...d, loaded: true,
        // O comentário é sempre o do servidor, mesmo quando o que já foi lido do schema é reaproveitado.
        schemas: infos.map((s) => ({
          ...((force ? undefined : d.schemas.find((x) => x.name === s.name)) ?? { name: s.name, objects: {}, loaded: false }),
          comment: s.comment ?? undefined,
        })),
      })),
    }));
    void loadBackups(connId, dbName, "").catch(() => undefined);
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
    void loadBackups(connId, dbName, schemaName).catch(() => undefined);
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
        const dbs = await fetchDatabases(connId, showHiddenRef.current);
        updateConn(connId, (cc) => ({ ...cc, hiddenDatabases: dbs.hidden, databases: mergeDatabases(cc.databases, dbs) }));
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
  /** Texto atual de cada consulta aberta (o editor guarda o texto; aqui fica a cópia para o snapshot da sessão). */
  const queryTexts = useRef(new Map<string, string>());
  const [activeTab, setActiveTab] = useState("objects");
  const [docStatus, setDocStatus] = useState<Record<string, StatusCell[]>>({});
  const queryActions = useRef<QueryActions | null>(null);
  /** Aba em tela cheia (o documento cobre a janela); troca de aba, fechar a aba ou Esc saem dela. */
  const [fullDoc, setFullDoc] = useState<string | null>(null);
  useEffect(() => {
    if (!fullDoc) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setFullDoc(null); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullDoc]);
  useEffect(() => { setFullDoc((f) => (f === activeTab ? f : null)); }, [activeTab]);
  const tabSeq = useRef(0);

  const ctx = resolveNode(selectedNode, connections);
  const connectionOpen = !!ctx.connection && openConnections.has(ctx.connection.id);

  const openTab = (tab: Omit<DocTab, "id">, recentLabel?: string) => {
    // Reaproveita a aba se o mesmo objeto já estiver aberto no mesmo modo.
    const existing = tab.object && !appOptions.multipleForms && tabs.find((x) => x.kind === tab.kind && x.connectionId === tab.connectionId && x.database === tab.database && x.schema === tab.schema && x.object === tab.object);
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

  // Pedido de uma janela de ferramenta (ex.: Structure Synchronization): o script abre numa aba de consulta.
  const openScript = useRef((_r: OpenScriptRequest) => {});
  openScript.current = (r) => {
    const conn = connections.find((c) => c.id === r.connId);
    openTab({
      kind: "query", title: t("tab.untitledQuery"), windowTitle: t("tab.untitledQuery"), icon: "query",
      connectionId: conn?.id, database: r.database, schema: r.schema, sql: r.sql,
    });
  };
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void onOpenScript((r) => openScript.current(r)).then((f) => { if (alive) off = f; else f(); });
    return () => { alive = false; off?.(); };
  }, []);

  // Uma ferramenta em outra janela (Data Transfer) abriu uma conexão: esta janela passa a mostrá-la como aberta.
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void onSessionOpened((e) => {
      if (openRef.current.has(e.id) || openingRef.current.has(e.id)) return;
      const conn = connectionsRef.current.find((c) => c.id === e.id);
      if (!conn?.live) return;
      openingRef.current.add(e.id);
      void adoptSession(e.id, e).catch(() => undefined).finally(() => openingRef.current.delete(e.id));
    }).then((f) => { if (alive) off = f; else f(); });
    return () => { alive = false; off?.(); };
    // O ouvinte é criado uma vez; as funções que ele chama leem o estado mais recente por refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Abre o console (estilo psql) da conexão; a mesma conexão e o mesmo banco reaproveitam a aba. */
  const openConsole = (conn: ConnectionMock, dbName?: string) => {
    const database = dbName ?? conn.initialDatabase ?? "postgres";
    const title = `${conn.name} - ${t("con.title")}`;
    openTab({ kind: "console", title, windowTitle: title, icon: "query", object: "console", connectionId: conn.id, database });
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

  const askDdl = (req: DdlRequest) => {
    const d = { kind: "ddl" as const, ...req };
    if (req.danger || req.confirm || req.confirmText) setDialog(d);
    else void runDdl(d);
  };
  /** Pergunta da confirmação de exclusão, com o nome do objeto. */
  const confirmDelete = (kind: "database" | "schema" | "table" | "view" | "matview" | "function" | "procedure" | "type" | "sequence" | "extension" | "foreignServer" | "tablespace" | "role", name: string) =>
    t("msg.confirmDelete", { kind: t(`kind.${kind}`), name });
  const askName = (title: string, label: string, initial: string, onSubmit: (value: string) => void) =>
    setDialog({ kind: "name", title, label, initial, onSubmit });

  /**
   * Exibir → Mostrar itens ocultos: liga/desliga e relê os bancos de cada conexão aberta e os schemas dos bancos já lidos,
   * para a árvore refletir a escolha na hora.
   */
  const toggleShowHidden = () => {
    const next = !(layout.showHidden ?? false);
    showHiddenRef.current = next;
    patchLayout({ showHidden: next });
    for (const c of connectionsRef.current) {
      if (!c.live || !openRef.current.has(c.id)) continue;
      void fetchDatabases(c.id, next)
        .then((dbs) => {
          updateConn(c.id, (cc) => ({ ...cc, hiddenDatabases: dbs.hidden, databases: mergeDatabases(cc.databases, dbs) }));
          for (const d of c.databases) if (d.loaded) void loadDatabase(c.id, d.name, false, true).catch(() => undefined);
        })
        .catch((e) => showError(t("cmd.showHidden"), e));
    }
  };

  /** Novo schema no banco: pede o nome e cria direto. */
  const promptNewSchema = (connId: string, dbName: string) =>
    askName(t("cmd.newSchema"), t("msg.schemaName"), "", (name) => askDdl({
      title: t("cmd.newSchema"), statements: [`CREATE SCHEMA ${quoteIdent(name)};`],
      connId, database: dbName, refresh: "database",
    }));

  /** Editar banco: lê os atributos no servidor e abre o diálogo. */
  const openEditDatabase = async (connId: string, dbName: string, nodeId: string) => {
    try {
      setDialog({ kind: "editDatabase", connId, nodeId, detail: await pg.databaseDetail(connId, dbName) });
    } catch (e) {
      showError(t("cmd.editDatabase").replace(/\.\.\.$/, ""), e);
    }
  };

  const applyEditDatabase = (d: Extract<Dialog, { kind: "editDatabase" }>, state: DatabaseEditState) => {
    const conn = connectionsRef.current.find((c) => c.id === d.connId);
    const statements = databaseStatements(d.detail, state);
    if (!conn || !statements.length) return;
    const renamed = state.name.trim() !== d.detail.name;
    askDdl({
      title: t("cmd.editDatabase").replace(/\.\.\.$/, ""),
      message: renamed ? t("msg.confirmRename", { kind: t("kind.database"), from: d.detail.name, to: state.name.trim() }) : undefined,
      confirm: renamed,
      statements,
      connId: d.connId,
      // Roda pelo banco inicial da conexão: o PostgreSQL não renomeia o banco em que a sessão está.
      database: conn.initialDatabase || "postgres",
      refresh: "connection",
      confirmText: renamed && conn.environment === "production" ? d.detail.name : undefined,
      closeDbFirst: renamed ? d.detail.name : undefined,
      note: renamed ? t("ed.renameNote") : undefined,
      after: renamed ? () => setOpenDatabases((s) => remove(s, d.nodeId)) : undefined,
    });
  };

  /** Editar schema: lê dono e comentário no servidor e abre o diálogo. */
  const openEditSchema = async (connId: string, dbName: string, schemaName: string) => {
    try {
      setDialog({ kind: "editSchema", connId, database: dbName, detail: await pg.schemaDetail(connId, dbName, schemaName) });
    } catch (e) {
      showError(t("cmd.editSchema").replace(/\.\.\.$/, ""), e);
    }
  };

  const applyEditSchema = (d: Extract<Dialog, { kind: "editSchema" }>, state: SchemaEditState) => {
    const conn = connectionsRef.current.find((c) => c.id === d.connId);
    const statements = schemaStatements(d.detail, state);
    if (!conn || !statements.length) return;
    const renamed = state.name.trim() !== d.detail.name;
    askDdl({
      title: t("cmd.editSchema").replace(/\.\.\.$/, ""),
      message: renamed ? t("msg.confirmRename", { kind: t("kind.schema"), from: d.detail.name, to: state.name.trim() }) : undefined,
      statements,
      connId: d.connId,
      database: d.database,
      refresh: "database",
      confirmText: renamed && conn.environment === "production" ? d.detail.name : undefined,
    });
  };

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
      if (d.refresh === "roles") void reloadRoles(d.connId).catch(fail);
      else if (d.refresh === "schema" && d.schema) void loadSchema(d.connId, d.database, d.schema, true).catch(fail);
      else if (d.refresh === "extensions" && d.schema) reloadExtensions(d.connId, d.database, d.schema);
      else if (d.refresh === "database") void loadDatabase(d.connId, d.database, true).catch(fail);
      else {
        const node = nodes.find((n) => n.id === `conn:${seg(d.connId)}`);
        if (node) refreshNode(node);
      }
    } catch (e) {
      setDialog({ ...d, busy: false, error: errorText(toAtlasError(e)) });
    }
  };

  /** Cria o banco direto ao confirmar o nome; erro do servidor aparece no próprio diálogo do nome. */
  const createDatabase = async (d: Extract<Dialog, { kind: "newDatabase" }>, name: string) => {
    const conn = connectionsRef.current.find((c) => c.id === d.connId);
    if (!conn || !name) return;
    setDialog({ ...d, busy: true, error: undefined });
    try {
      // CREATE DATABASE não roda dentro de transação: executa isolado.
      await pg.execute(conn.id, conn.initialDatabase || "postgres", `CREATE DATABASE ${quoteIdent(name)}`, 1);
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

  const newFunction = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "tb.function");
    openTab({ kind: "functionDesigner", title, windowTitle: title, icon: "function", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  /** Como o `quote_ident` do PostgreSQL: só coloca aspas quando o nome precisa. */
  const quoteIfNeeded = (name: string) => (/^[a-z_][a-z0-9_$]*$/.test(name) ? name : quoteIdent(name));

  /** Abre a aba "Find in Database/Schema", já no banco ou schema da árvore quando houver. */
  const openFind = (connId?: string, database?: string, schema?: string) => {
    const connName = connectionsRef.current.find((c) => c.id === connId)?.name;
    const title = schema ? `${database}.${schema} (${connName}) - ${t("fd.titleSchema")}` : database ? `${database} (${connName}) - ${t("fd.title")}` : t("fd.title");
    openTab({ kind: "find", title, windowTitle: title, icon: "search", connectionId: connId, database, schema });
  };

  /** Abre o objeto de um achado da busca: carrega o schema (se preciso) e abre o designer ou os dados. */
  const openFindHit = async (connId: string, database: string, schemaName: string, hit: { kind: "table" | "view" | "matview" | "function"; name: string; identity: string | null }, what?: string) => {
    const conn = connectionsRef.current.find((c) => c.id === connId);
    if (!conn) return;
    await loadDatabase(connId, database).catch(() => undefined);
    await loadSchema(connId, database, schemaName).catch(() => undefined);
    const c: NodeContext = {
      connection: conn, database: { name: database, schemas: [], loaded: true }, schema: { name: schemaName, objects: {} },
    };
    if (hit.kind === "function") {
      const title = objectTitle(hit.name, "tb.function", c);
      const identity = hit.identity ?? "";
      openTab({
        kind: "functionDesigner", connectionId: connId, database, schema: schemaName, object: `${hit.name}(${identity})`, fnName: hit.name,
        fnIdentity: identity, title, windowTitle: title, icon: "function",
      });
      return;
    }
    // Achado de campo abre o designer da tabela; os demais, os dados.
    openObjects(what === "fieldName" ? "design" : "open", [hit.name], hit.kind, c);
  };

  const newRole = () => {
    if (!ctx.connection?.live) return;
    const title = objectTitle(t("des.untitled"), "tb.user", { connection: ctx.connection });
    openTab({ kind: "roleDesigner", title, windowTitle: title, icon: "design", connectionId: ctx.connection.id, database: ctx.database?.name ?? ctx.connection.initialDatabase });
  };

  const newType = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "cat.type");
    openTab({ kind: "typeDesigner", title, windowTitle: title, icon: "design", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  const newSequence = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "cat.sequence");
    openTab({ kind: "sequenceDesigner", title, windowTitle: title, icon: "sequence", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  const newExtension = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "cat.extension");
    openTab({ kind: "extensionDesigner", title, windowTitle: title, icon: "extension", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  const newForeignTable = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "ftb.tooltip");
    openTab({ kind: "foreignTableDesigner", title, windowTitle: title, icon: "foreignTable", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  const newTablespace = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "cat.tablespace");
    openTab({ kind: "tablespaceDesigner", title, windowTitle: title, icon: "tablespace", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  const newForeignServer = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "cat.foreignServer");
    openTab({ kind: "foreignServerDesigner", title, windowTitle: title, icon: "foreignServer", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  /**
   * Depois de instalar, atualizar ou remover uma extensão: a lista é do banco inteiro e vem junto com cada schema,
   * então relê o schema atual e marca os outros para serem relidos quando forem abertos.
   */
  const reloadExtensions = (connId: string, dbName: string, schemaName: string) => {
    updateConn(connId, (c) => ({
      ...c,
      databases: c.databases.map((d) => (d.name !== dbName ? d : { ...d, schemas: d.schemas.map((s) => (s.name === schemaName ? s : { ...s, loaded: false })) })),
    }));
    void loadSchema(connId, dbName, schemaName, true).catch(() => undefined);
  };

  const newMatview = () => {
    if (!ctx.connection || !ctx.database || !ctx.schema) return;
    const title = objectTitle(t("des.untitled"), "tb.matview");
    openTab({ kind: "viewDesigner", objectKind: "matview", title, windowTitle: title, icon: "matview", connectionId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name });
  };

  /** REFRESH MATERIALIZED VIEW (executa direto). */
  const refreshMatview = (c: NodeContext, name: string, concurrently: boolean) => {
    if (!c.connection || !c.database || !c.schema) return;
    askDdl({
      connId: c.connection.id, database: c.database.name, schema: c.schema.name, refresh: "schema",
      title: t(concurrently ? "act.refreshMatviewConcurrently" : "act.refreshMatview"),
      statements: [refreshMatviewSql(c.schema.name, name, concurrently)],
    });
  };

  /** Assistente de exportação de uma tabela/view (todos os registros). Só com conexão real. */
  const exportTablesOf = (schema: { objects: Partial<Record<Category, DbObject[]>> } | undefined, also?: string) => {
    const tables = (schema?.objects.table ?? []).map((o) => ({ name: o.name, view: false }));
    const views = (schema?.objects.view ?? []).map((o) => ({ name: o.name, view: true }));
    const matviews = (schema?.objects.matview ?? []).map((o) => ({ name: o.name, view: true }));
    const all = [...tables, ...views, ...matviews];
    return also && !all.some((x) => x.name === also) ? [...all, { name: also, view: false }] : all;
  };

  const startExport = (c: NodeContext, name: string) => {
    if (!c.connection?.live || !c.database || !c.schema) return;
    setExportTarget({ connId: c.connection.id, database: c.database.name, schema: c.schema.name, tables: exportTablesOf(c.schema, name), initial: name });
  };

  /** Assistente de importação: `name` é a tabela sugerida como destino. Só com conexão real. */
  const startImport = (c: NodeContext, name?: string) => {
    if (!c.connection?.live || !c.database || !c.schema) return;
    setImportTarget({ connId: c.connection.id, database: c.database.name, schema: c.schema.name, tables: (c.schema.objects.table ?? []).map((o) => o.name), initial: name });
  };

  /** Relê os backups guardados do schema para a lista da categoria Backup. */
  const loadBackups = async (connId: string, dbName: string, schemaName: string) => {
    const list = await pg.backupList(connId, dbName, schemaName);
    const objs = list.map((e) => ({ name: e.scope === "database" ? `${e.name} (${t("bk.scopeDatabase")})` : e.name, path: e.path, metaPath: e.metaPath, dataLength: e.bytes, modified: new Date(e.modifiedMs).toISOString(), comment: e.comment || undefined }));
    updateConn(connId, (c) => ({
      ...c,
      databases: c.databases.map((d) => (d.name !== dbName ? d : {
        ...d,
        backups: objs.filter((_, i) => list[i].scope === "database"),
        schemas: d.schemas.map((sc) => (sc.name !== schemaName ? sc : { ...sc, objects: { ...sc.objects, backup: objs } })),
      })),
    }));
  };

  /** Relê a lista de backups de todos os schemas já carregados do banco (um backup do banco aparece em todos). */
  const refreshBackups = (connId: string, dbName: string) => {
    const db = connectionsRef.current.find((c) => c.id === connId)?.databases.find((d) => d.name === dbName);
    void loadBackups(connId, dbName, "").catch(() => undefined);
    for (const sc of db?.schemas ?? []) if (sc.loaded) void loadBackups(connId, dbName, sc.name).catch(() => undefined);
  };

  /**
   * Novo backup: do schema do contexto (ou de `schemaName`) ou, com `schemaName === null`, do banco inteiro. Carrega a lista
   * de objetos para a seleção e abre o diálogo.
   */
  const openBackup = async (c = ctx, schemaName?: string | null) => {
    const conn = c.connection;
    const database = c.database?.name ?? conn?.databases[0]?.name;
    const dbNode = conn?.databases.find((d) => d.name === database);
    const schema = schemaName === null ? undefined : (schemaName ?? c.schema?.name ?? dbNode?.schemas[0]?.name);
    if (!conn?.live || !database || (schemaName !== null && !schema)) return;
    const wanted = schema ? [schema] : (dbNode?.schemas.map((x) => x.name) ?? []);
    try {
      for (const name of wanted) await loadSchema(conn.id, database, name);
    } catch (e) {
      return showError(t("cmd.newBackup"), e);
    }
    const loaded = connectionsRef.current.find((x) => x.id === conn.id)?.databases.find((d) => d.name === database)?.schemas.filter((x) => wanted.includes(x.name)) ?? [];
    const pick = (cat: Category) => loaded.flatMap((sc) => (sc.objects[cat] ?? []).map((o) => ({ schema: sc.name, name: o.fnName ?? o.name, rows: o.rows })));
    const major = Math.floor(conn.serverVersionNum / 10000);
    const version = major >= 10 ? String(major) : `${major}.${Math.floor((conn.serverVersionNum % 10000) / 100)}`;
    setDialog({
      kind: "backup", connId: conn.id, database, schema, server: `Postgres ${version}`,
      objects: { table: pick("table"), view: pick("view"), matview: pick("matview"), function: pick("function"), sequence: pick("sequence"), type: pick("type") },
    });
  };

  /** Restaurar um backup (ou extrair o SQL dele): `path` é o backup escolhido na lista; sem ele o diálogo pede o arquivo. */
  const openRestore = (mode: RestoreTarget["mode"], c = ctx, path?: string) => {
    const conn = c.connection;
    const database = c.database?.name ?? conn?.databases[0]?.name;
    if (!conn?.live || !database) return;
    setDialog({ kind: "restore", mode, connId: conn.id, database, databases: conn.databases.map((d) => d.name), path });
  };

  const openObjects = (kind: "open" | "design" | "new" | "export" | "import" | "backup" | "restore" | "extract" | "delete", names: string[], cat: Category = category, c = ctx) => {
    if (kind === "backup") return void openBackup(c, c.schema ? undefined : null);
    if (kind === "restore" || kind === "extract") return openRestore(kind, c, c.schema?.objects.backup?.find((o) => o.name === names[0])?.path);
    if (kind === "delete") {
      const items = (c.schema?.objects.backup ?? []).filter((o) => names.includes(o.name) && o.path);
      if (cat === "backup" && c.connection && c.database && c.schema && items.length > 0) {
        setDialog({ kind: "deleteBackup", connId: c.connection.id, database: c.database.name, schema: c.schema.name, names: items.map((o) => o.name), files: items.map((o) => ({ path: o.path!, metaPath: o.metaPath ?? "" })) });
      }
      return;
    }
    if (kind === "import") {
      if (cat === "table") startImport(c, names[0]);
      return;
    }
    if (kind === "export") {
      if ((cat === "table" || cat === "view" || cat === "matview") && names[0]) startExport(c, names[0]);
      return;
    }
    if (kind === "new") {
      if (cat === "query") return newQuery();
      if (cat === "table") return newTable();
      if (cat === "view") return newView();
      if (cat === "matview") return newMatview();
      if (cat === "function") return newFunction();
      if (cat === "sequence") return newSequence();
      if (cat === "type") return newType();
      if (cat === "extension") return newExtension();
      if (cat === "foreignServer") return newForeignServer();
      if (cat === "tablespace") return newTablespace();
      if (cat === "user") return newRole();
      return openTab({ kind: "placeholder", title: `${t("act.new")} ${t(CATEGORY_LABEL[cat])}`, icon: cat, note: t("placeholder.designer") });
    }
    for (const name of names) {
      const base = { connectionId: c.connection?.id, database: c.database?.name, schema: c.schema?.name, object: name };
      if (cat === "table" && kind === "open") {
        const title = objectTitle(name, "tb.table", c);
        openTab({ ...base, kind: "data", title, windowTitle: title, icon: "table" }, `${t("act.openTable")} - ${name}`);
      } else if (cat === "table" && kind === "design" && c.connection?.live && c.schema?.objects.table?.find((o) => o.name === name)?.dataType === "foreign") {
        const title = objectTitle(name, "ftb.tooltip", c);
        openTab({ ...base, kind: "foreignTableDesigner", title, windowTitle: title, icon: "foreignTable" }, `${t("act.design")} - ${name}`);
      } else if (cat === "table" && kind === "design") {
        const title = objectTitle(name, "tb.table", c);
        openTab({ ...base, kind: "designer", title, windowTitle: title, icon: "tableDesign" }, `${t("act.designTable")} - ${name}`);
      } else if (cat === "view" && kind === "open") {
        const title = objectTitle(name, "tb.view", c);
        openTab({ ...base, kind: "data", objectKind: "view", title, windowTitle: title, icon: "view" }, `${t("act.openView")} - ${name}`);
      } else if (cat === "view" && kind === "design") {
        const title = objectTitle(name, "tb.view", c);
        openTab({ ...base, kind: "viewDesigner", title, windowTitle: title, icon: "view" }, `${t("act.designView")} - ${name}`);
      } else if (cat === "function" && c.connection?.live) {
        const fn = c.schema?.objects.function?.find((o) => o.name === name);
        const title = objectTitle(fn?.fnName ?? name, "tb.function", c);
        openTab({ ...base, kind: "functionDesigner", fnName: fn?.fnName, fnIdentity: fn?.args, title, windowTitle: title, icon: "function" }, `${t("act.designFunction")} - ${name}`);
      } else if (cat === "user" && c.connection?.live) {
        const title = objectTitle(name, "tb.user", c);
        openTab({ ...base, kind: "roleDesigner", database: c.database?.name ?? c.connection.initialDatabase, title, windowTitle: title, icon: "design" }, `${t("act.editUser")} - ${name}`);
      } else if (cat === "type" && c.connection?.live) {
        const title = objectTitle(name, "cat.type", c);
        openTab({ ...base, kind: "typeDesigner", title, windowTitle: title, icon: "design" }, `${t("act.design")} - ${name}`);
      } else if (cat === "sequence" && c.connection?.live) {
        const title = objectTitle(name, "cat.sequence", c);
        openTab({ ...base, kind: "sequenceDesigner", title, windowTitle: title, icon: "sequence" }, `${t("act.design")} - ${name}`);
      } else if (cat === "tablespace" && c.connection?.live) {
        const title = objectTitle(name, "cat.tablespace", c);
        openTab({ ...base, kind: "tablespaceDesigner", title, windowTitle: title, icon: "tablespace" }, `${t("act.design")} - ${name}`);
      } else if (cat === "foreignServer" && c.connection?.live) {
        const title = objectTitle(name, "cat.foreignServer", c);
        openTab({ ...base, kind: "foreignServerDesigner", title, windowTitle: title, icon: "foreignServer" }, `${t("act.design")} - ${name}`);
      } else if (cat === "extension" && c.connection?.live) {
        const title = objectTitle(name, "cat.extension", c);
        openTab({ ...base, kind: "extensionDesigner", title, windowTitle: title, icon: "extension" }, `${t("act.design")} - ${name}`);
      } else if (cat === "matview" && kind === "open") {
        const title = objectTitle(name, "tb.matview", c);
        openTab({ ...base, kind: "data", objectKind: "matview", title, windowTitle: title, icon: "matview" }, `${t("act.openMatview")} - ${name}`);
      } else if (cat === "matview" && kind === "design") {
        const title = objectTitle(name, "tb.matview", c);
        openTab({ ...base, kind: "viewDesigner", objectKind: "matview", title, windowTitle: title, icon: "matview" }, `${t("act.designMatview")} - ${name}`);
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

  /** Arrastar uma aba: ela passa para antes (ou depois) da aba alvo. A aba Objects continua sendo a primeira. */
  const reorderTab = (id: string, targetId: string, after: boolean) => {
    setTabs((ts) => {
      const moving = ts.find((x) => x.id === id);
      if (!moving || id === "objects" || id === targetId) return ts;
      const rest = ts.filter((x) => x.id !== id);
      const at = rest.findIndex((x) => x.id === targetId);
      if (at < 0) return ts;
      rest.splice(Math.max(1, at + (after ? 1 : 0)), 0, moving);
      return rest;
    });
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

  /** Abas de consulta com transação aberta (fechar sempre pergunta: a transação seria desfeita). */
  const txTabs = useRef(new Set<string>());
  const closeTab = (id: string) => {
    const tab = tabs.find((x) => x.id === id);
    const skipAsk = tab?.kind === "query" && !appOptions.confirmCloseQueries && !txTabs.current.has(id);
    if (tab?.dirty && !skipAsk) setDialog({ kind: "closeDirty", tabId: id });
    else doCloseTab(id);
  };

  const cycleTab = (dir: 1 | -1) => {
    const idx = tabs.findIndex((x) => x.id === activeTab);
    setActiveTab(tabs[(idx + dir + tabs.length) % tabs.length].id);
  };

  /* ---------- Tarefas agendadas (backup e exportação) ---------- */
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;
  const [tasksLoaded, setTasksLoaded] = useState(!isDesktop);
  const [runningTasks, setRunningTasks] = useState<Set<string>>(new Set());
  const runningRef = useRef(new Set<string>());
  useEffect(() => {
    if (!isDesktop) return;
    void local.getPref<ScheduledTask[]>("tasks.v1")
      .then((v) => { if (Array.isArray(v)) setTasks(v.filter((x) => x && typeof x.id === "string" && x.job && x.schedule)); })
      .catch(() => undefined)
      .finally(() => setTasksLoaded(true));
  }, []);
  useEffect(() => {
    if (isDesktop && tasksLoaded) void local.setPref("tasks.v1", tasks).catch(() => undefined);
  }, [tasks, tasksLoaded]);

  /** Roda a tarefa agora: abre a conexão se preciso, grava o resultado na própria tarefa e mostra o erro (se houver). */
  const executeTask = async (id: string) => {
    const task = tasksRef.current.find((x) => x.id === id);
    if (!task || runningRef.current.has(id)) return;
    runningRef.current.add(id);
    setRunningTasks(new Set(runningRef.current));
    let status: "ok" | "error" = "ok";
    let message: string;
    try {
      message = await runTask(task, queryBackend.ensureOpen);
    } catch (e) {
      status = "error";
      message = errorText(toAtlasError(e));
      showError(`${t("tk.title")}: ${task.name}`, message);
    } finally {
      runningRef.current.delete(id);
      setRunningTasks(new Set(runningRef.current));
    }
    setTasks((list) => list.map((x) => (x.id === id ? { ...x, lastRunMs: Date.now(), lastStatus: status, lastMessage: message } : x)));
  };
  // A cada 20 segundos: o que venceu roda (uma vez por vencimento; tarefa atrasada roda já e a contagem segue da última execução).
  useEffect(() => {
    if (!isDesktop || !tasksLoaded) return;
    const id = window.setInterval(() => {
      const now = Date.now();
      for (const task of tasksRef.current) if (!runningRef.current.has(task.id) && isDue(task, now)) void executeTask(task.id);
    }, 20_000);
    return () => window.clearInterval(id);
    // As funções chamadas leem o estado por refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tasksLoaded]);

  /* ---------- Sessão: consultas abertas (Opções → Tabs e Auto Recovery) ---------- */
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const activeTabRef = useRef(activeTab);
  activeTabRef.current = activeTab;
  /** Só depois de decidir o que reabrir é que a sessão passa a ser gravada (senão a aba Objects sozinha apagaria a anterior). */
  const sessionReady = useRef(false);
  const lastSession = useRef("");
  /** Tabelas, views e views materializadas da sessão anterior que esperam a conexão abrir (continuam gravadas enquanto esperam). */
  const pendingTables = useRef<SavedTable[]>([]);
  const restoring = useRef(new Set<string>());
  const saveSession = useCallback(() => {
    if (!isDesktop || !sessionReady.current) return;
    // Consultas ligadas a um banco se comportam como as tabelas (voltam quando o banco é aberto); as sem banco voltam ao iniciar.
    const queries = tabsRef.current.filter((x) => x.kind === "query" && !(x.connectionId && x.database));
    const list = queries.map((x) => ({ title: x.title, connectionId: x.connectionId, database: x.database, schema: x.schema, sql: queryTexts.current.get(x.id) ?? x.sql ?? "" }));
    const active = Math.max(0, queries.findIndex((x) => x.id === activeTabRef.current));
    const dbTabs = tabsRef.current.filter((x) => (x.kind === "data" && x.connectionId && x.database && x.schema && x.object) || (x.kind === "query" && x.connectionId && x.database));
    const tables: SavedTable[] = [
      ...dbTabs.map((x) => savedFromTab(x, tabsRef.current, activeTabRef.current, queryTexts.current)),
      ...pendingTables.current,
    ];
    const activeTable = tables.findIndex((x) => x.active);
    const state = { tabs: list, active, tables, activeTable };
    const json = JSON.stringify(state);
    if (json === lastSession.current) return;
    lastSession.current = json;
    void local.setPref("session.tabs", state).catch(() => undefined);
  }, []);

  /** Abre de volta as tabelas guardadas de um banco que acabou de ser aberto (lendo o schema se preciso). */
  const restoreTables = async (connId: string, dbName: string) => {
    const key = `${connId}/${dbName}`;
    if (restoring.current.has(key)) return;
    const mine = pendingTables.current.filter((x) => x.connectionId === connId && x.database === dbName);
    if (!mine.length) return;
    if (!connectionsRef.current.find((c) => c.id === connId)?.databases.some((d) => d.name === dbName)) return;
    restoring.current.add(key);
    for (const saved of mine) {
      pendingTables.current = pendingTables.current.filter((x) => x !== saved);
      if (saved.kind === "query") {
        const id = `doc${++tabSeq.current}`;
        queryTexts.current.set(id, saved.sql ?? "");
        const tab: DocTab = { id, kind: "query", title: saved.title, windowTitle: saved.title, icon: "query", sql: saved.sql ?? "", connectionId: connId, database: saved.database, schema: saved.schema || undefined };
        const at = Math.min(Math.max(saved.pos ?? tabsRef.current.length, 1), tabsRef.current.length);
        tabsRef.current = [...tabsRef.current.slice(0, at), tab, ...tabsRef.current.slice(at)];
        setTabs((ts) => { const i = Math.min(at, ts.length); return [...ts.slice(0, i), tab, ...ts.slice(i)]; });
        if (saved.active) setActiveTab(id);
        continue;
      }
      // O estado novo só chega às refs depois de renderizar: espera o schema aparecer (e depois ficar lido) antes de olhar.
      const schemaNow = () => connectionsRef.current.find((c) => c.id === connId)?.databases.find((d) => d.name === saved.database)?.schemas.find((s) => s.name === saved.schema);
      const until = async (done: () => boolean) => { for (let i = 0; i < 40 && !done(); i++) await new Promise((r) => setTimeout(r, 50)); };
      try {
        await loadDatabase(connId, saved.database);
        await until(() => !!schemaNow());
        await loadSchema(connId, saved.database, saved.schema);
        await until(() => !!schemaNow()?.loaded);
      } catch { continue; }
      const sc = schemaNow();
      const list = saved.kind === "view" ? sc?.objects.view : saved.kind === "matview" ? sc?.objects.matview : sc?.objects.table;
      // Objeto que não existe mais: a aba simplesmente não volta.
      if (!list?.some((o) => o.name === saved.object)) continue;
      const taken = tabsRef.current.some((x) => x.kind === "data" && x.connectionId === connId && x.database === saved.database && x.schema === saved.schema && x.object === saved.object);
      if (taken) continue;
      const id = `doc${++tabSeq.current}`;
      const tab: DocTab = {
        id, kind: "data", connectionId: connId, database: saved.database, schema: saved.schema, object: saved.object,
        objectKind: saved.kind === "table" ? undefined : saved.kind, title: saved.title, windowTitle: saved.title,
        icon: saved.kind === "view" ? "view" : saved.kind === "matview" ? "matview" : "table",
      };
      // Volta na posição em que estava na barra (limitada ao que existe agora).
      const at = Math.min(Math.max(saved.pos ?? tabsRef.current.length, 1), tabsRef.current.length);
      tabsRef.current = [...tabsRef.current.slice(0, at), tab, ...tabsRef.current.slice(at)];
      setTabs((ts) => { const i = Math.min(at, ts.length); return [...ts.slice(0, i), tab, ...ts.slice(i)]; });
      if (saved.active) setActiveTab(id);
    }
    restoring.current.delete(key);
  };
  // O usuário abriu um banco: voltam as tabelas que estavam abertas nele (abrir só a conexão não traz nada).
  const openDbKey = [...openDatabases].join("|");
  useEffect(() => {
    const seen = new Set<string>();
    for (const p of pendingTables.current) {
      const key = `${p.connectionId}/${p.database}`;
      if (seen.has(key) || !openDatabases.has(`db:${seg(p.connectionId)}/${seg(p.database)}`)) continue;
      seen.add(key);
      void restoreTables(p.connectionId, p.database);
    }
    // `restoreTables` lê o estado atual por refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openDbKey]);

  /**
   * Fechar a conexão (ou um banco) fecha as abas de tabela dele, mas elas ficam guardadas e voltam quando o banco for aberto de novo;
   * só o X da própria aba faz a tabela ser esquecida. Com alterações não gravadas, pergunta antes. `then` roda em seguida.
   */
  const parkTables = (connId: string, dbName: string | undefined, then: () => void) => {
    const mine = tabsRef.current.filter((x) => x.connectionId === connId && x.database && (!dbName || x.database === dbName)
      && ((x.kind === "data" && x.schema && x.object) || x.kind === "query"));
    const run = () => {
      for (const x of mine) {
        const entry = savedFromTab(x, tabsRef.current, activeTabRef.current, queryTexts.current);
        // Consultas sempre entram (podem ter o mesmo título); tabelas iguais não se repetem.
        pendingTables.current = [...pendingTables.current.filter((p) => x.kind === "query" || !(p.connectionId === connId && p.database === entry.database && p.schema === entry.schema && p.object === entry.object)), entry];
      }
      if (mine.length) {
        const ids = new Set(mine.map((x) => x.id));
        setTabs((ts) => ts.filter((x) => !ids.has(x.id)));
        if (ids.has(activeTabRef.current)) setActiveTab("objects");
      }
      then();
    };
    // Tabela com edições não gravadas ou consulta com transação aberta: pergunta (o texto da consulta fica guardado, não conta).
    if (mine.some((x) => (x.kind === "query" ? txTabs.current.has(x.id) : x.dirty))) setDialog({ kind: "closeDbDirty", run });
    else run();
  };

  // Ao iniciar: "Continue where you left off" reabre as consultas da sessão anterior.
  useEffect(() => {
    if (!isDesktop || !profilesLoaded || !optionsLoaded || sessionReady.current) return;
    const done = () => { sessionReady.current = true; };
    if (appOptions.startup !== "continue") return done();
    void local.getPref<{ tabs?: { title: string; connectionId?: string; database?: string; schema?: string; sql: string }[]; active?: number; tables?: SavedTable[]; activeTable?: number }>("session.tabs")
      .then((saved) => {
        const known = new Set(connectionsRef.current.map((c) => c.id));
        pendingTables.current = (Array.isArray(saved?.tables) ? saved.tables : [])
          .filter((x) => x && known.has(x.connectionId) && typeof x.database === "string" && typeof x.schema === "string" && typeof x.object === "string" && typeof x.title === "string" && (x.kind !== "query" || typeof x.sql === "string"))
          .map((x, i) => ({ ...x, kind: x.kind === "view" || x.kind === "matview" || x.kind === "query" ? x.kind : "table", active: i === saved?.activeTable }));
        const restored: DocTab[] = (saved?.tabs ?? []).filter((x) => x && typeof x.sql === "string").map((x) => {
          const keep = !!x.connectionId && known.has(x.connectionId);
          const id = `doc${++tabSeq.current}`;
          queryTexts.current.set(id, x.sql);
          return { id, kind: "query", title: String(x.title || t("tab.untitledQuery")), windowTitle: String(x.title || t("tab.untitledQuery")), icon: "query", sql: x.sql,
            connectionId: keep ? x.connectionId : undefined, database: keep ? x.database : undefined, schema: keep ? x.schema : undefined };
        });
        if (restored.length) {
          setTabs((ts) => [...ts, ...restored]);
          setActiveTab(restored[Math.min(Math.max(saved?.active ?? 0, 0), restored.length - 1)].id);
        }
      })
      .catch(() => undefined)
      .finally(done);
    // Uma vez, quando perfis e opções já foram lidos.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profilesLoaded, optionsLoaded]);

  useEffect(() => {
    const id = window.setTimeout(saveSession, 600);
    return () => window.clearTimeout(id);
  }, [tabs, activeTab, saveSession]);
  useEffect(() => {
    if (!isDesktop || !appOptions.autoSaveQueries) return;
    const id = window.setInterval(saveSession, appOptions.autoSaveSeconds * 1000);
    return () => window.clearInterval(id);
  }, [appOptions.autoSaveQueries, appOptions.autoSaveSeconds, saveSession]);
  useEffect(() => {
    window.addEventListener("beforeunload", saveSession);
    return () => window.removeEventListener("beforeunload", saveSession);
  }, [saveSession]);

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
            const key = tab.objectKind === "matview" ? "matview" : "view";
            const views = s.objects[key] ?? [];
            const prev = views.find((x) => x.name === name);
            const merged: DbObject = { ...prev, name, definition, comment, owner: owner || prev?.owner };
            const next = prev ? views.map((x) => (x.name === name ? merged : x)) : [...views, merged];
            return { ...s, objects: { ...s.objects, [key]: next.sort((a, b) => a.name.localeCompare(b.name)) } };
          }),
        }),
      };
    }));
    const c = resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections);
    const title = objectTitle(name, tab.objectKind === "matview" ? "tb.matview" : "tb.view", c);
    setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
  };

  /* ---------- Conexões: salvar/excluir ---------- */
  const saveConnection = (form: ConnectionForm, version: number | null, editing?: ConnectionMock, group?: string) => {
    const base = {
      name: form.name, provider: form.provider, host: form.host, port: Number(form.port) || 0, user: form.user,
      initialDatabase: form.initialDatabase, environment: form.environment,
      sslMode: (form.useSsl ? form.sslMode : "disable") as ConnectionMock["sslMode"],
      customDbs: form.customDbList ? form.databases : undefined,
      remarks: form.remarks.trim() ? form.remarks : undefined,
      ssh: form.useSsh ? { host: form.sshHost.trim(), port: Number(form.sshPort) || 22, user: form.sshUser, auth: form.sshAuth, keyPath: form.sshKeyPath, savePassword: form.sshSavePassword } : undefined,
    };
    const id = editing?.id ?? `c${Date.now().toString(36)}`;
    // "Salvar senha" marcado: vai para o cofre do sistema (app desktop). Desmarcado: só na memória
    // desta sessão, para conectar agora sem gravar em lugar nenhum.
    const keepInMemory = () => {
      if (!isDesktop && form.savePassword && form.password) sessionPasswords.set(id, form.password);
      else if (!form.savePassword && form.password) sessionPasswords.set(id, form.password);
      else sessionPasswords.delete(id);
      if (form.useSsh && !form.sshSavePassword && form.sshPassword) sessionPasswords.set(`${id}:ssh`, form.sshPassword);
      else sessionPasswords.delete(`${id}:ssh`);
    };
    const hasPassword = form.savePassword && (!!form.password || !!editing?.hasPassword);
    const apply = () => {
      keepInMemory();
      if (editing) {
        setConnections((list) => list.map((c) => (c.id === id ? { ...c, ...base, hasPassword } : c)));
      } else {
        const conn = { ...profileToConnection({ id, ...base }), hasPassword, group };
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
    const record = toRecord({ ...(current ?? profileToConnection({ id, ...base })), ...base, hasPassword, ...(current ? {} : { group }) }, current ? connections.indexOf(current) : connections.length);
    // Marca como persistido antes de atualizar o estado, para a sincronização não regravar sem a senha.
    persistedProfiles.current.set(id, profileKey(record));
    void local
      .saveProfile(record, form.savePassword ? form.password || null : null, form.savePassword)
      // Senha do túnel SSH: no cofre, ou só nesta sessão quando "Salvar senha" está desmarcado.
      .then(() => local.sshPasswordSave(id, form.sshPassword || null, form.useSsh && form.sshSavePassword))
      .then(apply)
      .catch((e) => {
        persistedProfiles.current.delete(id);
        showError(t("cmd.editConnection"), e);
      });
  };

  /* ---------- Grupos de conexões ---------- */
  const groupError = (title: string, message: string) => setDialog({ kind: "error", title, message });
  const groupNameProblem = (name: string, except?: string): string | null => {
    if (!name) return t("grp.empty");
    if (allGroups.some((g) => g.toLowerCase() === name.toLowerCase() && g !== except)) return t("grp.exists", { name });
    return null;
  };
  const newGroup = (then?: (name: string) => void) =>
    askName(t("cmd.newGroup"), t("grp.name"), "", (value) => {
      setDialog(null);
      const name = value.trim();
      const problem = groupNameProblem(name);
      if (problem) return groupError(t("cmd.newGroup"), problem);
      setGroups([...allGroups, name]);
      setExpanded((s) => add(s, `group:${seg(name)}`));
      then?.(name);
    });
  const renameGroup = (old: string) =>
    askName(t("cmd.renameGroup"), t("grp.name"), old, (value) => {
      setDialog(null);
      const name = value.trim();
      if (name === old) return;
      const problem = groupNameProblem(name, old);
      if (problem) return groupError(t("cmd.renameGroup"), problem);
      setGroups(allGroups.map((g) => (g === old ? name : g)));
      setConnections((list) => list.map((c) => (c.group === old ? { ...c, group: name } : c)));
      setExpanded((s) => (s.has(`group:${seg(old)}`) ? add(remove(s, `group:${seg(old)}`), `group:${seg(name)}`) : s));
    });
  /** Apagar o grupo não apaga as conexões: elas voltam para fora de grupo. */
  const deleteGroup = (name: string) => {
    setGroups(allGroups.filter((g) => g !== name));
    setConnections((list) => list.map((c) => (c.group === name ? { ...c, group: undefined } : c)));
  };
  const moveToGroup = (connId: string, group: string | null) => {
    updateConn(connId, (c) => ({ ...c, group: group ?? undefined }));
    if (group) setExpanded((s) => add(s, `group:${seg(group)}`));
  };
  const moveToGroupMenu = (conn: ConnectionMock): MenuEntry => ({
    kind: "sub", label: "cmd.moveToGroup",
    items: [
      { kind: "item", text: t("grp.none"), checked: !conn.group, run: () => moveToGroup(conn.id, null) },
      ...allGroups.map((g): MenuEntry => ({ kind: "item", text: g, checked: conn.group === g, run: () => moveToGroup(conn.id, g) })),
      { kind: "sep" },
      { kind: "item", text: t("cmd.newGroup"), run: () => newGroup((name) => moveToGroup(conn.id, name)) },
    ],
  });
  useEffect(() => {
    if (!isDesktop || !profilesLoaded) return;
    void local.setPref("conn.groups", allGroups).catch(() => undefined);
  }, [allGroups, profilesLoaded]);

  /** Arquivo → Exportar conexões: todas as conexões salvas, sem as senhas. */
  const exportConnections = async () => {
    const path = await chooseExportPath("atlas-connections.json", ["json"]);
    if (!path) return;
    try {
      const n = await local.connectionsExport(path, connections.map((c, i) => toRecord(c, i)));
      setDialog({ kind: "info", title: t("cmd.exportConnections"), message: t("conn.exported", { n, path }) });
    } catch (e) {
      showError(t("cmd.exportConnections"), e);
    }
  };

  /** Arquivo → Importar conexões: entram como conexões novas (id novo; nome repetido ganha " (2)") e sem senha. */
  const importConnections = async () => {
    const path = await chooseOpenFile(["json"]);
    if (!path) return;
    try {
      const records = await local.connectionsImport(path);
      const names = new Set(connections.map((c) => c.name.toLowerCase()));
      const stamp = Date.now().toString(36);
      const added = records.map((r, i) => {
        let name = r.name;
        for (let k = 2; names.has(name.toLowerCase()); k++) name = `${r.name} (${k})`;
        names.add(name.toLowerCase());
        return fromRecord({ ...r, id: `c${stamp}${i}`, name, hasPassword: false });
      });
      if (added.length) setConnections((list) => [...list, ...added]);
      setDialog({ kind: "info", title: t("cmd.importConnections"), message: t("conn.imported", { n: added.length }) });
    } catch (e) {
      showError(t("cmd.importConnections"), e);
    }
  };

  /* ---------- Menus de contexto ---------- */
  const showContextMenu = (node: TreeNode, x: number, y: number) => {
    const c = resolveNode(node.id, connections);
    const items: MenuEntry[] = [];
    const item = (text: MessageKey, run?: () => void): MenuEntry => ({ kind: "item", text: t(text), run });
    const sep: MenuEntry = { kind: "sep" };
    if (node.kind === "root") items.push(item("cmd.newConnection", () => setDialog({ kind: "connection" })), item("cmd.newGroup", () => newGroup()));
    if (node.kind === "group") {
      const name = node.label;
      items.push(
        item("cmd.newConnection", () => setDialog({ kind: "connection", group: name })),
        item("cmd.newGroup", () => newGroup()),
        sep,
        item("cmd.renameGroup", () => renameGroup(name)),
        item("cmd.deleteGroup", () => deleteGroup(name)),
      );
    }
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
        moveToGroupMenu(conn),
        sep,
        item("cmd.newDatabase", isOpen && live ? () => setDialog({ kind: "newDatabase", connId: conn.id }) : undefined),
        item("cmd.newQuery", () => (isOpen ? newQuery() : newQuery(undefined, conn))),
        sep,
        item("cmd.console", isOpen && conn.live ? () => openConsole(conn) : undefined),
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
    // Despejar arquivo SQL: banco, schema ou tabela (estrutura e dados, ou só a estrutura).
    const dumpMenu = (scope: DumpScope, label: string, connId: string, database: string): MenuEntry => ({
      kind: "sub", label: "sub.dumpSql",
      items: [
        { kind: "item", text: t("cmd.dumpStructData"), run: live ? () => setDialog({ kind: "dump", connId, database, scope, label, structureOnly: false }) : undefined },
        { kind: "item", text: t("cmd.dumpStructOnly"), run: live ? () => setDialog({ kind: "dump", connId, database, scope, label, structureOnly: true }) : undefined },
      ],
    });

    if (node.kind === "database" && c.connection && c.database) {
      const conn = c.connection;
      const dbName = c.database.name;
      const isOpen = openDatabases.has(node.id);
      const isInitial = dbName === (conn.initialDatabase || "postgres");
      items.push(
        isOpen
          ? item("cmd.closeDatabase", () => parkTables(conn.id, dbName, () => {
              setOpenDatabases((s) => remove(s, node.id));
              if (live) void pg.closeDatabase(conn.id, dbName).catch(() => undefined);
            }))
          : item("cmd.openDatabase", () => toggle(node)),
        sep,
        item("cmd.newDatabase", live ? () => setDialog({ kind: "newDatabase", connId: conn.id }) : undefined),
        item("cmd.newSchema", isOpen && live ? () => promptNewSchema(conn.id, dbName) : undefined),
        item("cmd.editDatabase", live ? () => void openEditDatabase(conn.id, dbName, node.id) : undefined),
        // O banco inicial da conexão não pode ser excluído por ela mesma (a sessão está nele).
        item("cmd.deleteDatabase", live && !isInitial ? () => askDdl({
          title: t("cmd.deleteDatabase"), message: confirmDelete("database", dbName), statements: [`DROP DATABASE ${quoteIdent(dbName)};`],
          connId: conn.id, database: conn.initialDatabase || "postgres", refresh: "connection",
          danger: true, transactional: false, confirmText: dbName, closeDbFirst: dbName,
          after: () => setOpenDatabases((s) => remove(s, node.id)),
        }) : undefined),
        sep,
        item("cmd.newQuery", () => newQuery()),
        item("cmd.console", isOpen && live ? () => openConsole(conn, dbName) : undefined),
        item("cmd.executeSqlFile", isOpen && live ? () => pickSqlFile(conn.id, dbName) : undefined),
        dumpMenu({ kind: "database" }, dbName, conn.id, dbName),
        item("cmd.newBackup", live ? () => void openBackup(c, null) : undefined),
        item("cmd.restoreBackup", live ? () => openRestore("restore", c) : undefined),
        item("cmd.extractSql", live ? () => openRestore("extract", c) : undefined),
        sep,
        item("cmd.reverseToModel"),
        item("cmd.findInDatabase", live ? () => openFind(conn.id, dbName) : undefined),
        sep,
        item("cmd.refresh", isOpen && live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "schema" && c.connection && c.database && c.schema) {
      const conn = c.connection;
      const dbName = c.database.name;
      const scName = c.schema.name;
      if (conn.live) items.push(item("cmd.findInSchema", () => openFind(conn.id, dbName, scName)), sep);
      const scPrefix = `${seg(conn.id)}/${seg(dbName)}/${seg(scName)}`;
      const closeSchema = () => {
        // Colapsa o schema e qualquer descendente (pastas, objetos, colunas),
        // e marca como não carregado para a próxima abertura rebuscar do servidor.
        setExpanded((s) => {
          const n = new Set(s);
          for (const id of n) {
            if (id === node.id || id.startsWith(`folder:${scPrefix}/`) || id.startsWith(`object:${scPrefix}/`) || id.startsWith(`col:${scPrefix}/`)) n.delete(id);
          }
          return n;
        });
        setConnections((cs) => cs.map((cc) => cc.id !== conn.id ? cc : {
          ...cc,
          databases: cc.databases.map((d) => d.name !== dbName ? d : {
            ...d,
            schemas: d.schemas.map((s) => s.name !== scName ? s : { ...s, loaded: false, objects: {} }),
          }),
        }));
      };
      items.push(
        item("cmd.closeSchema", closeSchema),
        sep,
        item("cmd.newSchema", live ? () => promptNewSchema(conn.id, dbName) : undefined),
        item("cmd.editSchema", live ? () => void openEditSchema(conn.id, dbName, scName) : undefined),
        item("cmd.deleteSchema", live ? () => askDdl({
          title: t("cmd.deleteSchema"), message: confirmDelete("schema", scName), statements: [`DROP SCHEMA ${quoteIdent(scName)};`],
          connId: conn.id, database: dbName, refresh: "database", danger: true, confirmText: prod ? scName : undefined,
          note: t("msg.dropSchemaRestrict"),
        }) : undefined),
        sep,
        item("act.newTable", newTable),
        item("cmd.newQuery", () => newQuery()),
        item("cmd.executeSqlFile", live ? () => pickSqlFile(conn.id, dbName) : undefined),
        dumpMenu({ kind: "schema", schema: scName }, scName, conn.id, dbName),
        item("cmd.newBackup", live ? () => void openBackup(c, scName) : undefined),
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
      const foreign = c.schema?.objects.table?.find((o) => o.name === name)?.dataType === "foreign";
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
          ...scope, title: t("act.deleteTable"), message: confirmDelete("table", name), statements: [`DROP ${foreign ? "FOREIGN TABLE" : "TABLE"} ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        item("cmd.emptyTable", live && !foreign ? () => askDdl({
          ...scope, title: t("cmd.emptyTable"), message: t("msg.confirmEmpty", { name }), statements: [`DELETE FROM ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        item("cmd.truncateTable", live && !foreign ? () => askDdl({
          ...scope, title: t("cmd.truncateTable"), message: t("msg.confirmTruncate", { name }), statements: [`TRUNCATE TABLE ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        {
          kind: "sub", label: "sub.duplicateTable",
          items: [
            { kind: "item", text: t("cmd.dupStructData"), run: live && !foreign ? () => duplicate(true) : undefined },
            { kind: "item", text: t("cmd.dupStructure"), run: live && !foreign ? () => duplicate(false) : undefined },
          ],
        },
        sep,
        item("act.importWizard", live ? () => startImport(c, name) : undefined),
        item("act.exportWizard", live ? () => startExport(c, name) : undefined),
        dumpMenu({ kind: "tables", schema: c.schema?.name ?? "public", tables: [name] }, name, c.connection?.id ?? "", c.database?.name ?? ""),
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

    if (node.kind === "folder" && node.category === "user") {
      items.push(
        item("act.newUser", newRole),
        sep,
        item("cmd.refresh", live && c.connection ? () => { void reloadRoles(c.connection!.id).catch((e) => showError(t("cmd.refresh"), e)); } : undefined),
      );
    }

    if (node.kind === "object" && node.category === "user" && c.connection && c.objectName) {
      const name = c.objectName;
      const conn = c.connection;
      items.push(
        item("act.editUser", () => openObjects("design", [name], "user", c)),
        item("act.newUser", newRole),
        item("act.deleteUser", live ? () => askDdl({
          title: t("act.deleteUser"), message: confirmDelete("role", name), statements: [dropRoleSql(name)], connId: conn.id, database: c.database?.name ?? conn.initialDatabase,
          refresh: "roles", danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        sep,
        item("cmd.refresh", live ? () => { void reloadRoles(conn.id).catch((e) => showError(t("cmd.refresh"), e)); } : undefined),
      );
    }

    if (node.kind === "folder" && node.category === "type") {
      items.push(
        item("act.new", newType),
        item("cmd.newQuery", () => newQuery()),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "object" && node.category === "type" && c.connection && c.database && c.schema && c.objectName) {
      const scName = c.schema.name;
      const name = c.objectName;
      const kind = c.object?.dataType ?? "";
      const scope = { connId: c.connection.id, database: c.database.name, schema: scName, refresh: "schema" as const };
      items.push(
        item("act.design", () => openObjects("design", [name], "type", c)),
        item("act.new", newType),
        item("act.delete", live ? () => askDdl({
          ...scope, title: t("act.delete"), message: confirmDelete("type", name), statements: [dropTypeSql(scName, name, kind)], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
      );
    }

    if (node.kind === "folder" && node.category === "sequence") {
      items.push(
        item("act.new", newSequence),
        item("cmd.newQuery", () => newQuery()),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "object" && node.category === "sequence" && c.connection && c.database && c.schema && c.objectName) {
      const scName = c.schema.name;
      const name = c.objectName;
      const scope = { connId: c.connection.id, database: c.database.name, schema: scName, refresh: "schema" as const };
      items.push(
        item("act.design", () => openObjects("design", [name], "sequence", c)),
        item("act.new", newSequence),
        item("act.delete", live ? () => askDdl({
          ...scope, title: t("act.delete"), message: confirmDelete("sequence", name), statements: [dropSeqSql(scName, name)], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
      );
    }

    if (node.kind === "folder" && node.category === "extension") {
      items.push(
        item("act.new", newExtension),
        item("cmd.newQuery", () => newQuery()),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "object" && node.category === "extension" && c.connection && c.database && c.schema && c.objectName) {
      const name = c.objectName;
      const scope = { connId: c.connection.id, database: c.database.name, schema: c.schema.name, refresh: "extensions" as const };
      items.push(
        item("act.design", () => openObjects("design", [name], "extension", c)),
        item("act.new", newExtension),
        item("act.delete", live ? () => askDdl({
          ...scope, title: t("act.delete"), message: confirmDelete("extension", name), note: t("ext.deleteNote"),
          statements: [dropExtSql(name)], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
      );
    }

    if (node.kind === "folder" && node.category === "foreignServer") {
      items.push(
        item("act.new", newForeignServer),
        item("cmd.newQuery", () => newQuery()),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "object" && node.category === "tablespace" && c.connection && c.database && c.schema && c.objectName) {
      const name = c.objectName;
      const scope = { connId: c.connection.id, database: c.database.name, schema: c.schema.name, refresh: "extensions" as const };
      const system = name.toLowerCase().startsWith("pg_");
      items.push(
        item("act.design", () => openObjects("design", [name], "tablespace", c)),
        item("act.new", newTablespace),
        item("act.delete", live && !system ? () => askDdl({
          ...scope, title: t("act.delete"), message: confirmDelete("tablespace", name), note: t("tsp.deleteNote"),
          statements: [dropTablespaceStatement(name)], transactional: false, danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
      );
    }

    if (node.kind === "object" && node.category === "foreignServer" && c.connection && c.database && c.schema && c.objectName) {
      const name = c.objectName;
      const scope = { connId: c.connection.id, database: c.database.name, schema: c.schema.name, refresh: "extensions" as const };
      items.push(
        item("act.design", () => openObjects("design", [name], "foreignServer", c)),
        item("act.new", newForeignServer),
        item("act.delete", live ? () => askDdl({
          ...scope, title: t("act.delete"), message: confirmDelete("foreignServer", name), note: t("fsv.deleteNote"),
          statements: dropServerStatements(name), danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
      );
    }

    if (node.kind === "folder" && node.category === "function") {
      items.push(
        item("act.newFunction", newFunction),
        item("cmd.newQuery", () => newQuery()),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "object" && node.category === "function" && c.connection && c.database && c.schema && c.objectName && c.object?.fnName) {
      const scName = c.schema.name;
      const fn = c.object;
      const scope = { connId: c.connection.id, database: c.database.name, schema: scName, refresh: "schema" as const };
      items.push(
        item("act.designFunction", () => openObjects("design", [fn.name], "function", c)),
        item("act.newFunction", newFunction),
        item("act.deleteFunction", live ? () => askDdl({
          ...scope, title: t("act.deleteFunction"), message: confirmDelete(fn.procedure ? "procedure" : "function", fn.name), statements: [dropFuncSql(scName, fn.fnName ?? fn.name, fn.args ?? "", !!fn.procedure)], danger: true, confirmText: prod ? fn.fnName : undefined,
        }) : undefined),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(fn.fnName ?? fn.name).catch(() => undefined); }),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
      );
    }

    if (node.kind === "folder" && node.category === "matview") {
      items.push(
        item("act.newMatview", newMatview),
        item("cmd.newQuery", () => newQuery()),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
      );
    }

    if (node.kind === "object" && node.category === "matview" && c.connection && c.database && c.schema && c.objectName) {
      const scName = c.schema.name;
      const name = c.objectName;
      const target = `${quoteIdent(scName)}.${quoteIdent(name)}`;
      const scope = { connId: c.connection.id, database: c.database.name, schema: scName, refresh: "schema" as const };
      items.push(
        item("act.openMatview", () => openObjects("open", [name], "matview", c)),
        item("act.designMatview", () => openObjects("design", [name], "matview", c)),
        item("act.newMatview", newMatview),
        item("act.deleteMatview", live ? () => askDdl({
          ...scope, title: t("act.deleteMatview"), message: confirmDelete("matview", name), statements: [`DROP MATERIALIZED VIEW ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("act.refreshMatview", live ? () => refreshMatview(c, name, false) : undefined),
        item("act.refreshMatviewConcurrently", live ? () => refreshMatview(c, name, true) : undefined),
        sep,
        item("act.exportWizard", live ? () => startExport(c, name) : undefined),
        sep,
        item("cmd.copyName", () => { void navigator.clipboard?.writeText(name).catch(() => undefined); }),
        item("cmd.renameView", live ? () => askName(t("cmd.renameView"), t("msg.newName"), name, (newName) => askDdl({
          ...scope, title: t("cmd.renameView"), statements: [`ALTER MATERIALIZED VIEW ${target} RENAME TO ${quoteIdent(newName)};`],
        })) : undefined),
        sep,
        item("cmd.newQuery", () => newQuery(`SELECT * FROM ${target} LIMIT 100;`)),
        sep,
        item("cmd.refresh", live ? () => refreshNode(node) : undefined),
        item("cmd.objectInfo", () => patchLayout({ infoVisible: true })),
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
          ...scope, title: t("act.deleteView"), message: confirmDelete("view", name), statements: [`DROP VIEW ${target};`], danger: true, confirmText: prod ? name : undefined,
        }) : undefined),
        sep,
        item("act.exportWizard", live ? () => startExport(c, name) : undefined),
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

  // Sem o menu padrão do navegador (Voltar/Atualizar/Imprimir/Inspecionar): só os menus do Atlas.
  // Campos de texto e o editor SQL mantêm o menu nativo de recortar/copiar/colar.
  useEffect(() => {
    const block = (e: MouseEvent) => {
      const el = e.target as HTMLElement | null;
      if (el?.closest?.("input, textarea, [contenteditable='true'], .cm-editor")) return;
      e.preventDefault();
    };
    window.addEventListener("contextmenu", block);
    return () => window.removeEventListener("contextmenu", block);
  }, []);

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
    newTableForeign: { id: "newTableForeign", label: "cmd.newTableForeign", run: ctx.connection?.live && hasSchema ? newForeignTable : undefined },
    newView: { id: "newView", label: "cmd.newView", run: hasSchema ? newView : undefined },
    newMatview: { id: "newMatview", label: "cmd.newMatview", run: hasSchema ? newMatview : undefined },
    newFunction: { id: "newFunction", label: "cmd.newFunction", run: hasSchema ? newFunction : undefined },
    newRole: { id: "newRole", label: "cmd.newRole", run: ctx.connection?.live ? newRole : undefined },
    newTablespace: { id: "newTablespace", label: "cmd.newTablespace", run: ctx.connection?.live && ctx.schema && openConnections.has(ctx.connection.id) ? newTablespace : undefined },
    newCast: { id: "newCast", label: "cmd.newCast" },
    newForeignServer: { id: "newForeignServer", label: "cmd.newForeignServer", run: ctx.connection?.live && ctx.schema && openConnections.has(ctx.connection.id) ? newForeignServer : undefined },
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
    newBackup: { id: "newBackup", label: "cmd.newBackup", run: () => void openBackup() },
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
    importConnections: { id: "importConnections", label: "cmd.importConnections", run: isDesktop ? () => void importConnections() : undefined },
    exportConnections: { id: "exportConnections", label: "cmd.exportConnections", run: isDesktop && connections.length ? () => void exportConnections() : undefined },
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
    flatten: { id: "flatten", label: "cmd.flatten", checked: layout.flatten ?? false, run: () => patchLayout({ flatten: !(layout.flatten ?? false) }) },
    chooseColumns: { id: "chooseColumns", label: "cmd.chooseColumns", run: () => setDialog({ kind: "chooseColumns" }) },
    showHidden: { id: "showHidden", label: "cmd.showHidden", checked: layout.showHidden ?? false, run: () => toggleShowHidden() },
    viewEr: { id: "viewEr", label: "cmd.viewEr", checked: layout.viewMode === "er", run: () => patchLayout({ viewMode: "er" }) },
    erShowGrid: { id: "erShowGrid", label: "cmd.erShowGrid", checked: layout.erShowGrid ?? true, run: () => patchLayout({ erShowGrid: !(layout.erShowGrid ?? true) }) },
    erSnapGrid: { id: "erSnapGrid", label: "cmd.erSnapGrid", checked: layout.erSnapGrid ?? true, run: () => patchLayout({ erSnapGrid: !(layout.erSnapGrid ?? true) }) },
    erShowFkNames: { id: "erShowFkNames", label: "cmd.erShowFkNames", checked: layout.erShowFkNames ?? true, run: () => patchLayout({ erShowFkNames: !(layout.erShowFkNames ?? true) }) },
    themeSystem: { id: "themeSystem", label: "cmd.themeSystem", checked: layout.theme === "system", run: () => patchLayout({ theme: "system" }) },
    themeLight: { id: "themeLight", label: "cmd.themeLight", checked: layout.theme === "light", run: () => patchLayout({ theme: "light" }) },
    themeDark: { id: "themeDark", label: "cmd.themeDark", checked: layout.theme === "dark", run: () => patchLayout({ theme: "dark" }) },
    langPt: { id: "langPt", label: "cmd.langPt", checked: layout.lang === "pt-BR", run: () => patchLayout({ lang: "pt-BR" }) },
    langEn: { id: "langEn", label: "cmd.langEn", checked: layout.lang === "en-US", run: () => patchLayout({ lang: "en-US" }) },
    resetLayout: { id: "resetLayout", label: "cmd.resetLayout", run: () => setLayout({ ...DEFAULT_LAYOUT, lang: layout.lang, theme: layout.theme }) },
    addFavorite: { id: "addFavorite", label: "cmd.addFavorite" },
    manageFavorites: { id: "manageFavorites", label: "cmd.manageFavorites" },
    dataTransfer: {
      id: "dataTransfer", label: "cmd.dataTransfer",
      run: () => {
        const live = connections.filter((c) => c.live);
        if (live.length === 0) return;
        void openDataTransfer(t("dt.title"), {
          connections: live.map((c) => ({
            id: c.id, name: c.name, provider: c.provider, host: c.host, port: c.port, user: c.user,
            version: c.serverVersion, versionNum: c.serverVersionNum, open: openConnections.has(c.id),
            initialDatabase: c.initialDatabase, sslMode: c.sslMode, hasPassword: c.hasPassword,
          })),
          source: ctx.connection?.live && ctx.database && ctx.schema ? { connId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name } : undefined,
        }).catch((e) => showError(t("dt.title"), e));
      },
    },
    dataSync: {
      id: "dataSync", label: "cmd.dataSync",
      run: () => {
        const live = connections.filter((c) => c.live);
        if (live.length === 0) return;
        void openDataSync(t("ds.title"), {
          connections: live.map((c) => ({
            id: c.id, name: c.name, provider: c.provider, host: c.host, port: c.port, user: c.user,
            version: c.serverVersion, versionNum: c.serverVersionNum, open: openConnections.has(c.id),
            initialDatabase: c.initialDatabase, sslMode: c.sslMode, hasPassword: c.hasPassword,
          })),
          source: ctx.connection?.live && ctx.database && ctx.schema ? { connId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name } : undefined,
        }).catch((e) => showError(t("ds.title"), e));
      },
    },
    structureSync: {
      id: "structureSync", label: "cmd.structureSync",
      run: () => {
        const live = connections.filter((c) => c.live);
        if (live.length === 0) return;
        void openStructSync(t("ss.title"), {
          connections: live.map((c) => ({
            id: c.id, name: c.name, provider: c.provider, host: c.host, port: c.port, user: c.user,
            version: c.serverVersion, versionNum: c.serverVersionNum, open: openConnections.has(c.id),
            initialDatabase: c.initialDatabase, sslMode: c.sslMode, hasPassword: c.hasPassword,
          })),
          source: ctx.connection?.live && ctx.database && ctx.schema ? { connId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name } : undefined,
        }).catch((e) => showError(t("ss.title"), e));
      },
    },
    dataGen: {
      id: "dataGen", label: "cmd.dataGen",
      run: () => {
        const live = connections.filter((c) => c.live);
        if (live.length === 0) return;
        void openDataGen(t("dg.title"), {
          connections: live.map((c) => ({
            id: c.id, name: c.name, provider: c.provider, host: c.host, port: c.port, user: c.user,
            version: c.serverVersion, versionNum: c.serverVersionNum, open: openConnections.has(c.id),
            initialDatabase: c.initialDatabase, sslMode: c.sslMode, hasPassword: c.hasPassword,
          })),
          source: ctx.connection?.live && ctx.database && ctx.schema ? { connId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name } : undefined,
        }).catch((e) => showError(t("dg.title"), e));
      },
    },
    dataDictionary: {
      id: "dataDictionary", label: "cmd.dataDictionary",
      run: () => {
        const live = connections.filter((c) => c.live);
        if (live.length === 0) return;
        void openDataDictionary(t("dd.title"), {
          connections: live.map((c) => ({
            id: c.id, name: c.name, provider: c.provider, host: c.host, port: c.port, user: c.user,
            version: c.serverVersion, versionNum: c.serverVersionNum, open: openConnections.has(c.id),
            initialDatabase: c.initialDatabase, sslMode: c.sslMode, hasPassword: c.hasPassword,
          })),
          source: ctx.connection?.live && ctx.database && ctx.schema ? { connId: ctx.connection.id, database: ctx.database.name, schema: ctx.schema.name } : undefined,
        }).catch((e) => showError(t("dd.title"), e));
      },
    },
    console: {
      id: "console", label: "cmd.console", shortcut: "F6",
      run: ctx.connection?.live && openConnections.has(ctx.connection.id) ? () => openConsole(ctx.connection!, ctx.database?.name) : undefined,
    },
    scheduledTasks: { id: "scheduledTasks", label: "cmd.scheduledTasks", run: isDesktop ? () => setDialog({ kind: "tasks" }) : undefined },
    historyLog: {
      id: "historyLog", label: "cmd.historyLog", shortcut: "Ctrl+L",
      run: () => openTab({ kind: "historyLog", title: t("hl.title"), windowTitle: t("hl.title"), icon: "history", object: "historylog" }),
    },
    findInDatabase: {
      id: "findInDatabase", label: "cmd.findInDatabase",
      run: connections.some((c) => c.live)
        ? () => openFind(ctx.connection?.live ? ctx.connection.id : undefined, ctx.connection?.live ? ctx.database?.name : undefined, ctx.connection?.live ? ctx.schema?.name : undefined)
        : undefined,
    },
    monitorPostgres: {
      id: "monitorPostgres", label: "mon.postgres",
      run: () => {
        if (!connections.some((c) => c.live)) return;
        openTab({ kind: "monitor", title: `${t("mon.title")} - PostgreSQL`, windowTitle: `${t("mon.title")} - PostgreSQL`, icon: "settings", object: "monitor", connectionId: ctx.connection?.id });
      },
    },
    options: { id: "options", label: "cmd.options", run: () => setDialog({ kind: "options" }) },
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
    minify: { id: "minify", label: "cmd.minify", run: active.kind === "query" ? () => qa()?.minify() : undefined },
    queryBuilder: { id: "queryBuilder", label: "cmd.queryBuilder" },
    indent: { id: "indent", label: "cmd.indent", shortcut: "Tab" },
    outdent: { id: "outdent", label: "cmd.outdent", shortcut: "Shift+Tab" },
    toggleComment: { id: "toggleComment", label: "cmd.comment", shortcut: "Ctrl+/" },
    upper: { id: "upper", label: "cmd.upper", run: active.kind === "query" ? () => qa()?.upper() : undefined },
    lower: { id: "lower", label: "cmd.lower", run: active.kind === "query" ? () => qa()?.lower() : undefined },
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
  // Exibir → Ordenar: as colunas da lista de objetos da categoria atual (marca a ordenação vigente) e o sentido.
  const sortKey = layout.objSortKey ?? "name";
  const sortDir = layout.objSortDir ?? 1;
  const sortSubmenu: MenuEntry = {
    kind: "sub",
    label: "cmd.sort",
    items: [
      ...objectColumns(category, layout.lang, ctx.connection?.provider).map((c): MenuEntry => ({
        kind: "item", text: t(c.label), checked: sortKey === c.key || undefined, run: () => patchLayout({ objSortKey: c.key }),
      })),
      { kind: "sep" },
      { kind: "item", text: t("cmd.sortAsc"), checked: sortDir === 1 || undefined, run: () => patchLayout({ objSortDir: 1 }) },
      { kind: "item", text: t("cmd.sortDesc"), checked: sortDir === -1 || undefined, run: () => patchLayout({ objSortDir: -1 }) },
    ],
  };
  const viewMenu: MenuDef = {
    label: "menu.view",
    items: entries("navigationPane", "informationPane", "-", "viewList", "viewDetail", "viewIcons", "viewEr", "-", sortSubmenu, "chooseColumns", "-", "flatten", "showHidden", "-", "erShowGrid", "erSnapGrid", "erShowFkNames", "-", "resetLayout"),
  };
  const queryMenu: MenuDef = { label: "menu.query", items: entries("run", "runSelected", "stop", "-", "explain", "-", "beautify", "minify", "queryBuilder") };
  const formatMenu: MenuDef = { label: "menu.format", items: entries("indent", "outdent", "-", "toggleComment", "-", "upper", "lower") };
  const tableMenu: MenuDef = { label: "menu.table", items: entries("addRecord", "deleteRecord", "-", "applyChanges", "discardChanges", "-", "refreshData") };
  const tailMenus: MenuDef[] = [
    { label: "menu.favorites", items: entries("addFavorite", "manageFavorites") },
    { label: "menu.tools", items: entries("dataTransfer", "dataGen", "dataDictionary", "dataSync", "structureSync", "-", "console", sub("mon.title", "monitorPostgres"), "-", "findInDatabase", "-", "scheduledTasks", "historyLog", "-", "options") },
    { label: "menu.window", items: entries("nextTab", "previousTab", "-", "closeTab", "closeAllTabs") },
    { label: "menu.help", items: entries("documentation", "-", "about") },
  ];
  // A barra de menus muda conforme o documento ativo (CONFIRMADO_EM_CAPTURA: Query/Format e Table).
  const menus: MenuDef[] = [
    fileMenu, editMenu, viewMenu,
    ...(active.kind === "query" ? [queryMenu, formatMenu] : active.kind === "data" ? [tableMenu] : []),
    ...tailMenus,
  ];

  // F5/Ctrl+R/Ctrl+F5/Ctrl+Shift+R nunca recarregam a janela (perderia abas e edições pendentes).
  // F5 atualiza o documento ativo: a tabela aberta trata no próprio TableDataDoc; na aba
  // Objetos, relê o nó selecionado na árvore. Ctrl+R no editor SQL continua sendo "Executar".
  const f5Ref = useRef<() => void>(() => undefined);
  f5Ref.current = () => {
    if (active.kind !== "objects") return;
    const node = nodes.find((n) => n.id === selectedNode);
    if (node) refreshNode(node);
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const reload = e.key === "F5" || ((e.ctrlKey || e.metaKey) && (e.key === "r" || e.key === "R"));
      if (!reload) return;
      // No editor SQL, F5/Ctrl+R/Ctrl+Shift+R são "Executar" do próprio CodeMirror (que já
      // impede o recarregamento); um evento cancelado aqui ele ignoraria.
      if (!(e.key === "F5" && e.ctrlKey) && (e.target as HTMLElement | null)?.closest?.(".cm-editor")) return;
      e.preventDefault();
      if (e.key === "F5" && !e.ctrlKey && !document.querySelector(".modal-overlay")) f5Ref.current();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

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
      // Bancos que o filtro da árvore esconde entram na conta de "itens ocultos".
      const matches = makeMatcher(filter, filterOptions);
      const shown = ctx.connection.databases.filter((db) => databaseVisible(db, matches)).length;
      const h = ctx.connection.hiddenDatabases + (ctx.connection.databases.length - shown);
      return { left: h ? t("status.databasesHidden", { n: shown, h }) : t("status.databases", { n: shown }), cells };
    }
    if (selectedNode?.startsWith("db:") && ctx.database && connectionOpen && openDatabases.has(selectedNode)) {
      const matches = makeMatcher(filter, filterOptions);
      const shown = ctx.database.schemas.filter((sc) => schemaVisible(sc, matches)).length;
      const h = ctx.database.schemas.length - shown;
      return { left: h ? t("status.schemasHidden", { n: shown, h }) : t("status.schemas", { n: shown }), cells };
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
    : active.kind === "functionDesigner" ? "function"
    : active.kind === "sequenceDesigner" ? "sequence"
    : active.kind === "typeDesigner" ? "type"
    : active.kind === "extensionDesigner" ? "extension"
    : active.kind === "foreignServerDesigner" ? "foreignServer"
    : active.kind === "tablespaceDesigner" ? "tablespace"
    : active.kind === "foreignTableDesigner" ? "table"
    : active.kind === "roleDesigner" ? "user"
    : active.objectKind === "matview" ? "matview"
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
    execute: (connId, db, sqlText, max, schema) => pg.execute(connId, db, sqlText, max, schema),
    cancel: (connId, db) => pg.cancel(connId, db),
    beginTx: (connId, db) => pg.txBegin(connId, db),
    executeInTx: (txId, sqlText, max, schema) => pg.txExecute(txId, sqlText, max, schema),
    endTx: (txId, commit) => pg.txEnd(txId, commit),
  }).current;

  const dataBackends = useRef<Record<string, TableDataBackend>>({});
  const tableBackendFor = (tab: DocTab, base: DbObject): TableDataBackend => {
    const connId = tab.connectionId ?? "";
    const db = tab.database ?? "";
    const schema = tab.schema ?? "";
    // Transação manual da aba: enquanto aberta, gravação, leitura e contagem usam a conexão dedicada dela.
    let txId: string | null = null;
    return (dataBackends.current[tab.id] ??= {
      loadDetail: async () => tableFromServer(base, await pg.tableDetail(connId, db, schema, tab.object ?? base.name)),
      loadProfile: tab.objectKind === "view" ? undefined : () => pg.tableProfile(connId, db, schema, tab.object ?? base.name),
      beginTx: async () => { txId = await pg.txBegin(connId, db); },
      endTx: async (commit) => {
        const id = txId;
        txId = null;
        if (id) await pg.txEnd(id, commit);
      },
      loadRows: async (orderBy, filter, limit, offset) => {
        const r = txId
          ? await pg.txRows(txId, schema, tab.object ?? base.name, orderBy, filter, limit, offset)
          : await pg.tableRows(connId, db, schema, tab.object ?? base.name, orderBy, filter, limit, offset);
        const rs = r.outcome.statements[0]?.result;
        return { sql: r.sql, rows: rs?.rows ?? [], truncated: !!rs?.truncated };
      },
      apply: async (statements) => {
        if (txId) await pg.txApply(txId, statements);
        else await pg.apply(connId, db, statements);
      },
      cancel: async () => {
        await pg.cancel(connId, db);
      },
      count: async (filter) => (txId
        ? pg.txCount(txId, schema, tab.object ?? base.name, filter)
        : pg.tableCount(connId, db, schema, tab.object ?? base.name, filter)),
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
          labels: (schema, table, keyColumn, keys) => pg.fkLabels(conn.id, db, schema, table, keyColumn, keys),
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
    const own = tab.kind === "functionDesigner" ? sc?.objects.function ?? []
      : tab.objectKind === "matview" ? sc?.objects.matview ?? []
      : tab.objectKind === "view" || tab.kind === "viewDesigner" ? sc?.objects.view ?? [] : tables;
    return { conn, table: own.find((x) => x.name === tab.object), tables };
  };

  const viewBackends = useRef<Record<string, ViewBackend>>({});
  const viewBackendFor = (tab: DocTab, conn: ConnectionMock): ViewBackend => {
    const db = tab.database ?? "";
    return (viewBackends.current[tab.id] ??= {
      loadDetail: () => (tab.objectKind === "matview"
        ? pg.matviewDetail(conn.id, db, tab.schema ?? "", tab.object ?? "")
        : pg.viewDetail(conn.id, db, tab.schema ?? "", tab.object ?? "")),
      // A prévia da view enxerga as tabelas do schema dela sem qualificar.
      execute: (sqlText, maxRows) => pg.execute(conn.id, db, sqlText, maxRows, tab.schema),
      apply: async (statements) => {
        await pg.apply(conn.id, db, statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      },
    });
  };

  const functionBackends = useRef<Record<string, FunctionBackend>>({});
  const functionBackendFor = (tab: DocTab, conn: ConnectionMock): FunctionBackend => {
    const db = tab.database ?? "";
    return (functionBackends.current[tab.id] ??= {
      loadDetail: (name, identity) => pg.functionDetail(conn.id, db, tab.schema ?? "", name, identity),
      // A execução enxerga as tabelas do schema da função sem qualificar.
      execute: (sqlText, maxRows) => pg.execute(conn.id, db, sqlText, maxRows, tab.schema),
      apply: async (statements) => {
        await pg.apply(conn.id, db, statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      },
    });
  };

  const roleBackends = useRef<Record<string, RoleBackend>>({});
  const roleBackendFor = (tab: DocTab, conn: ConnectionMock): RoleBackend => {
    const db = tab.database ?? conn.initialDatabase;
    return (roleBackends.current[tab.id] ??= {
      loadDetail: (name) => pg.roleDetail(conn.id, name),
      apply: async (statements) => {
        await pg.apply(conn.id, db, statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      },
    });
  };

  const typeBackends = useRef<Record<string, TypeBackend>>({});
  const typeBackendFor = (tab: DocTab, conn: ConnectionMock): TypeBackend => {
    const db = tab.database ?? "";
    return (typeBackends.current[tab.id] ??= {
      loadDetail: (name) => pg.typeDetail(conn.id, db, tab.schema ?? "", name),
      apply: async (statements) => {
        await pg.apply(conn.id, db, statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      },
      exec: async (sql) => { await pg.execute(conn.id, db, sql, 1); },
    });
  };

  const sequenceBackends = useRef<Record<string, SequenceBackend>>({});
  const sequenceBackendFor = (tab: DocTab, conn: ConnectionMock): SequenceBackend => {
    const db = tab.database ?? "";
    return (sequenceBackends.current[tab.id] ??= {
      loadDetail: (name) => pg.sequenceDetail(conn.id, db, tab.schema ?? "", name),
      apply: async (statements) => {
        await pg.apply(conn.id, db, statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      },
    });
  };

  const extensionBackends = useRef<Record<string, ExtensionBackend>>({});
  const extensionBackendFor = (tab: DocTab, conn: ConnectionMock): ExtensionBackend => {
    const db = tab.database ?? "";
    return (extensionBackends.current[tab.id] ??= {
      loadDetail: (name) => pg.extensionDetail(conn.id, db, name),
      loadAvailable: () => pg.availableExtensions(conn.id, db),
      apply: async (statements) => {
        await pg.apply(conn.id, db, statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      },
    });
  };

  const tablespaceBackends = useRef<Record<string, TablespaceBackend>>({});
  const tablespaceBackendFor = (tab: DocTab, conn: ConnectionMock): TablespaceBackend => {
    const db = tab.database ?? "";
    return (tablespaceBackends.current[tab.id] ??= {
      loadDetail: (name) => pg.tablespaceDetail(conn.id, db, name),
      // CREATE TABLESPACE não roda em bloco de transação: um comando por vez.
      apply: async (statements) => {
        for (const sql of statements) await pg.execute(conn.id, db, sql, 1);
      },
    });
  };

  const foreignServerBackends = useRef<Record<string, ForeignServerBackend>>({});
  const foreignServerBackendFor = (tab: DocTab, conn: ConnectionMock): ForeignServerBackend => {
    const db = tab.database ?? "";
    return (foreignServerBackends.current[tab.id] ??= {
      loadDetail: (name) => pg.foreignServerDetail(conn.id, db, name),
      loadWrappers: () => pg.foreignWrappers(conn.id, db),
      apply: async (statements) => {
        await pg.apply(conn.id, db, statements.map((sql) => ({ sql, params: [], expectOneRow: false })));
      },
    });
  };

  const foreignTableBackends = useRef<Record<string, ForeignTableBackend>>({});
  const foreignTableBackendFor = (tab: DocTab, conn: ConnectionMock): ForeignTableBackend => {
    const db = tab.database ?? "";
    const schema = tab.schema ?? "";
    return (foreignTableBackends.current[tab.id] ??= {
      loadDetail: async (name) => {
        const [detail, table] = await Promise.all([pg.foreignTableDetail(conn.id, db, schema, name), pg.tableDetail(conn.id, db, schema, name)]);
        return { detail, table };
      },
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
          autoRun={tab.autoRun}
          active={isActive}
          onStatus={statusFor(tab.id)}
          onDirty={(d) => setTabDirty(tab.id, d)}
          onTransaction={(open) => { if (open) txTabs.current.add(tab.id); else txTabs.current.delete(tab.id); }}
          onText={(text) => { queryTexts.current.set(tab.id, text); }}
          onContext={(c) => setTabs((ts) => (ts.some((x) => x.id === tab.id && (x.connectionId !== c.connectionId || x.database !== c.database || x.schema !== c.schema)) ? ts.map((x) => (x.id === tab.id ? { ...x, connectionId: c.connectionId, database: c.database, schema: c.schema } : x)) : ts))}
          registerActions={(a) => { if (isActive) queryActions.current = a; }}
          backend={queryBackend}
          connecting={connecting}
          onOpenConnection={connect}
          onLoadSchemas={(connId, dbName) => { void loadDatabase(connId, dbName).catch((e) => showError(dbName, e)); }}
          onExecuted={(e) => {
            // O histórico fica só no banco local; a preferência "history.enabled" (padrão ligada) o desliga.
            void local.getPref<boolean>("history.enabled").then((on) => {
              if (on === false) return;
              return local.historyAdd({
                connectionId: e.connectionId, connectionName: e.connectionName, database: e.database, schema: e.schema ?? null,
                sql: e.sql, durationMs: e.durationMs, rows: e.rows, error: e.error,
              });
            }).catch(() => undefined);
          }}
          onRefreshSchemas={(connId, dbName) => { void loadDatabase(connId, dbName, false, true).catch(() => undefined); }}
          onExport={(info) => setExportTarget({
            connId: info.connectionId, database: info.database, schema: info.schema ?? "public",
            tables: [{ name: t("exp.queryName"), view: false, query: true }], initial: t("exp.queryName"),
            query: { sql: info.sql, schema: info.schema, columns: info.columns, name: t("exp.queryName") },
          })}
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
          lookup={tab.objectKind === "view" || tab.objectKind === "matview" ? undefined : lookupSourceFor(tab, conn)}
          isView={tab.objectKind === "view" || tab.objectKind === "matview"}
          onDirty={(d) => setTabDirty(tab.id, d)}
          viewKey={isDesktop && conn?.live && tab.database ? viewKeyOf(conn.id, tab.database, tab.schema ?? "", tab.object ?? base.name) : undefined}
          onExport={conn?.live && tab.database ? (scope) => {
            const table = tab.object ?? base.name;
            const sc = conn.databases.find((d) => d.name === tab.database)?.schemas.find((x) => x.name === tab.schema);
            setExportTarget({
              connId: conn.id, database: tab.database ?? "", schema: tab.schema ?? "", tables: exportTablesOf(sc, table), initial: table,
              scope: { table, ...scope },
            });
          } : undefined}
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
          materialized={tab.objectKind === "matview"}
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
    if (tab.kind === "roleDesigner") {
      const conn = connections.find((c) => c.id === tab.connectionId);
      return (
        <RoleDesignerDoc
          roleName={tab.object}
          allRoles={conn?.roles.map((r) => r.name) ?? []}
          serverVersionNum={conn?.serverVersionNum ?? 0}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(name) => {
            const title = objectTitle(name, "tb.user", { connection: conn });
            setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
            if (conn?.live) void reloadRoles(conn.id).catch(() => undefined);
          }}
          backend={conn?.live ? roleBackendFor(tab, conn) : undefined}
        />
      );
    }
    if (tab.kind === "typeDesigner") {
      const conn = connections.find((c) => c.id === tab.connectionId);
      return (
        <TypeDesignerDoc
          schema={tab.schema ?? "public"}
          typeName={tab.object}
          owners={conn?.roles.map((r) => r.name) ?? []}
          serverVersionNum={conn?.serverVersionNum ?? 0}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(name) => {
            const title = objectTitle(name, "cat.type", resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections));
            setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
            if (conn?.live && tab.database && tab.schema) void loadSchema(conn.id, tab.database, tab.schema, true).catch(() => undefined);
          }}
          backend={conn?.live ? typeBackendFor(tab, conn) : undefined}
        />
      );
    }
    if (tab.kind === "sequenceDesigner") {
      const conn = connections.find((c) => c.id === tab.connectionId);
      return (
        <SequenceDesignerDoc
          schema={tab.schema ?? "public"}
          seqName={tab.object}
          owners={conn?.roles.map((r) => r.name) ?? []}
          serverVersionNum={conn?.serverVersionNum ?? 0}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(name) => {
            const title = objectTitle(name, "cat.sequence", resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections));
            setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
            if (conn?.live && tab.database && tab.schema) void loadSchema(conn.id, tab.database, tab.schema, true).catch(() => undefined);
          }}
          backend={conn?.live ? sequenceBackendFor(tab, conn) : undefined}
        />
      );
    }
    if (tab.kind === "console" && tab.connectionId) {
      return <ConsoleDoc sessionId={tab.connectionId} database={tab.database ?? "postgres"} active={isActive} onQuit={() => closeTab(tab.id)} />;
    }
    if (tab.kind === "find") {
      return (
        <FindDoc
          conns={connections.filter((c) => c.live).map((c) => ({ id: c.id, name: c.name, provider: c.provider, open: openConnections.has(c.id) }))}
          initial={{ connId: tab.connectionId, database: tab.database, schema: tab.schema }}
          onConnect={(id) => void connect(id)}
          onOpen={(connId, database, schemaName, object, what) => void openFindHit(connId, database, schemaName, object, what)}
          onOpenRows={(connId, database, schemaName, sql) => openTab({ kind: "query", title: t("tab.untitledQuery"), windowTitle: t("tab.untitledQuery"), icon: "query", connectionId: connId, database, schema: schemaName, sql, autoRun: true })}
          active={isActive}
          onStatus={statusFor(tab.id)}
        />
      );
    }
    if (tab.kind === "historyLog") {
      return <HistoryLogDoc active={isActive} onStatus={statusFor(tab.id)} />;
    }
    if (tab.kind === "monitor") {
      return (
        <ServerMonitorDoc
          conns={connections.filter((c) => c.live).map((c) => ({ id: c.id, name: c.name, provider: c.provider, open: openConnections.has(c.id) }))}
          initial={tab.connectionId}
          onConnect={(id) => void connect(id)}
          active={isActive}
          onStatus={statusFor(tab.id)}
        />
      );
    }
    if (tab.kind === "foreignTableDesigner") {
      const conn = connections.find((c) => c.id === tab.connectionId);
      const scope = resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections);
      return (
        <ForeignTableDesignerDoc
          tableName={tab.object}
          schema={tab.schema ?? "public"}
          servers={scope.schema?.objects.foreignServer?.map((s) => s.name) ?? []}
          roles={conn?.roles.map((r) => r.name) ?? []}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(name) => {
            const title = objectTitle(name, "ftb.tooltip", scope);
            setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
            if (conn?.live && tab.database && tab.schema) void loadSchema(conn.id, tab.database, tab.schema, true).catch(() => undefined);
          }}
          backend={conn?.live ? foreignTableBackendFor(tab, conn) : undefined}
        />
      );
    }
    if (tab.kind === "tablespaceDesigner") {
      const conn = connections.find((c) => c.id === tab.connectionId);
      return (
        <TablespaceDesignerDoc
          tablespaceName={tab.object}
          roles={conn?.roles.map((r) => r.name) ?? []}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(name) => {
            const title = objectTitle(name, "cat.tablespace", resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections));
            setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
            if (conn?.live && tab.database && tab.schema) reloadExtensions(conn.id, tab.database, tab.schema);
          }}
          backend={conn?.live ? tablespaceBackendFor(tab, conn) : undefined}
        />
      );
    }
    if (tab.kind === "foreignServerDesigner") {
      const conn = connections.find((c) => c.id === tab.connectionId);
      return (
        <ForeignServerDesignerDoc
          serverName={tab.object}
          roles={conn?.roles.map((r) => r.name) ?? []}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(name) => {
            const title = objectTitle(name, "cat.foreignServer", resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections));
            setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
            if (conn?.live && tab.database && tab.schema) reloadExtensions(conn.id, tab.database, tab.schema);
          }}
          backend={conn?.live ? foreignServerBackendFor(tab, conn) : undefined}
        />
      );
    }
    if (tab.kind === "extensionDesigner") {
      const conn = connections.find((c) => c.id === tab.connectionId);
      return (
        <ExtensionDesignerDoc
          schema={tab.schema ?? "public"}
          schemas={conn?.databases.find((d) => d.name === tab.database)?.schemas.map((s) => s.name) ?? []}
          extName={tab.object}
          serverVersionNum={conn?.serverVersionNum ?? 0}
          active={isActive}
          production={conn?.environment === "production"}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(name) => {
            const title = objectTitle(name, "cat.extension", resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections));
            setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: name, title, windowTitle: title } : x)));
            if (conn?.live && tab.database && tab.schema) reloadExtensions(conn.id, tab.database, tab.schema);
          }}
          backend={conn?.live ? extensionBackendFor(tab, conn) : undefined}
        />
      );
    }
    if (tab.kind === "functionDesigner") {
      const { conn } = lookupTable(tab);
      const completion: Record<string, string[]> = {};
      for (const tb of lookupTable(tab).tables) completion[tb.name] = (tb.columns ?? []).map((c) => c.name);
      return (
        <FunctionDesignerDoc
          schema={tab.schema ?? "public"}
          fnName={tab.fnName}
          identity={tab.fnIdentity}
          owners={conn?.roles.map((r) => r.name) ?? []}
          serverVersionNum={conn?.serverVersionNum ?? 0}
          active={isActive}
          production={conn?.environment === "production"}
          completionSchema={completion}
          onStatus={statusFor(tab.id)}
          onDirty={(dirty) => setTabDirty(tab.id, dirty)}
          onSaved={(fnName, identity) => {
            const title = objectTitle(fnName, "tb.function", resolveNode(`schema:${seg(tab.connectionId ?? "")}/${seg(tab.database ?? "")}/${seg(tab.schema ?? "")}`, connections));
            setTabs((ts) => ts.map((x) => (x.id === tab.id ? { ...x, object: `${fnName}(${identity})`, fnName, fnIdentity: identity, title, windowTitle: title } : x)));
            if (conn?.live && tab.database && tab.schema) void loadSchema(conn.id, tab.database, tab.schema, true).catch(() => undefined);
          }}
          backend={conn?.live ? functionBackendFor(tab, conn) : undefined}
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
          triggerFunctions={(conn?.databases.find((db) => db.name === tab.database)?.schemas ?? []).flatMap((sc) =>
            (sc.objects.function ?? []).filter((f) => f.returnType === "trigger" && f.fnName).map((f) => `${quoteIfNeeded(sc.name)}.${quoteIfNeeded(f.fnName ?? "")}`),
          )}
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
      <OptionsContext.Provider value={appOptions}>
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
                  filterOptions={filterOptions}
                  onFilterOptions={setFilterOptions}
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
            <TabBar tabs={tabsForBar} activeId={activeTab} onActivate={setActiveTab} onClose={closeTab} onReorder={reorderTab} />
            <div className="tab-content">
              <div className={`doc-host${activeTab === "objects" ? "" : " is-hidden"}`}>
                <ObjectsView
                  category={category}
                  ctx={ctx}
                  connectionOpen={connectionOpen}
                  viewMode={layout.viewMode}
                  erOptions={{ showGrid: layout.erShowGrid ?? true, snapToGrid: layout.erSnapGrid ?? true, showFkNames: layout.erShowFkNames ?? true }}
                  sort={{ key: layout.objSortKey ?? "name", dir: layout.objSortDir ?? 1 }}
                  onSort={(s) => patchLayout({ objSortKey: s.key, objSortDir: s.dir })}
                  hiddenColumns={layout.hiddenObjCols?.[category] ?? []}
                  lang={layout.lang}
                  selected={selectedObjects}
                  onSelect={setSelectedObjects}
                  onAction={(kind, names) => openObjects(kind, names)}
                  onContextMenu={(name, x, y) => {
                    // Mesmo menu da árvore: monta o nó equivalente (objeto ou pasta da categoria).
                    // Papéis pertencem ao servidor, não a um schema: o menu só precisa da conexão.
                    if (category === "user" && ctx.connection) {
                      const userPath = `${seg(ctx.connection.id)}/${seg(ctx.database?.name ?? "")}/${seg(ctx.schema?.name ?? "")}`;
                      showContextMenu(name
                        ? { id: `obj:${userPath}/user/${seg(name)}`, kind: "object", label: name, depth: 4, expandable: false, category }
                        : { id: `folder:${userPath}/user`, kind: "folder", label: "", depth: 3, expandable: false, category }, x, y);
                      return;
                    }
                    if (!ctx.connection || !ctx.database || !ctx.schema) return;
                    const scPath = `${seg(ctx.connection.id)}/${seg(ctx.database.name)}/${seg(ctx.schema.name)}`;
                    const node: TreeNode = name
                      ? { id: `obj:${scPath}/${category}/${seg(name)}`, kind: "object", label: name, depth: 4, expandable: false, category }
                      : { id: `folder:${scPath}/${category}`, kind: "folder", label: "", depth: 3, expandable: false, category };
                    showContextMenu(node, x, y);
                  }}
                />
              </div>
              {tabs.filter((x) => x.kind !== "objects").map((tab) => (
                <div key={tab.id} className={`doc-host${tab.id === activeTab ? "" : " is-hidden"}${fullDoc === tab.id ? " is-fullscreen" : ""}`}>
                  <DocFullscreenContext.Provider value={{ on: fullDoc === tab.id, toggle: () => setFullDoc((f) => (f === tab.id ? null : tab.id)) }}>{renderDoc(tab)}</DocFullscreenContext.Provider>
                </div>
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

      {exportTarget && <ExportWizard target={exportTarget} onClose={() => setExportTarget(null)} />}
      {importTarget && (
        <ImportWizard
          target={importTarget}
          onClose={() => setImportTarget(null)}
          onDone={() => void loadSchema(importTarget.connId, importTarget.database, importTarget.schema, true).catch(() => undefined)}
        />
      )}

      {dialog?.kind === "options" && (
        <OptionsDialog
          options={appOptions}
          appearance={{ theme: layout.theme, lang: layout.lang, showCaptions: layout.showCaptions }}
          onApply={(opts, look) => { setAppOptions(opts); patchLayout(look); }}
          onClose={() => setDialog(null)}
        />
      )}

      {dialog?.kind === "dump" && <DumpDialog target={dialog} onClose={() => setDialog(null)} />}
      {dialog?.kind === "backup" && <BackupDialog target={dialog} onClose={() => setDialog(null)} onDone={() => refreshBackups(dialog.connId, dialog.database)} />}
      {dialog?.kind === "restore" && <RestoreDialog target={dialog} onClose={() => setDialog(null)} />}
      {dialog?.kind === "deleteBackup" && (
        <MessageDialog
          title={t("act.deleteBackup")}
          message={dialog.names.length === 1 ? t("msg.confirmDeleteBackup", { name: dialog.names[0] }) : t("msg.confirmDeleteBackups", { n: dialog.names.length })}
          danger
          onOk={() => {
            const d = dialog;
            setDialog(null);
            void Promise.all(d.files.map((f) => pg.backupDelete(f.path, f.metaPath)))
              .catch((e) => showError(t("act.deleteBackup"), e))
              .finally(() => refreshBackups(d.connId, d.database));
          }}
          onCancel={() => setDialog(null)}
        />
      )}

      {dialog?.kind === "chooseColumns" && (() => {
        // Exibir → Escolher colunas: as colunas da lista de objetos da categoria atual; o nome (a primeira) não some.
        const cols = objectColumns(category, layout.lang, ctx.connection?.provider);
        const hiddenKeys = layout.hiddenObjCols?.[category] ?? [];
        const setHidden = (keys: string[]) => patchLayout({ hiddenObjCols: { ...(layout.hiddenObjCols ?? {}), [category]: keys } });
        return (
          <ColumnsDialog
            columns={cols.map((c) => ({ name: t(c.label), type: "" }))}
            hidden={new Set(cols.filter((c) => hiddenKeys.includes(c.key)).map((c) => t(c.label)))}
            onToggle={(label) => {
              const c = cols.find((x) => t(x.label) === label);
              if (!c || c.key === cols[0].key) return;
              setHidden(hiddenKeys.includes(c.key) ? hiddenKeys.filter((k) => k !== c.key) : [...hiddenKeys, c.key]);
            }}
            onShowAll={() => setHidden([])}
            onHideAll={() => setHidden(cols.slice(1).map((c) => c.key))}
            onClose={() => setDialog(null)}
          />
        );
      })()}

      {dialog?.kind === "editDatabase" && (() => {
        const conn = connections.find((c) => c.id === dialog.connId);
        return (
          <EditDatabaseDialog
            detail={dialog.detail}
            owners={conn?.roles.map((r) => r.name) ?? []}
            canRename={dialog.detail.name !== (conn?.initialDatabase || "postgres")}
            supportsFlags={(conn?.serverVersionNum ?? 0) >= 90500}
            onApply={(state) => applyEditDatabase(dialog, state)}
            onClose={() => setDialog(null)}
          />
        );
      })()}

      {dialog?.kind === "editSchema" && (
        <EditSchemaDialog
          detail={dialog.detail}
          owners={connections.find((c) => c.id === dialog.connId)?.roles.map((r) => r.name) ?? []}
          onApply={(state) => applyEditSchema(dialog, state)}
          onClose={() => setDialog(null)}
        />
      )}


      {dialog?.kind === "connection" && (
        <ConnectionDialog
          editing={dialog.editing}
          knownPassword={dialog.editing ? sessionPasswords.get(dialog.editing.id) : undefined}
          recentProviders={recentProviders}
          onSave={(form, version) => saveConnection(form, version, dialog.editing, dialog.group)}
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
      {dialog?.kind === "closeDbDirty" && (
        <MessageDialog
          title={t("msg.unsavedTitle")}
          message={t("msg.unsavedTables")}
          okLabel={t("msg.discard")}
          onOk={() => { dialog.run(); setDialog(null); }}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "closeDirty" && (
        <MessageDialog
          title={t("msg.unsavedTitle")}
          message={t(tabs.find((x) => x.id === dialog.tabId)?.kind === "data" ? "msg.unsavedData" : "msg.unsavedQuery")}
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
          message={[dialog.message ?? (dialog.danger ? undefined : t("des.reviewMessage")), dialog.danger ? t("msg.ddlDanger") : undefined, dialog.note].filter(Boolean).join("\n\n")}
          danger={dialog.danger}
          confirmText={dialog.confirmText}
          okLabel={t("msg.execute")}
          busy={dialog.busy}
          error={dialog.error}
          onOk={() => void runDdl(dialog)}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "tasks" && (
        <TasksDialog
          tasks={tasks}
          conns={connections.filter((c) => c.live).map((c) => ({ id: c.id, name: c.name }))}
          running={runningTasks}
          onChange={setTasks}
          onRun={(id) => void executeTask(id)}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "info" && (
        <MessageDialog title={dialog.title} message={dialog.message} onOk={() => setDialog(null)} onCancel={() => setDialog(null)} />
      )}
      {dialog?.kind === "newDatabase" && (
        <MessageDialog
          title={t("cmd.newDatabase").replace(/\.\.\.$/, "")}
          prompt={{ label: t("msg.newDatabaseName"), initial: "" }}
          okLabel={t("msg.create")}
          busy={dialog.busy}
          error={dialog.error}
          onOk={(name) => name && void createDatabase(dialog, name)}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "execFile" && (() => {
        const conn = connections.find((c) => c.id === dialog.connId);
        return (
          <MessageDialog
            title={t("cmd.executeSqlFile").replace(/\.\.\.$/, "")}
            message={t("msg.execFileMessage", { file: dialog.fileName, kb: Math.max(1, Math.round(dialog.size / 1024)), db: dialog.database, conn: conn?.name ?? "" })}
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
        <MessageDialog
          title={t("cmd.about")}
          message={(
            <div className="about-box">
              <img className="about-logo" src="/logo.png" alt="Atlas DB Manager" width={72} height={72} draggable={false} />
              <div className="about-text">
                <div className="about-name">Atlas DB Manager</div>
                <div className="muted">{t("about.tagline")}</div>
                <dl className="about-list">
                  <div><dt>{t("about.version")}</dt><dd>{APP_VERSION} <span className="muted">({t("about.stage")})</span></dd></div>
                  <div><dt>{t("about.author")}</dt><dd>{APP_AUTHOR}</dd></div>
                  <div><dt>{t("about.license")}</dt><dd>MIT</dd></div>
                  <div><dt>GitHub</dt><dd><a href={APP_REPO} className="about-link" onClick={(e) => { e.preventDefault(); void local.openExternal(APP_REPO).catch((err) => showError(t("cmd.about"), err)); }}>{APP_REPO.replace("https://", "")}</a></dd></div>
                  <div><dt>{t("about.built")}</dt><dd>Rust, Tauri, React</dd></div>
                </dl>
              </div>
            </div>
          )}
          onOk={() => setDialog(null)}
          onCancel={() => setDialog(null)}
        />
      )}
      </OptionsContext.Provider>
    </I18nContext.Provider>
  );
}
