/*
 * Ponte com o núcleo Rust (comandos IPC do Tauri).
 * Dentro do app desktop tudo vai ao banco real; no navegador (`npm run dev` puro)
 * a interface continua no modo de demonstração com dados fictícios.
 */
import { routeCommand } from "./providerRoute";
import { TAB_EVENT, type TabEvent } from "./tabWindow";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { desktopDir, homeDir, sep } from "@tauri-apps/api/path";
import { open, save } from "@tauri-apps/plugin-dialog";

export const isDesktop = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type SslMode = "disable" | "prefer" | "require" | "verify-ca" | "verify-full";

export interface ConnectionParams {
  host: string;
  port: number;
  database: string;
  user: string;
  password: string;
  sslMode: SslMode;
  connectTimeoutSecs?: number;
  /** Banco da conexão; ausente = PostgreSQL. */
  provider?: string;
  /** Túnel SSH até o servidor do banco; com `keyPath` a autenticação é por chave (`password` vira a frase da chave). */
  ssh?: { host: string; port: number; user: string; password: string; keyPath: string };
}

export interface ServerInfo {
  version: string;
  versionNum: number;
  currentDatabase: string;
  currentUser: string;
  tls: boolean | null;
  latencyMs: number;
}

export type ErrorKind =
  | "authenticationFailed" | "hostUnreachable" | "certificateRejected" | "permissionDenied" | "connectionLost"
  | "databaseNotFound" | "sqlSyntaxError" | "constraintViolation" | "timeout" | "canceled" | "unsupportedFeature"
  | "invalidMutationIdentity" | "concurrencyConflict" | "invalidInput" | "sessionNotFound" | "server" | "internal";

export interface AtlasError {
  kind: ErrorKind;
  code: string | null;
  message: string;
  detail: string | null;
  position: number | null;
}

/** Normaliza qualquer falha (erro tipado do Rust ou exceção JS) em AtlasError. */
export function toAtlasError(e: unknown): AtlasError {
  if (e && typeof e === "object" && "kind" in e && "message" in e) return e as AtlasError;
  return { kind: "internal", code: null, message: e instanceof Error ? e.message : String(e), detail: null, position: null };
}

export function errorText(e: AtlasError): string {
  return [e.code ? `[${e.code}] ${e.message}` : e.message, e.detail].filter(Boolean).join("\n");
}

export interface ObjectInfo {
  name: string;
  rows: number | null;
  comment: string | null;
  owner: string | null;
  args: string | null;
  returnType: string | null;
  language: string | null;
  dataType: string | null;
  procedure?: boolean;
}

export interface SchemaObjects {
  tables: ObjectInfo[];
  views: ObjectInfo[];
  matviews: ObjectInfo[];
  functions: ObjectInfo[];
  types: ObjectInfo[];
  sequences: ObjectInfo[];
  /** Extensões do banco inteiro (a mesma lista em qualquer schema). */
  extensions: ExtensionInfo[];
  /** Servidores estrangeiros do banco inteiro (a mesma lista em qualquer schema). */
  foreignServers: ForeignServerInfo[];
  /** Tablespaces do cluster (a mesma lista em qualquer schema). */
  tablespaces: TablespaceInfo[];
}

/** Diagrama ER de um schema: tabelas com colunas (PK/FK marcadas) e chaves estrangeiras. */
export interface DiagramColumn {
  name: string;
  dataType: string;
  length: number;
  decimal: number;
  isPk: boolean;
  isFk: boolean;
}
export interface DiagramTable { name: string; columns: DiagramColumn[] }
export interface DiagramRelation {
  name: string;
  table: string;
  columns: string[];
  refSchema: string;
  refTable: string;
  refColumns: string[];
  /** Todas as colunas da chave são NOT NULL: a referência é obrigatória; senão, opcional. */
  required: boolean;
}
export interface SchemaDiagram { tables: DiagramTable[]; relations: DiagramRelation[] }

/** Servidor estrangeiro (`CREATE SERVER`). */
export interface TableProfile {
  owner: string;
  kind: string;
  persistence: "permanent" | "unlogged" | "temporary";
  tablespace: string;
  estimatedRows: number;
  tableBytes: number;
  indexBytes: number;
  totalBytes: number;
  columns: number;
  indexes: number;
  primaryKeys: number;
  foreignKeys: number;
  uniques: number;
  checks: number;
  triggers: number;
  rowSecurity: boolean;
  replicaIdentity: string;
  liveTuples: number | null;
  deadTuples: number | null;
  seqScans: number | null;
  indexScans: number | null;
  lastVacuum: string | null;
  lastAutovacuum: string | null;
  lastAnalyze: string | null;
  lastAutoanalyze: string | null;
  comment: string | null;
}

export interface TablespaceInfo { name: string; owner: string; location: string; comment: string | null }

export interface TablespaceDetail {
  name: string;
  owner: string;
  location: string;
  options: OptionPair[];
  comment: string | null;
}

export interface ForeignServerInfo { name: string; wrapper: string; owner: string; comment: string | null }
export interface OptionPair { key: string; value: string }
export interface UserMappingInfo { user: string; options: OptionPair[] }
export interface ForeignServerDetail {
  name: string;
  wrapper: string;
  serverType: string | null;
  version: string | null;
  owner: string;
  options: OptionPair[];
  mappings: UserMappingInfo[];
  comment: string | null;
}

export interface ForeignTableDetail {
  name: string;
  server: string;
  options: OptionPair[];
  owner: string;
  comment: string | null;
}

/** Extensão instalada, com o schema onde foi instalada. */
export interface ExtensionInfo {
  name: string;
  version: string;
  schema: string;
  owner: string;
  comment: string | null;
}

/** Extensão que o servidor oferece e ainda não está instalada neste banco. */
export interface AvailableExtension {
  name: string;
  defaultVersion: string;
  comment: string | null;
  /** Schema fixo no arquivo de controle da extensão; `null` = escolha livre. */
  fixedSchema: string | null;
}

export interface ExtensionDetail {
  name: string;
  version: string;
  schema: string;
  owner: string;
  /** Só uma extensão relocável muda de schema. */
  relocatable: boolean;
  comment: string | null;
  /** Versões para as quais há caminho de atualização a partir da instalada. */
  updateVersions: string[];
}

export interface OrderBy { column: string; descending?: boolean }
export interface TableFilter { expr: string }

export interface ColumnInfo {
  name: string;
  dataType: string;
  formattedType: string;
  length: number;
  decimal: number;
  notNull: boolean;
  key: number | null;
  default: string | null;
  comment: string | null;
  collation: string | null;
  /** "a" = GENERATED ALWAYS AS IDENTITY, "d" = GENERATED BY DEFAULT, null = nenhum. */
  identity?: string | null;
  /** "s" = GENERATED ... AS (expr) STORED, null = nenhum. */
  generated?: string | null;
}

