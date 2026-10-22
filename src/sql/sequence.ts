import type { SequenceDetail } from "../backend";
import { quoteIdent } from "../model";

export type SeqType = "smallint" | "integer" | "bigint";
export const SEQ_TYPES: SeqType[] = ["smallint", "integer", "bigint"];

/** Estado editável de uma sequência. Os números ficam em texto (um bigint não cabe num número JS). */
export interface SeqState {
  name: string;
  dataType: SeqType;
  increment: string;
  /** Vazio = o padrão do servidor (só vale ao criar). */
  min: string;
  max: string;
  start: string;
  cache: string;
  cycle: boolean;
  /** `tabela.coluna` ou `schema.tabela.coluna`; vazio = sem dono. */
  ownedBy: string;
  owner: string;
  comment: string;
}

export const emptySeq = (): SeqState => ({
  name: "", dataType: "bigint", increment: "1", min: "", max: "", start: "", cache: "1", cycle: false, ownedBy: "", owner: "", comment: "",
});

export function fromDetail(d: SequenceDetail): SeqState {
  return {
    name: d.name, dataType: d.dataType, increment: d.increment, min: d.min, max: d.max, start: d.start, cache: d.cache,
    cycle: d.cycle, ownedBy: d.ownedBy ?? "", owner: d.owner ?? "", comment: d.comment ?? "",
  };
}

const literal = (s: string) => `'${s.replace(/'/g, "''")}'`;
const INTEGER = /^[+-]?\d+$/;
export const isInteger = (s: string) => INTEGER.test(s.trim());

/** Partes de `a.b.c` (com ou sem aspas), já sem as aspas. */
function nameParts(text: string): string[] {
  const parts: string[] = [];
  for (const m of text.trim().matchAll(/"((?:[^"]|"")+)"|([^."\s]+)/g)) parts.push(m[1] !== undefined ? m[1].replace(/""/g, '"') : m[2]);
  return parts;
}

/** `OWNED BY` completo (`"schema"."tabela"."coluna"`); `null` se o texto não tem 2 ou 3 partes. */
export function ownedByTarget(schema: string, text: string): string | null {
  const parts = nameParts(text);
  if (parts.length < 2 || parts.length > 3) return null;
  return (parts.length === 2 ? [schema, ...parts] : parts).map(quoteIdent).join(".");
}

/** Erro de validação (texto para o usuário) ou `null`. */
export function validateSeq(s: SeqState): "name" | "number" | "owned" | null {
  if (!s.name.trim()) return "name";
  for (const v of [s.increment, s.cache]) if (!isInteger(v)) return "number";
  for (const v of [s.min, s.max, s.start]) if (v.trim() !== "" && !isInteger(v)) return "number";
  if (BigInt(s.increment) === 0n) return "number";
  if (s.ownedBy.trim() && !ownedByTarget("x", s.ownedBy)) return "owned";
  return null;
}

const target = (schema: string, name: string) => `${quoteIdent(schema)}.${quoteIdent(name)}`;
const norm = (v: string) => (v.trim() === "" ? "" : BigInt(v.trim()).toString());

/** `AS` só existe a partir do PostgreSQL 10; antes a sequência é sempre bigint. */
function options(s: SeqState, versionNum: number, only?: (k: keyof SeqState) => boolean): string[] {
  const want = (k: keyof SeqState) => !only || only(k);
  const out: string[] = [];
  if (want("dataType") && versionNum >= 100000) out.push(`AS ${s.dataType}`);
  if (want("increment")) out.push(`INCREMENT BY ${norm(s.increment)}`);
  if (want("min")) out.push(s.min.trim() ? `MINVALUE ${norm(s.min)}` : "NO MINVALUE");
  if (want("max")) out.push(s.max.trim() ? `MAXVALUE ${norm(s.max)}` : "NO MAXVALUE");
  if (want("start") && s.start.trim()) out.push(`START WITH ${norm(s.start)}`);
  if (want("cache")) out.push(`CACHE ${norm(s.cache)}`);
  if (want("cycle")) out.push(s.cycle ? "CYCLE" : "NO CYCLE");
  return out;
}

