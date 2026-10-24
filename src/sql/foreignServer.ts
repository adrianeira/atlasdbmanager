import type { ForeignServerDetail, OptionPair } from "../backend";
import { quoteIdent } from "../model";

export interface MappingState {
  /** `public` é o mapeamento de todos os usuários. */
  user: string;
  options: OptionPair[];
}

/** Estado editável de um servidor estrangeiro (novo ou existente). */
export interface ServerState {
  name: string;
  wrapper: string;
  type: string;
  version: string;
  owner: string;
  options: OptionPair[];
  mappings: MappingState[];
  comment: string;
}

export const emptyServer = (): ServerState => ({ name: "", wrapper: "", type: "", version: "", owner: "", options: [], mappings: [], comment: "" });

export function fromDetail(d: ForeignServerDetail): ServerState {
  return {
    name: d.name, wrapper: d.wrapper, type: d.serverType ?? "", version: d.version ?? "", owner: d.owner,
    options: d.options.map((o) => ({ ...o })), mappings: d.mappings.map((m) => ({ user: m.user, options: m.options.map((o) => ({ ...o })) })), comment: d.comment ?? "",
  };
}

export type ServerProblem = "name" | "wrapper" | "optionKey" | "mappingUser" | "mappingDuplicate";

export function validateServer(s: ServerState): ServerProblem | null {
  if (!s.name.trim()) return "name";
  if (!s.wrapper.trim()) return "wrapper";
  if ([...s.options, ...s.mappings.flatMap((m) => m.options)].some((o) => !o.key.trim())) return "optionKey";
  if (s.mappings.some((m) => !m.user.trim())) return "mappingUser";
  const users = s.mappings.map((m) => m.user.trim().toLowerCase());
  if (new Set(users).size !== users.length) return "mappingDuplicate";
  return null;
}

const literal = (v: string) => `'${v.replace(/'/g, "''")}'`;
/** Nomes de opção são identificadores: só vão com aspas quando precisam. */
const optionKey = (k: string) => quoteIdent(k.trim());
const user = (u: string) => (u.trim().toLowerCase() === "public" ? "PUBLIC" : quoteIdent(u.trim()));

const optionList = (options: OptionPair[]) => options.map((o) => `${optionKey(o.key)} ${literal(o.value)}`).join(", ");

/** `ADD`, `SET` e `DROP` que levam as opções de `before` às de `after`; vazio se nada mudou. */
export function optionChanges(before: OptionPair[], after: OptionPair[]): string[] {
  const out: string[] = [];
  for (const o of after) {
    const old = before.find((b) => b.key === o.key);
    if (!old) out.push(`ADD ${optionKey(o.key)} ${literal(o.value)}`);
    else if (old.value !== o.value) out.push(`SET ${optionKey(o.key)} ${literal(o.value)}`);
  }
  for (const b of before) if (!after.some((o) => o.key === b.key)) out.push(`DROP ${optionKey(b.key)}`);
  return out;
}

/** `CREATE SERVER` com as opções, os mapeamentos de usuário e o comentário (também a prévia "Salvar como"). */
export function createServerStatements(s: ServerState): string[] {
  const name = quoteIdent(s.name.trim());
  const head = [`CREATE SERVER ${name}`];
  if (s.type.trim()) head.push(`TYPE ${literal(s.type.trim())}`);
  if (s.version.trim()) head.push(`VERSION ${literal(s.version.trim())}`);
  head.push(`FOREIGN DATA WRAPPER ${quoteIdent(s.wrapper.trim())}`);
  if (s.options.length) head.push(`OPTIONS (${optionList(s.options)})`);
  const out = [`${head.join("\n  ")};`];
  for (const m of s.mappings) {
    out.push(`CREATE USER MAPPING FOR ${user(m.user)} SERVER ${name}${m.options.length ? `\n  OPTIONS (${optionList(m.options)})` : ""};`);
  }
  if (s.owner.trim()) out.push(`ALTER SERVER ${name} OWNER TO ${quoteIdent(s.owner.trim())};`);
  if (s.comment.trim()) out.push(`COMMENT ON SERVER ${name} IS ${literal(s.comment)};`);
  return out;
}

/** Só o que mudou, na ordem que o servidor aceita: renomear primeiro, depois versão, opções, mapeamentos, dono e comentário. */
export function alterServerStatements(before: ServerState, after: ServerState): string[] {
  const out: string[] = [];
  let name = quoteIdent(before.name);
  if (after.name.trim() && after.name !== before.name) {
    out.push(`ALTER SERVER ${name} RENAME TO ${quoteIdent(after.name.trim())};`);
    name = quoteIdent(after.name.trim());
  }
  if (after.version !== before.version) out.push(`ALTER SERVER ${name} VERSION ${literal(after.version)};`);
  const opts = optionChanges(before.options, after.options);
  if (opts.length) out.push(`ALTER SERVER ${name} OPTIONS (${opts.join(", ")});`);
  const key = (u: string) => u.trim().toLowerCase();
  for (const m of before.mappings) {
    if (!after.mappings.some((a) => key(a.user) === key(m.user))) out.push(`DROP USER MAPPING FOR ${user(m.user)} SERVER ${name};`);
  }
  for (const m of after.mappings) {
    const old = before.mappings.find((b) => key(b.user) === key(m.user));
    if (!old) {
      out.push(`CREATE USER MAPPING FOR ${user(m.user)} SERVER ${name}${m.options.length ? `\n  OPTIONS (${optionList(m.options)})` : ""};`);
    } else {
      const changes = optionChanges(old.options, m.options);
      if (changes.length) out.push(`ALTER USER MAPPING FOR ${user(m.user)} SERVER ${name} OPTIONS (${changes.join(", ")});`);
    }
  }
  if (after.owner.trim() && after.owner !== before.owner) out.push(`ALTER SERVER ${name} OWNER TO ${quoteIdent(after.owner.trim())};`);
  if (after.comment !== before.comment) out.push(`COMMENT ON SERVER ${name} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}

/**
 * Excluir o servidor: antes saem os mapeamentos de usuário dele (são dependentes e o servidor recusa o `DROP` sem `CASCADE`),
 * mas sem `CASCADE`, para que uma tabela externa que usa o servidor continue impedindo a exclusão.
 */
export function dropServerStatements(name: string): string[] {
  const lit = literal(name);
  return [
    `DO $$ DECLARE m record; BEGIN FOR m IN SELECT usename FROM pg_catalog.pg_user_mappings WHERE srvname = ${lit} LOOP ` +
      `EXECUTE format('DROP USER MAPPING FOR %s SERVER %I', CASE WHEN m.usename = 'public' THEN 'PUBLIC' ELSE quote_ident(m.usename) END, ${lit}); END LOOP; END $$;`,
    `DROP SERVER ${quoteIdent(name)};`,
  ];
}

/** A opção guarda uma senha? (a tela mostra o valor mascarado). */
export const isSecretOption = (key: string): boolean => /pass(word)?|secret|token/i.test(key);