export interface IndexInfo { name: string; fields: string; method: string; unique: boolean; primary: boolean; definition: string; comment: string | null }
export interface ConstraintInfo {
  name: string; kind: "f" | "u" | "c" | "x"; fields: string; refSchema: string | null; refTable: string | null;
  refFields: string | null; onDelete: string | null; onUpdate: string | null; definition: string; comment: string | null;
}

export interface TableDetail {
  columns: ColumnInfo[];
  indexes: IndexInfo[];
  constraints: ConstraintInfo[];
  triggers: TriggerInfo[];
  owner: string | null;
  comment: string | null;
}

export interface RoleInfo { name: string; canLogin: boolean; superuser: boolean }

export interface ViewDetail {
  /** Só o SELECT, sem `;` final. */
  definition: string;
  owner: string | null;
  comment: string | null;
  checkOption: "local" | "cascaded" | null;
  securityBarrier: boolean;
  rules: { name: string; definition: string }[];
  /** View materializada já preenchida com dados. Views comuns: sempre true. */
  populated: boolean;
}

/* ----- Importação ----- */

export type ImportFormat =
  | { kind: "delimited"; delimiter: string; qualifier: string; encoding: "utf8" | "windows1252" }
  | { kind: "json" }
  | { kind: "xml" }
  | { kind: "excel" }
  | { kind: "dbf" };

export type ImportMode = "append" | "update" | "appendUpdate" | "appendNoUpdate" | "delete" | "copy";

export interface SourceSpec {
  path: string;
  format: ImportFormat;
  /** Tabela do arquivo (planilha do Excel; nos demais formatos só há uma). */
  table: string;
  options: { fieldNameRow: number; firstDataRow: number; lastDataRow: number };
}

export interface ConvertOptions {
  dateOrder: "DMY" | "MDY" | "YMD";
  dateTimeOrder: "DateTime" | "TimeDate";
  dateDelimiter: string;
  timeDelimiter: string;
  decimalSymbol: string;
  emptyAsNull?: boolean;
}

export interface ImportPreview {
  columns: string[];
  rows: (string | null)[][];
  /** Tipo PostgreSQL sugerido para cada campo (para criar uma tabela nova). */
  types: string[];
}

export interface ImportJobRequest {
  source: SourceSpec;
  target: string;
  create?: { name: string; type: string }[];
  mappings: { source: string; target: string }[];
  keys: string[];
  mode: ImportMode;
}

export interface ImportRequest {
  importId: string;
  sessionId: string;
  database: string;
  schema: string;
  jobs: ImportJobRequest[];
  advanced: { extendedInsert: boolean; maxStatementKb: number; emptyAsNull: boolean; continueOnError: boolean };
  convert: ConvertOptions;
}

export interface ImportCounts { table: string; processed: number; added: number; updated: number; deleted: number; errors: number; rolledBack: boolean }
export interface ImportSummary { jobs: ImportCounts[]; elapsedMs: number }

/** Progresso da importação (a cada ~200 ms): contagens da tabela `job` em andamento. */
export async function onImportProgress(importId: string, cb: (job: number, counts: ImportCounts) => void): Promise<() => void> {
  return listen<ImportCounts & { importId: string; job: number }>("import-progress", (e) => {
    if (e.payload.importId === importId) cb(e.payload.job, e.payload);
  });
}

/** Linhas do registro da importação (`[IMP] ...`, `[ERR] ...`). */
export async function onImportLog(importId: string, cb: (line: string) => void): Promise<() => void> {
  return listen<{ importId: string; line: string }>("import-log", (e) => {
    if (e.payload.importId === importId) cb(e.payload.line);
  });
}

/** Janela nativa "Abrir" para escolher arquivos de origem. */
export async function chooseImportFiles(extensions: string[]): Promise<string[]> {
  // Só em desenvolvimento: testes automatizados informam os arquivos sem abrir a janela nativa.
  const injected = import.meta.env.DEV ? (window as { __atlasImportFiles?: string[] }).__atlasImportFiles : undefined;
  if (injected) return injected;
  const picked = await open({ multiple: true, directory: false, filters: [{ name: extensions.map((x) => `*.${x}`).join(", "), extensions }, { name: "*.*", extensions: ["*"] }] });
  return Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
}

/* ----- Query Builder (janela separada) ----- */

export interface QueryBuilderOpen {
  /** Identifica a aba de consulta; uma aba abre no máximo uma janela. */
  id: string;
  connId: string;
  database: string;
  defaultSchema: string;
  /** SELECT do editor, para o desenho começar dele. */
  initialSql: string;
  title: string;
}

export interface QueryBuilderResult { id: string; sql: string; run: boolean; replaced: boolean }

/** Abre (ou traz para a frente) a janela do Query Builder. */
export const openQueryBuilder = (req: QueryBuilderOpen) => call<void>("open_query_builder", { req });

/** Fecha a janela do Query Builder da aba (se estiver aberta). */
export const closeQueryBuilder = (id: string) => call<void>("close_query_builder", { id });

/** A janela do Query Builder devolveu um SELECT (Build / Build and Run). */
/** A janela principal ouve os pedidos das ferramentas para abrir um script numa aba de consulta. */
/** Uma sessão foi aberta (por esta ou por outra janela, como o Data Transfer); a janela principal marca a conexão como aberta. */
export async function onSessionOpened(cb: (e: { id: string; version: string; versionNum: number }) => void): Promise<() => void> {
  return listen<{ id: string; version: string; versionNum: number }>("session-opened", (e) => cb(e.payload));
}

export async function onOpenScript(cb: (r: OpenScriptRequest) => void): Promise<() => void> {
  return listen<OpenScriptRequest>("tool-open-query", (e) => cb(e.payload));
}

export async function onQueryBuilderBuild(id: string, cb: (r: QueryBuilderResult) => void): Promise<() => void> {
  return listen<QueryBuilderResult>("qb-build", (e) => {
    if (e.payload.id === id) cb(e.payload);
  });
}

/* ----- Data Transfer (janela separada) ----- */

export interface TransferFieldMap { source: string; target: string }
export interface TransferTableOverride {
  table: string;
  targetName?: string;
  /** Vazio/ausente = todos os campos com o mesmo nome. */
  fields?: TransferFieldMap[];
  /** Linhas por lote, cada lote numa transação; ausente = a tabela inteira numa transação só. */
  rowsPerBatch?: number;
  /** Filtros (`WHERE`): cada um é transferido à parte. */
  recordsets?: string[];
}
export interface TransferSpec {
  sourceSchema: string;
  targetSchema: string;
  tables: string[];
  views: string[];
  /** `nome(argumentos)`. */
  functions: string[];
  sequences: string[];
  types: string[];
  overrides: TransferTableOverride[];
  dropBeforeCreate: boolean;
  continueOnError: boolean;
  structure: boolean;
  data: boolean;
}
export interface TransferRequest {
  transferId: string;
  sourceConn: string;
  sourceDatabase: string;
  targetConn: string;
  targetDatabase: string;
  spec: TransferSpec;
}
export interface TransferCounts { objects: number; processed: number; transferred: number; errors: number; elapsedMs: number }

