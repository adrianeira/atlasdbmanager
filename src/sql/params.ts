/** Parâmetros nomeados (`:nome`) de uma consulta do editor. */

export type ParamType = "text" | "number" | "null" | "raw";

export interface ParamValue {
  type: ParamType;
  value: string;
}

/** Textos, identificadores entre aspas, comentários e blocos `$$...$$` (onde um `:nome` não é parâmetro) ou o resto, símbolo a símbolo. */
const TOKENS = /'(?:[^']|'')*'|"(?:[^"]|"")*"|--[^\n]*|\/\*[\s\S]*?\*\/|\$\w*\$[\s\S]*?\$\w*\$|::|:[A-Za-z_]\w*|[\s\S]/g;

/** `:nome` que não é cast (`::tipo`) e não está colado numa palavra (`a:b`, `[1:2]`). Devolve os nomes na ordem, sem repetir. */
export function findParams(sql: string): string[] {
  const names: string[] = [];
  let prev = "";
  for (const tok of sql.match(TOKENS) ?? []) {
    if (tok.length > 1 && tok.startsWith(":") && tok !== "::" && !/\w/.test(prev) && !names.includes(tok.slice(1))) names.push(tok.slice(1));
    prev = tok[tok.length - 1] ?? "";
  }
  return names;
}

const NUMBER = /^-?\d+(\.\d+)?([eE][+-]?\d+)?$/;

/** Problema do valor informado (ou `null`): número que não é número. */
export function paramProblem(p: ParamValue): "number" | "raw" | null {
  if (p.type === "number" && !NUMBER.test(p.value.trim())) return "number";
  if (p.type === "raw" && !p.value.trim()) return "raw";
  return null;
}

/** O valor como literal SQL: texto entre aspas simples (com as aspas internas dobradas), número como está, `NULL` ou a expressão tal e qual. */
export function paramLiteral(p: ParamValue): string {
  switch (p.type) {
    case "null": return "NULL";
    case "number": return p.value.trim();
    case "raw": return p.value.trim();
    default: return `'${p.value.replace(/'/g, "''")}'`;
  }
}

/** Troca cada `:nome` encontrado pelo literal do valor; o resto do texto fica idêntico. */
export function substituteParams(sql: string, values: Record<string, ParamValue>): string {
  let out = "";
  let prev = "";
  for (const tok of sql.match(TOKENS) ?? []) {
    const name = tok.length > 1 && tok.startsWith(":") && tok !== "::" && !/\w/.test(prev) ? tok.slice(1) : null;
    out += name !== null && values[name] ? paramLiteral(values[name]) : tok;
    prev = tok[tok.length - 1] ?? "";
  }
  return out;
}
