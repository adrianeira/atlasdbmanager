import type { FunctionDetail } from "../backend";
import { quoteIdent } from "../model";

export type ParamMode = "IN" | "OUT" | "INOUT" | "VARIADIC";
export const PARAM_MODES: ParamMode[] = ["IN", "OUT", "INOUT", "VARIADIC"];

export interface FuncParam {
  mode: ParamMode;
  name: string;
  type: string;
  /** Expressão do `DEFAULT` (vazio = sem valor padrão). */
  default: string;
}

/** Estado editável de uma função ou procedimento no designer. */
export interface FuncState {
  name: string;
  params: FuncParam[];
  /** Tipo de retorno; vazio em procedimentos (e opcional quando há parâmetros OUT). */
  returns: string;
  language: string;
  volatility: "VOLATILE" | "STABLE" | "IMMUTABLE";
  strict: boolean;
  securityDefiner: boolean;
  owner: string;
  comment: string;
  body: string;
  procedure: boolean;
}

export const emptyFunc = (procedure = false): FuncState => ({
  name: "", params: [], returns: procedure ? "" : "void", language: "plpgsql", volatility: "VOLATILE",
  strict: false, securityDefiner: false, owner: "", comment: "", body: "", procedure,
});

/** Esqueleto do corpo para uma função nova, para o usuário já ter onde escrever. */
export const DEFAULT_PLPGSQL_BODY = "BEGIN\n  \nEND;";

const literal = (s: string) => `'${s.replace(/'/g, "''")}'`;

/** Divide por vírgulas fora de parênteses, colchetes e aspas (`numeric(10,2)`, `DEFAULT 'a,b'`). */
function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let quote: string | null = null;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (quote) {
      if (ch === quote) {
        if (text[i + 1] === quote) i++; // aspas duplicadas dentro do literal/identificador
        else quote = null;
      }
    } else if (ch === "'" || ch === '"') quote = ch;
    else if (ch === "(" || ch === "[") depth++;
    else if (ch === ")" || ch === "]") depth--;
    else if (ch === "," && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  if (text.slice(start).trim() !== "") parts.push(text.slice(start));
  return parts.map((p) => p.trim()).filter((p) => p !== "");
}

/** Posição do ` DEFAULT ` fora de aspas e parênteses, ou -1. */
function defaultIndex(text: string): number {
  let depth = 0;
  let quote: string | null = null;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (quote) {
      if (ch === quote) quote = null;
    } else if (ch === "'" || ch === '"') quote = ch;
    else if (ch === "(" || ch === "[") depth++;
    else if (ch === ")" || ch === "]") depth--;
    else if (depth === 0 && /^\sDEFAULT\s/i.test(text.slice(i, i + 9))) return i;
  }
  return -1;
}

/**
 * Lê `pg_get_function_arguments` (ex.: `a integer, OUT b text, c numeric(10,2) DEFAULT 1`). `names` diz, na mesma
 * ordem, quais argumentos têm nome: sem isso `integer` (sem nome) e `x integer` seriam indistinguíveis.
 */
export function parseArguments(args: string, names: string[]): FuncParam[] {
  return splitTopLevel(args).map((raw, i) => {
    let rest = raw;
    let mode: ParamMode = "IN";
    const m = /^(INOUT|OUT|IN|VARIADIC)\s+/i.exec(rest);
    if (m) {
      mode = m[1].toUpperCase() as ParamMode;
      rest = rest.slice(m[0].length);
    }
    let def = "";
    const d = defaultIndex(rest);
    if (d >= 0) {
      def = rest.slice(d).replace(/^\sDEFAULT\s/i, "").trim();
      rest = rest.slice(0, d);
    }
    rest = rest.trim();
    const name = names[i] ?? "";
    if (name) {
      for (const prefix of [`${quoteIdent(name)} `, `${name} `]) {
        if (rest.startsWith(prefix)) {
          rest = rest.slice(prefix.length);
          break;
        }
      }
    }
    return { mode, name, type: rest.trim(), default: def };
  });
}

export function fromDetail(d: FunctionDetail): FuncState {
  return {
    name: d.name,
    params: parseArguments(d.arguments, d.argNames),
    returns: d.returns ?? "",
    language: d.language,
    volatility: d.volatility === "immutable" ? "IMMUTABLE" : d.volatility === "stable" ? "STABLE" : "VOLATILE",
    strict: d.strict,
    securityDefiner: d.securityDefiner,
    owner: d.owner ?? "",
    comment: d.comment ?? "",
    body: d.body,
    procedure: d.procedure,
  };
}

const formatParam = (p: FuncParam, withDefault: boolean): string =>
  `${p.mode === "IN" ? "" : `${p.mode} `}${p.name ? `${quoteIdent(p.name)} ` : ""}${p.type.trim()}${withDefault && p.default.trim() ? ` DEFAULT ${p.default.trim()}` : ""}`;

/** Lista de argumentos como entra no `CREATE`. */
export const paramList = (params: FuncParam[]): string => params.map((p) => formatParam(p, true)).join(", ");

/** Lista que identifica a função (sem `DEFAULT`), para `DROP`/`ALTER`/`COMMENT`. */
export const identityList = (params: FuncParam[]): string => params.map((p) => formatParam(p, false)).join(", ");