/** Conexão como a janela do Data Transfer a enxerga (a lista vem da janela principal). */
export interface DataTransferConn {
  id: string;
  name: string;
  provider: string;
  host: string;
  port: number;
  user: string;
  version: string;
  versionNum: number;
  open: boolean;
  /** Para a janela abrir a conexão sozinha (a senha vem do cofre pelo `id`). */
  initialDatabase: string;
  sslMode?: SslMode;
  hasPassword?: boolean;
}
export interface DataTransferParams {
  connections: DataTransferConn[];
  source?: { connId: string; database: string; schema: string };
}

/** Abre (ou traz para a frente) a janela do Data Transfer. */
export const openDataTransfer = (title: string, params: DataTransferParams) => call<void>("open_data_transfer", { req: { title, params: { ...params, title } } });

export async function onTransferProgress(id: string, cb: (c: TransferCounts) => void): Promise<() => void> {
  return listen<TransferCounts & { transferId: string }>("transfer-progress", (e) => {
    if (e.payload.transferId === id) cb(e.payload);
  });
}

export async function onTransferLog(id: string, cb: (line: string) => void): Promise<() => void> {
  return listen<{ transferId: string; line: string }>("transfer-log", (e) => {
    if (e.payload.transferId === id) cb(e.payload.line);
  });
}

/* ----- Data Synchronization (janela separada) ----- */

export interface SyncTableSpec {
  source: string;
  target: string;
  /** Ausente = a chave primária da origem. */
  key?: TransferFieldMap[];
  /** Ausente = os campos de mesmo nome. */
  fields?: TransferFieldMap[];
}
export interface SyncSpec { sourceSchema: string; targetSchema: string; tables: SyncTableSpec[] }
export interface SyncCompareRequest {
  compareId: string;
  sourceConn: string;
  sourceDatabase: string;
  targetConn: string;
  targetDatabase: string;
  spec: SyncSpec;
}
export interface SyncTableSummary {
  source: string;
  target: string;
  insert: number;
  update: number;
  delete: number;
  same: number;
  message: string | null;
  sourceFields: string[];
  targetFields: string[];
  keyPositions: number[];
  rows: number;
}
export interface SyncSummary { tables: SyncTableSummary[] }
export interface SyncRow {
  /** Número da linha na lista da tabela (o que se marca e desmarca). */
  id: number;
  kind: "insert" | "update" | "delete";
  source: (string | null)[] | null;
  target: (string | null)[] | null;
  changed: boolean[];
}
export interface SyncRowsPage { total: number; rows: SyncRow[] }
export interface SyncSelection { table: string; insert: boolean; update: boolean; delete: boolean; excluded: number[] }
export interface SyncScript { statements: number; text: string; truncated: boolean }
export interface SyncDeployRequest {
  deployId: string;
  compareId: string;
  targetConn: string;
  targetDatabase: string;
  selection: SyncSelection[];
  transaction: boolean;
  continueOnError: boolean;
}
export interface SyncDeployCounts { total: number; done: number; errors: number; rolledBack: boolean; elapsedMs: number }

/** Abre (ou traz para a frente) a janela do Data Synchronization. */
export const openDataSync = (title: string, params: DataTransferParams) => call<void>("open_data_sync", { req: { title, params: { ...params, title } } });

/** Progresso da comparação: o que está sendo lido e a fração concluída. */
export async function onSyncProgress(id: string, cb: (text: string, fraction: number) => void): Promise<() => void> {
  return listen<{ id: string; text: string; fraction: number }>("datasync-progress", (e) => {
    if (e.payload.id === id) cb(e.payload.text, e.payload.fraction);
  });
}
export async function onSyncLog(id: string, cb: (line: string) => void): Promise<() => void> {
  return listen<{ id: string; line: string }>("datasync-log", (e) => {
    if (e.payload.id === id) cb(e.payload.line);
  });
}
export async function onSyncDeployProgress(id: string, cb: (done: number, total: number) => void): Promise<() => void> {
  return listen<{ id: string; done: number; total: number }>("datasync-deploy-progress", (e) => {
    if (e.payload.id === id) cb(e.payload.done, e.payload.total);
  });
}

/* ----- Console (aba) ----- */

export interface ConsoleOptions { timing: boolean; expanded: boolean; vars: Record<string, string> }
export interface ConsoleOutput { text: string; switchDb: string | null; quit: boolean; watch: number | null; options: ConsoleOptions }
export interface ConsoleCompletion { start: number; candidates: string[]; append: string }

/* ----- History Log (aba) ----- */

export interface SqlLogEntry {
  seq: number;
  atMs: number;
  server: string;
  pid: number;
  text: string;
  secs: number | null;
  error: boolean;
  failed: boolean;
}
export interface SqlLogPage { entries: SqlLogEntry[]; last: number; paused: boolean }

/* ----- Server Monitor (aba) ----- */

export interface MonitorProcess {
  id: number;
  user: string | null;
  host: string | null;
  port: number | null;
  db: string | null;
  command: string | null;
  state: string | null;
  time: string | null;
  info: string | null;
}
export interface MonitorSnapshot { processes: MonitorProcess[]; variables: [string, string][]; status: [string, string][] }

/* ----- Data Dictionary (janela separada) ----- */

export interface DictFont { family: string; size: number }
export type DictTemplate = "orange" | "red" | "blackWhite" | "blue" | "green";
export interface DictSettings {
  template: DictTemplate;
  includeCover: boolean;
  coverHeader: string;
  coverTitle: string;
  coverSubtitle: string;
  coverRemarks: string;
  coverBackground: "default" | "custom" | "none";
  includeToc: boolean;
  breadcrumbs: boolean;
  breadcrumbFont: DictFont;
  headingFont: DictFont;
  subheadingFont: DictFont;
  leftFooter: boolean;
  leftFooterText: string;
  pageNumber: boolean;
  footerFont: DictFont;
  includeIntro: boolean;
  introText: string;
  bodyFont: string;
  paper: string;
  landscape: boolean;
  /** Centímetros: esquerda, direita, topo e base. */
  margins: [number, number, number, number];
  now: [number, number, number, number, number, number];
}
export interface DictSchemaSel { name: string; info: boolean; tables: string[] | null; views: string[] | null; functions: string[] | null }
export interface DictDbSel { name: string; info: boolean; schemas: DictSchemaSel[] }
export interface DictServerSel { connId: string; name: string; product: string; info: [string, string][] | null; databases: DictDbSel[] }
export interface DictGenerateRequest {
  runId: string;
  servers: DictServerSel[];
  settings: DictSettings;
  path: string;
  appendTimestamp: boolean;
  overwrite: boolean;
  continueOnError: boolean;
}
export interface DictCounts { tables: number; views: number; functions: number; errors: number; fraction: number }
export interface DictResult extends DictCounts { path: string | null; elapsedMs: number }
export interface DictPreviewPage { svg: string; width: number; height: number }

