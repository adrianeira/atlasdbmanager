import type { RoleDetail } from "../backend";
import { quoteIdent } from "../model";

/** Estado editável de um papel (usuário ou grupo). */
export interface RoleState {
  name: string;
  /** Senha nova; vazio = não mexe. Nunca é lida do servidor. */
  password: string;
  superuser: boolean;
  createDb: boolean;
  createRole: boolean;
  inherit: boolean;
  canLogin: boolean;
  replication: boolean;
  bypassRls: boolean;
  /** -1 = sem limite. */
  connLimit: string;
  /** Vazio = sem validade. */
  validUntil: string;
  comment: string;
  memberOf: string[];
  members: string[];
}

export const emptyRole = (): RoleState => ({
  name: "", password: "", superuser: false, createDb: false, createRole: false, inherit: true, canLogin: true, replication: false,
  bypassRls: false, connLimit: "-1", validUntil: "", comment: "", memberOf: [], members: [],
});

export function fromDetail(d: RoleDetail): RoleState {
  return {
    name: d.name, password: "", superuser: d.superuser, createDb: d.createDb, createRole: d.createRole, inherit: d.inherit,
    canLogin: d.canLogin, replication: d.replication, bypassRls: d.bypassRls ?? false, connLimit: String(d.connectionLimit),
    validUntil: d.validUntil ?? "", comment: d.comment ?? "", memberOf: [...d.memberOf], members: [...d.members],
  };
}

export function validateRole(s: RoleState): "name" | "limit" | null {
  if (!s.name.trim()) return "name";
  if (!/^-?\d+$/.test(s.connLimit.trim()) || Number(s.connLimit) < -1) return "limit";
  return null;
}

const literal = (s: string) => `'${s.replace(/'/g, "''")}'`;
/** A senha só aparece como texto no comando que vai ao servidor; a revisão e a prévia mostram asteriscos. */
const passwordClause = (pw: string, mask: boolean) => `PASSWORD ${mask ? "'********'" : literal(pw)}`;

/** Atributos que diferem do padrão do `CREATE ROLE` (ou, com `before`, os que mudaram). */
function attributeClauses(after: RoleState, before: RoleState | null, supportsBypassRls: boolean, mask: boolean): string[] {
  const base = before ?? { ...emptyRole(), canLogin: false };
  const out: string[] = [];
  const flag = (key: keyof RoleState, on: string, off: string) => {
    if (after[key] !== base[key]) out.push(after[key] ? on : off);
  };
  flag("superuser", "SUPERUSER", "NOSUPERUSER");
  flag("createDb", "CREATEDB", "NOCREATEDB");
  flag("createRole", "CREATEROLE", "NOCREATEROLE");
  flag("inherit", "INHERIT", "NOINHERIT");
  flag("canLogin", "LOGIN", "NOLOGIN");
  flag("replication", "REPLICATION", "NOREPLICATION");
  if (supportsBypassRls) flag("bypassRls", "BYPASSRLS", "NOBYPASSRLS");
  if (Number(after.connLimit) !== Number(base.connLimit)) out.push(`CONNECTION LIMIT ${Number(after.connLimit)}`);
  if (after.validUntil.trim() !== base.validUntil.trim()) out.push(`VALID UNTIL ${after.validUntil.trim() ? literal(after.validUntil.trim()) : "'infinity'"}`);
  if (after.password) out.push(passwordClause(after.password, mask));
  return out;
}

const q = quoteIdent;

/** DDL de um papel novo (também a prévia "Salvar como"). */
export function createRoleStatements(s: RoleState, supportsBypassRls: boolean, mask = false): string[] {
  const name = s.name.trim();
  const clauses = attributeClauses(s, null, supportsBypassRls, mask);
  const out = [`CREATE ROLE ${q(name)}${clauses.length ? ` WITH ${clauses.join(" ")}` : ""};`];
  for (const g of s.memberOf) out.push(`GRANT ${q(g)} TO ${q(name)};`);
  for (const m of s.members) out.push(`GRANT ${q(name)} TO ${q(m)};`);
  if (s.comment) out.push(`COMMENT ON ROLE ${q(name)} IS ${literal(s.comment)};`);
  return out;
}

/** Só o que mudou: renomear, atributos, senha, participações e comentário. */
export function alterRoleStatements(before: RoleState, after: RoleState, supportsBypassRls: boolean, mask = false): string[] {
  const out: string[] = [];
  let name = before.name;
  if (after.name.trim() !== before.name) {
    out.push(`ALTER ROLE ${q(before.name)} RENAME TO ${q(after.name.trim())};`);
    name = after.name.trim();
  }
  const clauses = attributeClauses(after, before, supportsBypassRls, mask);
  if (clauses.length) out.push(`ALTER ROLE ${q(name)} WITH ${clauses.join(" ")};`);
  const diff = (a: string[], b: string[]) => a.filter((x) => !b.includes(x));
  for (const g of diff(after.memberOf, before.memberOf)) out.push(`GRANT ${q(g)} TO ${q(name)};`);
  for (const g of diff(before.memberOf, after.memberOf)) out.push(`REVOKE ${q(g)} FROM ${q(name)};`);
  for (const m of diff(after.members, before.members)) out.push(`GRANT ${q(name)} TO ${q(m)};`);
  for (const m of diff(before.members, after.members)) out.push(`REVOKE ${q(name)} FROM ${q(m)};`);
  if (after.comment !== before.comment) out.push(`COMMENT ON ROLE ${q(name)} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}

export const dropRoleSql = (name: string): string => `DROP ROLE ${q(name)};`;