/** DDL de uma sequência nova (também a prévia "Salvar como"). */
export function createSeqStatements(schema: string, s: SeqState, versionNum: number): string[] {
  const t = target(schema, s.name.trim());
  const opts = options(s, versionNum, (k) => k !== "min" && k !== "max" ? true : s[k as "min" | "max"].trim() !== "");
  const owned = s.ownedBy.trim() ? ownedByTarget(schema, s.ownedBy) : null;
  const out = [`CREATE SEQUENCE ${t}${opts.length ? `\n  ${opts.join("\n  ")}` : ""}${owned ? `\n  OWNED BY ${owned}` : ""};`];
  if (s.owner) out.push(`ALTER SEQUENCE ${t} OWNER TO ${quoteIdent(s.owner)};`);
  if (s.comment) out.push(`COMMENT ON SEQUENCE ${t} IS ${literal(s.comment)};`);
  return out;
}

/** Só o que mudou. O valor inicial (`START WITH`) muda sem mexer na posição atual: para isso há "Reiniciar". */
export function alterSeqStatements(schema: string, before: SeqState, after: SeqState, versionNum: number): string[] {
  const out: string[] = [];
  let name = before.name;
  if (after.name.trim() !== before.name) {
    out.push(`ALTER SEQUENCE ${target(schema, before.name)} RENAME TO ${quoteIdent(after.name.trim())};`);
    name = after.name.trim();
  }
  const t = target(schema, name);
  const changed = (k: keyof SeqState) => {
    if (k === "cycle") return before.cycle !== after.cycle;
    if (k === "dataType") return before.dataType !== after.dataType;
    const a = String(before[k]);
    const b = String(after[k]);
    return k === "increment" || k === "min" || k === "max" || k === "start" || k === "cache" ? norm(a) !== norm(b) : a !== b;
  };
  // Ao mudar o tipo o servidor recalcula os limites padrão: reenvia os dois para a regra ficar explícita.
  const typeChanged = changed("dataType") && versionNum >= 100000;
  const keys = (["dataType", "increment", "min", "max", "start", "cache", "cycle"] as (keyof SeqState)[]).filter(
    (k) => changed(k) || (typeChanged && (k === "min" || k === "max")),
  );
  const owned = (s: SeqState) => (s.ownedBy.trim() ? ownedByTarget(schema, s.ownedBy) ?? "" : "");
  const ownedChanged = owned(before) !== owned(after);
  const opts = options(after, versionNum, (k) => keys.includes(k));
  if (opts.length || ownedChanged) {
    const ownedClause = ownedChanged ? [`OWNED BY ${owned(after) || "NONE"}`] : [];
    out.push(`ALTER SEQUENCE ${t}\n  ${[...opts, ...ownedClause].join("\n  ")};`);
  }
  if (after.owner && after.owner !== before.owner) out.push(`ALTER SEQUENCE ${t} OWNER TO ${quoteIdent(after.owner)};`);
  if (after.comment !== before.comment) out.push(`COMMENT ON SEQUENCE ${t} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}

/** O próximo `nextval` devolve este valor. */
export function nextValue(d: { lastValue: string; isCalled: boolean }, increment: string): string {
  if (!isInteger(d.lastValue) || !isInteger(increment)) return d.lastValue;
  return d.isCalled ? (BigInt(d.lastValue) + BigInt(increment)).toString() : d.lastValue;
}

export const restartSeqSql = (schema: string, name: string, value: string): string =>
  `ALTER SEQUENCE ${target(schema, name)} RESTART${value.trim() ? ` WITH ${norm(value)}` : ""};`;

/** Define a posição atual: o próximo `nextval` devolve `value` + incremento (`is_called` verdadeiro). */
export const setValueSql = (schema: string, name: string, value: string): string =>
  `SELECT setval(${literal(target(schema, name))}, ${norm(value)}, true);`;

export const dropSeqSql = (schema: string, name: string): string => `DROP SEQUENCE ${target(schema, name)};`;