/** Abre (ou traz para a frente) a janela do Data Dictionary. */
export const openDataDictionary = (title: string, params: DataTransferParams) => call<void>("open_data_dictionary", { req: { title, params: { ...params, title } } });

export async function onDictLog(id: string, cb: (line: string) => void): Promise<() => void> {
  return listen<{ id: string; line: string }>("dict-log", (e) => {
    if (e.payload.id === id) cb(e.payload.line);
  });
}
export async function onDictProgress(id: string, cb: (c: DictCounts) => void): Promise<() => void> {
  return listen<DictCounts & { id: string }>("dict-progress", (e) => {
    if (e.payload.id === id) cb(e.payload);
  });
}

/* ----- Data Generation (janela separada) ----- */

export type TextStyle = "sentence" | "words" | "letters";
export type Generator =
  | { kind: "sequence"; start: number | null; step: number }
  | { kind: "integer"; min: number; max: number }
  | { kind: "decimal"; min: number; max: number; scale: number }
  | { kind: "text"; minLen: number; maxLen: number; style: TextStyle }
  | { kind: "dateTime"; from: string; to: string }
  | { kind: "date"; from: string; to: string }
  | { kind: "boolean"; truePercent: number }
  | { kind: "uuid" }
  | { kind: "list"; values: string[] }
  | { kind: "fixed"; value: string }
  | { kind: "null" }
  | { kind: "reference"; table: string; column: string };
export type GeneratorKind = Generator["kind"];
export interface GenColumnMeta { name: string; dataType: string; notNull: boolean; isPk: boolean; generated: boolean; charLen: number | null; suggested: Generator }
export interface GenTableMeta { name: string; columns: GenColumnMeta[]; dependsOn: string[] }
export interface GenColumnSpec { name: string; generator: Generator; nullPercent: number }
export interface GenTableSpec { name: string; rows: number; columns: GenColumnSpec[] }
export interface GenOptions { continueOnError: boolean; emptyFirst: boolean; transaction: boolean; multiInsert: boolean }
export interface GenSpec { schema: string; tables: GenTableSpec[]; options: GenOptions }
export interface GenPreview { columns: string[]; rows: (string | null)[][] }
export interface GenStats { objects: number; generated: number; inserted: number; errors: number }
export interface GenResult extends GenStats { elapsedMs: number }

/** Abre a janela de uma aba de tabela ou de consulta destacada (uma por aba). */
export const openTabWindow = (id: string, title: string, params: unknown) => call<void>("open_tab_window", { id, req: { title, params } });

/** Avisos das janelas de abas destacadas (anexar de volta, fechar, última versão) para a janela principal. */
export async function onTabWindow(cb: (e: TabEvent) => void): Promise<() => void> {
  return listen<TabEvent>(TAB_EVENT, (e) => cb(e.payload));
}

/** Envia um aviso entre a janela principal e as janelas de abas destacadas. */
export const emitTabEvent = (e: TabEvent) => emit(TAB_EVENT, e);

/** Abre (ou traz para a frente) a janela do Data Generation. */
export const openDataGen = (title: string, params: DataTransferParams) => call<void>("open_data_gen", { req: { title, params: { ...params, title } } });

export async function onGenLog(id: string, cb: (line: string) => void): Promise<() => void> {
  return listen<{ id: string; line: string }>("datagen-log", (e) => {
    if (e.payload.id === id) cb(e.payload.line);
  });
}
export async function onGenProgress(id: string, cb: (s: GenStats) => void): Promise<() => void> {
  return listen<GenStats & { id: string }>("datagen-progress", (e) => {
    if (e.payload.id === id) cb(e.payload);
  });
}

/* ----- Structure Synchronization (janela separada) ----- */

export type StructOp = "add" | "delete" | "modify" | "same";
export type StructKind = "table" | "ftable" | "view" | "matview" | "sequence" | "function" | "type";
export interface StructPart { kind: string; name: string; op: StructOp }
export interface StructObject { kind: StructKind; name: string; op: StructOp; parts: StructPart[]; sourceDdl: string; targetDdl: string }
/** `parts` ausente (null) = todas as partes do objeto. */
export interface StructSelection { kind: string; name: string; parts: number[] | null }
export interface StructCompareRequest {
  compareId: string;
  sourceConn: string;
  sourceDatabase: string;
  targetConn: string;
  targetDatabase: string;
  spec: { sourceSchema: string; targetSchema: string; compareOwners?: boolean; comparePrivileges?: boolean };
}
export interface StructDeployRequest {
  deployId: string;
  targetConn: string;
  targetDatabase: string;
  statements: string[];
  continueOnError: boolean;
  includeQuery: boolean;
}

/** Abre (ou traz para a frente) a janela do Structure Synchronization. */
export const openStructSync = (title: string, params: DataTransferParams) => call<void>("open_struct_sync", { req: { title, params: { ...params, title } } });

/** O que a janela de ferramenta pede à principal: abrir um script numa aba de consulta. */
export interface OpenScriptRequest { connId: string; database: string; schema: string; sql: string }

/** O que o dump SQL cobre. */
export type DumpScope =
  | { kind: "database" }
  | { kind: "schema"; schema: string }
  | { kind: "tables"; schema: string; tables: string[] }
  | { kind: "selection"; schema: string; tables: string[]; views: string[]; functions: string[]; sequences: string[]; types: string[] };

export interface DumpRequest {
  /** Identifica o dump para o progresso e o cancelamento (`exportCancel`). */
  dumpId: string;
  sessionId: string;
  database: string;
  options: { scope: DumpScope; structure: boolean; data: boolean; clean: boolean; includeOwner: boolean };
  /** `copy` = COPY ... FROM stdin (rápido, precisa do psql); `insert` = comandos INSERT. */
  dataFormat: "copy" | "insert";
  rowsPerInsert: number;
  path: string;
}

export type BackupContent = "all" | "structure" | "data";
export interface BackupRequest {
  jobId: string; sessionId: string; database: string; /** Sem schema o backup é do banco inteiro. */ schema?: string; toolsDir?: string;
  /** Nome do backup, sem a extensão. */
  fileName: string; comment: string;
  /** `schema.nome` dos objetos desmarcados (tabelas, views e sequências). */
  exclude: string[];
  /** Pasta de destino; sem ela vale a pasta padrão do app. */
  folder?: string;
}
export interface BackupEntry { name: string; path: string; metaPath: string; bytes: number; modifiedMs: number; comment: string; scope: "schema" | "database" }
export interface RestoreRequest {
  jobId: string; sessionId: string; database: string; toolsDir?: string; path: string;
  content: BackupContent; clean: boolean; noOwner: boolean; noPrivileges: boolean; singleTransaction: boolean;
  /** Em vez de restaurar, grava o SQL do backup neste arquivo. */
  extractTo?: string;
}
export interface BackupResult { bytes: number; elapsedMs: number; path: string; tool: string; name: string }
export interface ToolsInfo { dir: string | null; version: number | null; others: { dir: string; version: number }[] }