/** `$body$...$body$`, com um rótulo que não aparece no corpo. */
export function dollarQuote(body: string): string {
  let tag = "$body$";
  for (let i = 1; body.includes(tag); i++) tag = `$body${i}$`;
  return `${tag}${body}${tag}`;
}

const kind = (s: FuncState) => (s.procedure ? "PROCEDURE" : "FUNCTION");
const target = (schema: string, name: string, s: FuncState, identity?: string) =>
  `${quoteIdent(schema)}.${quoteIdent(name)}(${identity ?? identityList(s.params)})`;

function defineSql(schema: string, s: FuncState, replace: boolean): string {
  const hasOut = s.params.some((p) => p.mode === "OUT" || p.mode === "INOUT");
  const returns = !s.procedure && s.returns.trim() && !(hasOut && !/^TABLE\s*\(/i.test(s.returns.trim()))
    ? `\n  RETURNS ${s.returns.trim()}` : "";
  const options = [
    `LANGUAGE ${s.language.trim() || "plpgsql"}`,
    ...(s.procedure ? [] : [s.volatility, ...(s.strict ? ["STRICT"] : [])]),
    ...(s.securityDefiner ? ["SECURITY DEFINER"] : []),
  ].join(" ");
  return `CREATE ${replace ? "OR REPLACE " : ""}${kind(s)} ${quoteIdent(schema)}.${quoteIdent(s.name)}(${paramList(s.params)})${returns}\n  ${options}\nAS ${dollarQuote(s.body)};`;
}

const ownerAndComment = (schema: string, s: FuncState, only?: { owner?: boolean; comment?: boolean }): string[] => {
  const t = target(schema, s.name, s);
  const out: string[] = [];
  if (s.owner && (!only || only.owner)) out.push(`ALTER ${kind(s)} ${t} OWNER TO ${quoteIdent(s.owner)};`);
  if (s.comment && (!only || only.comment)) out.push(`COMMENT ON ${kind(s)} ${t} IS ${literal(s.comment)};`);
  return out;
};

/** DDL de uma função nova (também a prévia "Salvar como"). */
export function createFuncStatements(schema: string, s: FuncState): string[] {
  return [defineSql(schema, s, false), ...ownerAndComment(schema, s)];
}

/**
 * Só o que mudou. Mudar argumentos (nome, tipo, modo, DEFAULT) ou o tipo de retorno não cabe num
 * `CREATE OR REPLACE` (o PostgreSQL recusa ou cria uma sobrecarga): vira `DROP` + `CREATE`. Fora isso, renomear
 * é `ALTER ... RENAME` e o resto (corpo, opções) é `CREATE OR REPLACE`.
 */
export function alterFuncStatements(schema: string, beforeIdentity: string, before: FuncState, after: FuncState): string[] {
  const out: string[] = [];
  const signatureChanged =
    paramList(before.params) !== paramList(after.params) ||
    (!after.procedure && before.returns.trim() !== after.returns.trim()) ||
    before.procedure !== after.procedure;
  if (signatureChanged) {
    out.push(`DROP ${kind(before)} ${target(schema, before.name, before, beforeIdentity)};`, defineSql(schema, after, false));
    out.push(...ownerAndComment(schema, after));
    return out;
  }
  let name = before.name;
  if (after.name !== before.name) {
    out.push(`ALTER ${kind(after)} ${target(schema, before.name, before, beforeIdentity)} RENAME TO ${quoteIdent(after.name)};`);
    name = after.name;
  }
  const bodyChanged =
    before.body !== after.body || before.language !== after.language || before.volatility !== after.volatility ||
    before.strict !== after.strict || before.securityDefiner !== after.securityDefiner;
  if (bodyChanged) out.push(defineSql(schema, { ...after, name }, true));
  const t = target(schema, name, after, name === before.name ? beforeIdentity : undefined);
  if (after.owner && after.owner !== before.owner) out.push(`ALTER ${kind(after)} ${t} OWNER TO ${quoteIdent(after.owner)};`);
  if (after.comment !== before.comment) out.push(`COMMENT ON ${kind(after)} ${t} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}

export const dropFuncSql = (schema: string, name: string, identity: string, procedure: boolean): string =>
  `DROP ${procedure ? "PROCEDURE" : "FUNCTION"} ${quoteIdent(schema)}.${quoteIdent(name)}(${identity});`;

/**
 * Chamada de teste. `values[i]` é o valor do i-ésimo parâmetro de entrada, em texto: vazio com `DEFAULT` = omitido;
 * vazio sem `DEFAULT` = NULL. Parâmetros com nome usam a notação nomeada (permite pular os do meio).
 */
export function callSql(schema: string, s: FuncState, values: string[]): string {
  const inputs = s.params.filter((p) => p.mode !== "OUT");
  const allNamed = inputs.every((p) => p.name);
  const args: string[] = [];
  inputs.forEach((p, i) => {
    const v = values[i] ?? "";
    if (v === "" && p.default.trim()) return;
    const lit = v === "" ? `NULL::${p.type}` : `${literal(v)}::${p.type}`;
    const value = p.mode === "VARIADIC" ? `VARIADIC ${lit}` : lit;
    args.push(allNamed && p.mode !== "VARIADIC" ? `${quoteIdent(p.name)} := ${value}` : value);
  });
  const callee = `${quoteIdent(schema)}.${quoteIdent(s.name)}(${args.join(", ")})`;
  return s.procedure ? `CALL ${callee};` : `SELECT * FROM ${callee};`;
}
