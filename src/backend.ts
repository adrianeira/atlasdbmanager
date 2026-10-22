/*
 * Ponte com o núcleo Rust (comandos IPC do Tauri).
 * Dentro do app desktop tudo vai ao banco real; no navegador (`npm run dev` puro)
 * a interface continua no modo de demonstração com dados fictícios.
 */
import { invoke } from "@tauri-apps/api/core";

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
}

export interface SchemaObjects {
  tables: ObjectInfo[];
  views: ObjectInfo[];
  matviews: ObjectInfo[];
  functions: ObjectInfo[];
  sequences: ObjectInfo[];
}

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
}

export interface IndexInfo { name: string; fields: string; method: string; unique: boolean; primary: boolean; definition: string; comment: string | null }
export interface ConstraintInfo {
  name: string; kind: "f" | "u" | "c" | "x"; fields: string; refSchema: string | null; refTable: string | null;
  refFields: string | null; onDelete: string | null; onUpdate: string | null; definition: string; comment: string | null;
}
export interface TriggerInfo { name: string; definition: string; enabled: boolean }

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
}

export interface ResultSet {
  columns: { name: string; typeName: string | null }[];
  rows: (string | null)[][];
  truncated: boolean;
}

export interface QueryOutcome {
  statements: { result: ResultSet | null; rowsAffected: number | null }[];
  elapsedMs: number;
}

export interface PlannedStatement {
  sql: string;
  params: (string | null)[];
  expectOneRow: boolean;
}

const call = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
  try {
    return await invoke<T>(cmd, args);
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

export const local = {
  status: () => call<{ path: string; schemaVersion: number | null; error: string | null }>("local_status"),
  profiles: () => call<ProfileRecord[]>("profiles_list"),
  /** `password` null/vazio + `savePassword` = mantém a senha já salva; `savePassword` false = remove do cofre. */
  saveProfile: (profile: ProfileRecord, password: string | null, savePassword: boolean) =>
    call<void>("profile_save", { profile, password, savePassword }),
  deleteProfile: (id: string) => call<void>("profile_delete", { id }),
  getPref: <T>(key: string) => call<T | null>("pref_get", { key }),
  setPref: (key: string, value: unknown) => call<void>("pref_set", { key, value }),
};

export const pg = {
  /** `profileId`: com senha vazia, o núcleo usa a senha salva no cofre (a senha nunca vem para a UI). */
  testConnection: (params: ConnectionParams, profileId?: string) => call<ServerInfo>("pg_test_connection", { params, profileId }),
  open: (sessionId: string, params: ConnectionParams, profileId?: string) => call<ServerInfo>("pg_open", { sessionId, params, profileId }),
  close: (sessionId: string) => call<void>("pg_close", { sessionId }),
  closeDatabase: (sessionId: string, database: string) => call<void>("pg_close_database", { sessionId, database }),
  listDatabases: (sessionId: string) => call<{ databases: string[]; hidden: number }>("pg_list_databases", { sessionId }),
  listSchemas: (sessionId: string, database: string) => call<string[]>("pg_list_schemas", { sessionId, database }),
  listObjects: (sessionId: string, database: string, schema: string) => call<SchemaObjects>("pg_list_objects", { sessionId, database, schema }),
  tableDetail: (sessionId: string, database: string, schema: string, table: string) =>
    call<TableDetail>("pg_table_detail", { sessionId, database, schema, table }),
  viewDetail: (sessionId: string, database: string, schema: string, view: string) =>
    call<ViewDetail>("pg_view_detail", { sessionId, database, schema, view }),
  listRoles: (sessionId: string) => call<RoleInfo[]>("pg_list_roles", { sessionId }),
  execute: (sessionId: string, database: string, sql: string, maxRows: number) =>
    call<QueryOutcome>("pg_execute", { sessionId, database, sql, maxRows }),
  cancel: (sessionId: string, database: string) => call<void>("pg_cancel", { sessionId, database }),
  /** `limit = null`: tabela inteira (até o teto do backend; o excedente vem como `truncated`). */
  tableRows: (sessionId: string, database: string, schema: string, table: string, orderBy: string[], limit: number | null, offset: number) =>
    call<{ sql: string; outcome: QueryOutcome }>("pg_table_rows", { sessionId, database, schema, table, orderBy, limit, offset }),
  apply: (sessionId: string, database: string, statements: PlannedStatement[]) =>
    call<number[]>("pg_apply", { sessionId, database, statements }),
};