export interface DumpResult { objects: number; tables: number; rows: number; bytes: number; elapsedMs: number; path: string }

export type FindMode = "contains" | "whole" | "prefix" | "regex";

export interface FindOptions {
  text: string;
  /** Só este schema; null = todos os schemas do usuário. */
  schema: string | null;
  mode: FindMode;
  caseInsensitive: boolean;
  tables: boolean;
  views: boolean;
  matviews: boolean;
  functions: boolean;
  indexes: boolean;
  triggers: boolean;
}

export type FindWhat =
  | "tableName" | "fieldName" | "foreignKeyName" | "indexName" | "triggerName"
  | "viewName" | "viewBody" | "matviewName" | "matviewBody" | "functionName" | "functionBody";

/** Um achado: o texto mostrado e onde destacar (posições em caracteres). */
export interface FindMatch {
  what: FindWhat;
  text: string;
  ranges: [number, number][];
}

export interface FindObject {
  schema: string;
  kind: "table" | "view" | "matview" | "function";
  name: string;
  /** Funções: argumentos de identidade. */
  identity: string | null;
  matches: FindMatch[];
}

export interface FindResult {
  objects: FindObject[];
  occurrences: number;
  /** Havia mais ocorrências que o limite. */
  truncated: boolean;
}

export interface DataMatch {
  schema: string;
  table: string;
  count: number;
  /** Consulta que lista as linhas achadas. */
  sql: string;
}

export interface FindDataResult {
  tables: DataMatch[];
  scanned: number;
  total: number;
  skipped: number;
  cancelled: boolean;
}

/** Banco de dados: encoding, collation e tablespace são só leitura (o ALTER não os troca). */
export interface DatabaseDetail {
  name: string;
  owner: string;
  encoding: string;
  collate: string;
  ctype: string;
  tablespace: string;
  /** -1 = sem limite. */
  connectionLimit: number;
  allowConnections: boolean;
  isTemplate: boolean;
  comment: string | null;
}

export interface SchemaDetail {
  name: string;
  owner: string;
  comment: string | null;
}

/** Schema da lista da árvore, com o comentário (dica ao passar o mouse). */
export interface SchemaInfo {
  name: string;
  comment: string | null;
}

export interface RoleDetail {
  name: string;
  superuser: boolean;
  inherit: boolean;
  createRole: boolean;
  createDb: boolean;
  canLogin: boolean;
  replication: boolean;
  /** PostgreSQL 9.5+; null antes disso. */
  bypassRls: boolean | null;
  /** -1 = sem limite. */
  connectionLimit: number;
  /** VALID UNTIL como texto do servidor; null = sem validade. */
  validUntil: string | null;
  comment: string | null;
  memberOf: string[];
  members: string[];
}

export interface TriggerInfo {
  name: string;
  definition: string;
  enabled: boolean;
  timing: "BEFORE" | "AFTER" | "INSTEAD OF";
  events: ("INSERT" | "UPDATE" | "DELETE" | "TRUNCATE")[];
  forEachRow: boolean;
  when: string | null;
  /** Função chamada, como schema.nome. */
  function: string;
  args: string[];
  updateColumns: string[];
  comment: string | null;
  /** Gatilho de restrição ou com tabelas de transição: o designer só o exibe. */
  readOnly: boolean;
}

export interface TypeDetail {
  name: string;
  kind: "enum" | "composite" | "domain" | "range";
  owner: string | null;
  comment: string | null;
  labels: string[];
  attributes: { name: string; dataType: string }[];
  baseType: string | null;
  default: string | null;
  notNull: boolean;
  /** `definition` vem do servidor, ex.: CHECK ((VALUE > 0)). */
  checks: { name: string; definition: string }[];
  subtype: string | null;
}

export interface SequenceDetail {
  name: string;
  owner: string | null;
  comment: string | null;
  dataType: "smallint" | "integer" | "bigint";
  /** Números como texto: um bigint não cabe num número JSON. */
  start: string;
  increment: string;
  min: string;
  max: string;
  cache: string;
  cycle: boolean;
  lastValue: string;
  /** Já entregou algum valor; falso = o próximo nextval devolve lastValue. */
  isCalled: boolean;
  /** Coluna dona (OWNED BY), como "schema"."tabela"."coluna". */
  ownedBy: string | null;
}

export interface FunctionDetail {
  name: string;
  language: string;
  body: string;
  /** Tipo de retorno; null em procedimentos. */
  returns: string | null;
  /** Argumentos como o servidor imprime (modo, nome, tipo, DEFAULT). */
  arguments: string;
  /** Nome de cada argumento, na ordem de `arguments` (vazio = sem nome). */
  argNames: string[];
  /** Argumentos que identificam a função (sem DEFAULT). */
  identity: string;
  volatility: "immutable" | "stable" | "volatile";
  securityDefiner: boolean;
  strict: boolean;
  owner: string | null;
  comment: string | null;
  procedure: boolean;
}

export interface ResultSet {
  columns: { name: string; typeName: string | null }[];
  rows: (string | null)[][];
  truncated: boolean;
}

export interface QueryOutcome {
  statements: { result: ResultSet | null; rowsAffected: number | null; sql: string; queryTime: number; fetchTime: number }[];
  elapsedMs: number;
}

/** Data Profiling: o que se sabe de cada coluna dos registros perfilados. */
export interface ValueCount { value: string | null; count: number }
export interface ColumnProfile {
  name: string;
  rows: number;
  nulls: number;
  distinct: number | null;
  unique: number | null;
  min: string | null;
  max: string | null;
  top: ValueCount[];
}
export interface DataProfile { rows: number; columns: ColumnProfile[] }

export interface PlannedStatement {
  sql: string;
  params: (string | null)[];
  expectOneRow: boolean;
  /** Tabela sem chave: o comando precisa mexer em exatamente esta quantidade de linhas (as marcadas na tela). */
  expectRows?: number;
}

const call = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
  try {
    // Conexão de outro banco (SQLite...) vai para o comando equivalente; no PostgreSQL a chamada sai como entrou.
    const routed = routeCommand(cmd, args);
    return await invoke<T>(routed.cmd, routed.args);
  } catch (e) {
    throw toAtlasError(e);
  }
};

/* ---------- Armazenamento local (atlas.db + cofre de credenciais) ---------- */

/** Perfil salvo. Nunca contém senha: `hasPassword` só diz que há credencial no cofre do sistema. */
export interface ProfileRecord {
  id: string;
  name: string;
  provider: string;
  host: string;
  port: number;
  initialDatabase: string;
  user: string;
  environment: string;
  options: Record<string, unknown>;
  sortOrder: number;
  hasPassword: boolean;
}

/** A interface terminou de carregar: o núcleo troca a tela de abertura pela janela principal. */
export const appReady = () => call<void>("app_ready");

/** Consulta executada no editor, como o histórico a guarda. */
export interface HistoryEntry {
  id: string;
  connectionId: string | null;
  connectionName: string | null;
  database: string | null;
  schema: string | null;
  sql: string;
  /** ISO 8601 em UTC. */
  executedUtc: string;
  durationMs: number | null;
  status: "ok" | "error";
  rows: number | null;
  error: string | null;
  favorite: boolean;
}

export interface NewHistory {
  connectionId: string | null;
  connectionName: string;
  database: string | null;
  schema: string | null;
  sql: string;
  durationMs: number | null;
  rows: number | null;
  error: string | null;
}

export interface HistoryQuery {
  search: string;
  connectionId: string | null;
  favoritesOnly: boolean;
  errorsOnly: boolean;
  limit: number;
  offset: number;
}

/** Trecho de código reutilizável; `${1:padrão}` no corpo vira ponto de tabulação ao inserir. */
export interface Snippet {
  id: string;
  name: string;
  body: string;
  updatedUtc: string;
}

export const local = {
  status: () => call<{ path: string; schemaVersion: number | null; error: string | null }>("local_status"),
  profiles: () => call<ProfileRecord[]>("profiles_list"),
  /** Senha (ou frase da chave) do servidor SSH do perfil, guardada no cofre; `save` falso a remove. */
  sshPasswordSave: (id: string, password: string | null, save: boolean) => call<void>("ssh_password_save", { id, password, save }),
  /** Baixa um arquivo `http(s)` para a pasta temporária (até 200 MB) e devolve o caminho. */
  downloadToTemp: (url: string) => call<{ path: string; bytes: number }>("download_to_temp", { url }),
  /** Abre um endereço do GitHub do projeto no navegador padrão. */
  openExternal: (url: string) => call<void>("open_external", { url }),
  /** Conteúdo de um arquivo como `\\x...` (hexa do bytea), até 64 MB. */
  fileReadHex: (path: string) => call<string>("file_read_hex", { path }),
  /** Grava os bytes de um valor `\\x...` num arquivo; devolve quantos. */
  fileWriteHex: (path: string, hex: string) => call<number>("file_write_hex", { path, hex }),
  /** Guarda o estado de uma tela por pouco tempo (para destacar, anexar ou fechar o banco sem perder o que está na tela) e o devolve uma vez só. */
  tabStateSave: (id: string, state: unknown) => call<void>("tab_state_save", { id, json: JSON.stringify(state) }),
  tabStateTake: async (id: string): Promise<unknown | null> => {
    const text = await call<string | null>("tab_state_take", { id });
    try { return text ? JSON.parse(text) : null; } catch { return null; }
  },
  /** Grava as conexões num arquivo JSON (sem senhas); devolve quantas. */
  connectionsExport: (path: string, profiles: ProfileRecord[]) => call<number>("connections_export", { path, profiles }),
  /** Lê as conexões de um arquivo exportado. */
  connectionsImport: (path: string) => call<ProfileRecord[]>("connections_import", { path }),
  /** `password` null/vazio + `savePassword` = mantém a senha já salva; `savePassword` false = remove do cofre. */
  saveProfile: (profile: ProfileRecord, password: string | null, savePassword: boolean) =>
    call<void>("profile_save", { profile, password, savePassword }),
  deleteProfile: (id: string) => call<void>("profile_delete", { id }),
  historyAdd: (entry: NewHistory) => call<string>("history_add", { entry }),
  historyList: (query: HistoryQuery) => call<HistoryEntry[]>("history_list", { query }),
  historyFavorite: (id: string, favorite: boolean) => call<void>("history_favorite", { id, favorite }),
  historyDelete: (id: string) => call<void>("history_delete", { id }),
  /** Apaga o histórico; com `keepFavorites` os favoritos ficam. Devolve quantas consultas saíram. */
  historyClear: (keepFavorites: boolean) => call<number>("history_clear", { keepFavorites }),
  snippetsList: () => call<Snippet[]>("snippets_list"),
  /** `id` null cria; com id atualiza. Devolve o id. */
  snippetSave: (id: string | null, name: string, body: string) => call<string>("snippet_save", { id, name, body }),
  snippetDelete: (id: string) => call<void>("snippet_delete", { id }),
  getPref: <T>(key: string) => call<T | null>("pref_get", { key }),
  setPref: (key: string, value: unknown) => call<void>("pref_set", { key, value }),
};

export const pg = {
  /** `profileId`: com senha vazia, o núcleo usa a senha salva no cofre (a senha nunca vem para a UI). */
  testConnection: (params: ConnectionParams, profileId?: string) => call<ServerInfo>("pg_test_connection", { params, profileId }),
  open: (sessionId: string, params: ConnectionParams, profileId?: string) => call<ServerInfo>("pg_open", { sessionId, params, profileId }),
  close: (sessionId: string) => call<void>("pg_close", { sessionId }),
  closeDatabase: (sessionId: string, database: string) => call<void>("pg_close_database", { sessionId, database }),
  /** `comments`: comentário de cada banco que tem um (dica na árvore). */
  /** `includeHidden`: inclui também os bancos que a conexão normalmente oculta (templates e sem conexão). */
  listDatabases: (sessionId: string, includeHidden = false) =>
    call<{ databases: string[]; hidden: number; comments: Record<string, string> }>("pg_list_databases", { sessionId, includeHidden }),
  /** `includeHidden`: inclui os schemas de sistema (`pg_catalog`, `information_schema`...). */
  listSchemas: (sessionId: string, database: string, includeHidden = false) =>
    call<SchemaInfo[]>("pg_list_schemas", { sessionId, database, includeHidden }),
  listObjects: (sessionId: string, database: string, schema: string) => call<SchemaObjects>("pg_list_objects", { sessionId, database, schema }),
  tableDetail: (sessionId: string, database: string, schema: string, table: string) =>
    call<TableDetail>("pg_table_detail", { sessionId, database, schema, table }),
  viewDetail: (sessionId: string, database: string, schema: string, view: string) =>
    call<ViewDetail>("pg_view_detail", { sessionId, database, schema, view }),
  syncCompare: (req: SyncCompareRequest) => call<SyncSummary>("datasync_compare", { req }),
  syncRows: (compareId: string, table: string, kind: string | null, offset: number, limit: number) =>
    call<SyncRowsPage>("datasync_rows", { compareId, table, kind, offset, limit }),
  syncScript: (compareId: string, selection: SyncSelection[]) => call<SyncScript>("datasync_script", { compareId, selection }),
  syncScriptFull: (compareId: string, selection: SyncSelection[]) => call<string>("datasync_script_full", { compareId, selection }),
  syncDeploy: (req: SyncDeployRequest) => call<SyncDeployCounts>("datasync_deploy", { req }),
  consoleRun: (sessionId: string, database: string, input: string, options: ConsoleOptions) => call<ConsoleOutput>("console_run", { sessionId, database, input, options }),
  consolePrompt: (sessionId: string, database: string) => call<string>("console_prompt", { sessionId, database }),
  consoleComplete: (sessionId: string, database: string, text: string) => call<ConsoleCompletion>("console_complete", { sessionId, database, text }),
  // Um comando com `.` (consoles do SQLite e do SQL Server) ou uma linha `GO` (SQL Server) fecha o comando; no PostgreSQL nada disso aparece.
  consoleIsComplete: (input: string) => (input.trimStart().startsWith(".") || /(^|\n)\s*go\s*$/i.test(input) ? Promise.resolve(true) : call<boolean>("console_is_complete", { input })),
  sqlLogRead: (after: number) => call<SqlLogPage>("sqllog_read", { after }),
  sqlLogClear: () => call<void>("sqllog_clear"),
  sqlLogSetPaused: (paused: boolean) => call<void>("sqllog_set_paused", { paused }),
  monitorSnapshot: (sessionId: string) => call<MonitorSnapshot>("monitor_snapshot", { sessionId }),
  monitorEndProcess: (sessionId: string, pid: number) => call<boolean>("monitor_end_process", { sessionId, pid }),
  dictDefaults: (title: string) => call<DictSettings>("dict_defaults", { title }),
  dictPreview: (servers: DictServerSel[], settings: DictSettings, which: string) => call<DictPreviewPage | null>("dict_preview", { servers, settings, which }),
  dictGenerate: (req: DictGenerateRequest) => call<DictResult>("dict_generate", { req }),
  dictCancel: (runId: string) => call<void>("dict_cancel", { runId }),
  dictOpen: (path: string, reveal: boolean) => call<void>("dict_open", { path, reveal }),
  genDescribe: (conn: string, database: string, schema: string) => call<GenTableMeta[]>("datagen_describe", { conn, database, schema }),
  genPreview: (conn: string, database: string, schema: string, table: GenTableSpec, rows: number) => call<GenPreview>("datagen_preview", { conn, database, schema, table, rows }),
  genRun: (req: { runId: string; conn: string; database: string; spec: GenSpec }) => call<GenResult>("datagen_run", { req }),
  structCompare: (req: StructCompareRequest) => call<StructObject[]>("structsync_compare", { req }),
  structScript: (compareId: string, selection: StructSelection[]) => call<string[]>("structsync_script", { compareId, selection }),
  structDeploy: (req: StructDeployRequest) => call<SyncDeployCounts>("structsync_deploy", { req }),
  structForget: (compareId: string) => call<void>("structsync_forget", { compareId }),
  syncForget: (compareId: string) => call<void>("datasync_forget", { compareId }),
  transfer: (req: TransferRequest) => call<TransferCounts>("pg_transfer", { req }),
  importListTables: (path: string, format: ImportFormat) => call<string[]>("import_list_tables", { path, format }),
  importPreview: (spec: SourceSpec, convert: ConvertOptions, limit: number) => call<ImportPreview>("import_preview", { spec, convert, limit }),
  import: (req: ImportRequest) => call<ImportSummary>("pg_import", { req }),
  dumpSql: (req: DumpRequest) => call<DumpResult>("pg_dump_sql", { req }),
  /** Procura o pg_dump/pg_restore (na pasta informada ou nos locais usuais). */
  toolsLocate: (customDir?: string) => call<ToolsInfo>("pg_tools_locate", { customDir: customDir || null }),
  backup: (req: BackupRequest) => call<BackupResult>("pg_backup", { req }),
  restore: (req: RestoreRequest) => call<BackupResult>("pg_restore", { req }),
  backupCancel: (jobId: string) => call<void>("pg_backup_cancel", { jobId }),
  backupList: (sessionId: string, database: string, schema: string) => call<BackupEntry[]>("pg_backup_list", { sessionId, database, schema }),
  backupDelete: (path: string, metaPath: string) => call<void>("pg_backup_delete", { path, metaPath }),
  /** Rótulos (coluna de descrição da tabela referenciada) das chaves pedidas; `labelColumn` nulo = a tabela não tem uma que sirva. */
  fkLabels: (sessionId: string, database: string, schema: string, table: string, keyColumn: string, keys: string[]) =>
    call<{ labelColumn: string | null; labels: [string, string][] }>("pg_fk_labels", { sessionId, database, schema, table, keyColumn, keys }),
  find: (sessionId: string, database: string, options: FindOptions) => call<FindResult>("pg_find", { sessionId, database, options }),
  /** `findId` identifica a busca para o `exportCancel` (botão Parar). */
  findData: (sessionId: string, database: string, findId: string, options: FindOptions) =>
    call<FindDataResult>("pg_find_data", { sessionId, database, findId, options }),
  schemaDiagram: (sessionId: string, database: string, schema: string) =>
    call<SchemaDiagram>("pg_schema_diagram", { sessionId, database, schema }),
  availableExtensions: (sessionId: string, database: string) =>
    call<AvailableExtension[]>("pg_available_extensions", { sessionId, database }),
  foreignWrappers: (sessionId: string, database: string) => call<string[]>("pg_foreign_wrappers", { sessionId, database }),
  foreignTableDetail: (sessionId: string, database: string, schema: string, name: string) =>
    call<ForeignTableDetail>("pg_foreign_table_detail", { sessionId, database, schema, name }),
  dataProfile: (sessionId: string, database: string, schema: string, table: string, columns: string[], orderBy: OrderBy[], filter: TableFilter | null, limit: number | null) =>
    call<DataProfile>("pg_data_profile", { sessionId, database, schema, table, columns, orderBy, filter, limit }),
  tableProfile: (sessionId: string, database: string, schema: string, table: string) =>
    call<TableProfile>("pg_table_profile", { sessionId, database, schema, table }),
  tablespaceDetail: (sessionId: string, database: string, name: string) => call<TablespaceDetail>("pg_tablespace_detail", { sessionId, database, name }),
  foreignServerDetail: (sessionId: string, database: string, name: string) => call<ForeignServerDetail>("pg_foreign_server_detail", { sessionId, database, name }),
  extensionDetail: (sessionId: string, database: string, name: string) =>
    call<ExtensionDetail>("pg_extension_detail", { sessionId, database, name }),
  databaseDetail: (sessionId: string, name: string) => call<DatabaseDetail>("pg_database_detail", { sessionId, name }),
  schemaDetail: (sessionId: string, database: string, name: string) =>
    call<SchemaDetail>("pg_schema_detail", { sessionId, database, name }),
  roleDetail: (sessionId: string, name: string) => call<RoleDetail>("pg_role_detail", { sessionId, name }),
  typeDetail: (sessionId: string, database: string, schema: string, name: string) =>
    call<TypeDetail>("pg_type_detail", { sessionId, database, schema, name }),
  sequenceDetail: (sessionId: string, database: string, schema: string, name: string) =>
    call<SequenceDetail>("pg_sequence_detail", { sessionId, database, schema, name }),
  functionDetail: (sessionId: string, database: string, schema: string, name: string, identity: string) =>
    call<FunctionDetail>("pg_function_detail", { sessionId, database, schema, name, identity }),
  matviewDetail: (sessionId: string, database: string, schema: string, view: string) =>
    call<ViewDetail>("pg_matview_detail", { sessionId, database, schema, view }),
  listRoles: (sessionId: string) => call<RoleInfo[]>("pg_list_roles", { sessionId }),
  /** `schema`: onde os nomes sem schema da consulta são procurados (o schema escolhido no editor). */
  execute: (sessionId: string, database: string, sql: string, maxRows: number, schema?: string) =>
    call<QueryOutcome>("pg_execute", { sessionId, database, sql, maxRows, schema: schema || null }),
  cancel: (sessionId: string, database: string) => call<void>("pg_cancel", { sessionId, database }),
  /** `limit = null`: tabela inteira (até o teto do backend; o excedente vem como `truncated`). */
  tableRows: (sessionId: string, database: string, schema: string, table: string, orderBy: OrderBy[], filter: TableFilter | null, limit: number | null, offset: number) =>
    call<{ sql: string; outcome: QueryOutcome }>("pg_table_rows", { sessionId, database, schema, table, orderBy, filter, limit, offset }),
  tableCount: (sessionId: string, database: string, schema: string, table: string, filter: TableFilter | null) =>
    call<number>("pg_table_count", { sessionId, database, schema, table, filter }),
  apply: (sessionId: string, database: string, statements: PlannedStatement[]) =>
    call<number[]>("pg_apply", { sessionId, database, statements }),
  /** Transação manual (conexão dedicada que fica com a transação aberta até `txEnd`). */
  txBegin: (sessionId: string, database: string) => call<string>("pg_tx_begin", { sessionId, database }),
  /** Executa SQL do editor dentro da transação aberta. */
  txExecute: (txId: string, sql: string, maxRows: number, schema?: string) => call<QueryOutcome>("pg_tx_execute", { txId, sql, maxRows, schema: schema ?? null }),
  txApply: (txId: string, statements: PlannedStatement[]) => call<number[]>("pg_tx_apply", { txId, statements }),
  txRows: (txId: string, schema: string, table: string, orderBy: OrderBy[], filter: TableFilter | null, limit: number | null, offset: number) =>
    call<{ sql: string; outcome: QueryOutcome }>("pg_tx_table_rows", { txId, schema, table, orderBy, filter, limit, offset }),
  txCount: (txId: string, schema: string, table: string, filter: TableFilter | null) =>
    call<number>("pg_tx_table_count", { txId, schema, table, filter }),
  txEnd: (txId: string, commit: boolean) => call<void>("pg_tx_end", { txId, commit }),
  export: (req: ExportRequest) => call<ExportResult>("pg_export", { req }),
  exportCancel: (exportId: string) => call<void>("pg_export_cancel", { exportId }),
};

/* ---------- Exportação de dados (CSV, JSON, SQL, Excel) ---------- */

export type ExportEncoding = "utf8" | "utf8Bom" | "windows1252";

/** Formato e opções; o mesmo desenho que o Rust lê (`atlas-export`). */
export type ExportFormat =
  | { kind: "delimited"; delimiter: string; qualifier: string; header: boolean; crlf: boolean; nullText: string; encoding: ExportEncoding }
  | { kind: "json"; pretty: boolean }
  | { kind: "sql"; rowsPerInsert: number }
  | { kind: "xlsx"; header: boolean; sheetName: string }
  | { kind: "xls"; header: boolean; sheetName: string }
  | { kind: "dbf"; decimals: number; textLength: number }
  | { kind: "xml" }
  | { kind: "html"; header: boolean };

export interface ExportRequest {
  exportId: string;
  sessionId: string;
  database: string;
  schema: string;
  table: string;
  /** Vazio = todas as colunas. */
  columns: string[];
  orderBy: OrderBy[];
  filter: TableFilter | null;
  /** Consulta do editor como origem; `table` vira só o nome do XML/HTML e do destino do `INSERT`. */
  query?: { sql: string; schema?: string | null };
  format: ExportFormat;
  path: string;
}

export interface ExportResult { rows: number; bytes: number; elapsedMs: number; path: string }

/** Progresso da exportação em andamento (a cada ~200 ms). Devolve a função que cancela a assinatura. */
export async function onExportProgress(exportId: string, cb: (rows: number) => void): Promise<() => void> {
  return listen<{ exportId: string; rows: number }>("export-progress", (e) => {
    if (e.payload.exportId === exportId) cb(e.payload.rows);
  });
}

/** Janela nativa "Salvar como". `null` se o usuário cancelar. */
export async function chooseExportPath(defaultName: string, extensions: string[]): Promise<string | null> {
  // Só em desenvolvimento: testes automatizados informam o arquivo sem abrir a janela nativa.
  const injected = import.meta.env.DEV ? (window as { __atlasSavePath?: string }).__atlasSavePath : undefined;
  if (injected) return injected;
  return save({ defaultPath: defaultName, filters: [{ name: extensions.map((x) => `*.${x}`).join(", "), extensions }] });
}

/** Saída (linha a linha) do pg_dump/pg_restore/psql de um backup ou restauração em andamento. */
export async function onBackupLog(jobId: string, cb: (line: string) => void): Promise<() => void> {
  return listen<{ jobId: string; line: string }>("backup-log", (e) => {
    if (e.payload.jobId === jobId) cb(e.payload.line);
  });
}

/** Janela nativa "Abrir arquivo". `null` se o usuário cancelar. */
export async function chooseOpenFile(extensions: string[]): Promise<string | null> {
  // Só em desenvolvimento: testes automatizados informam o arquivo sem abrir a janela nativa.
  const injected = import.meta.env.DEV ? (window as { __atlasOpenPath?: string }).__atlasOpenPath : undefined;
  if (injected) return injected;
  const f = await open({ multiple: false, filters: [{ name: extensions.map((x) => `*.${x}`).join(", "), extensions }, { name: "*", extensions: ["*"] }] });
  return typeof f === "string" ? f : null;
}

/** Janela nativa de escolha de pasta. `null` se o usuário cancelar. */
export async function chooseExportFolder(): Promise<string | null> {
  const dir = await open({ directory: true, multiple: false });
  return typeof dir === "string" ? dir : null;
}

/** Pasta padrão dos arquivos exportados: a Área de Trabalho (ou a pasta do usuário); "" se nenhuma existir. */
export async function defaultExportDir(): Promise<string> {
  try {
    return await desktopDir();
  } catch {
    try {
      return await homeDir();
    } catch {
      return "";
    }
  }
}

/** Separador de caminho do sistema (`\` no Windows, `/` nos demais). */
export const pathSeparator = (): string => {
  try {
    return sep();
  } catch {
    return "/";
  }
};
