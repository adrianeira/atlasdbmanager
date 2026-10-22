/**
 * "Beautify SQL" simples: palavras-chave em maiúsculas e quebra antes das cláusulas principais.
 * Respeita literais, identificadores entre aspas e comentários (não altera o que está dentro deles).
 * Não executa nada — só reformata texto.
 */
const KEYWORDS = new Set([
  "select", "from", "where", "and", "or", "not", "in", "is", "null", "as", "on", "join", "inner", "left", "right", "full", "outer",
  "cross", "group", "by", "order", "having", "limit", "offset", "insert", "into", "values", "update", "set", "delete", "create",
  "table", "alter", "drop", "index", "view", "distinct", "union", "all", "case", "when", "then", "else", "end", "asc", "desc",
  "like", "ilike", "between", "exists", "returning", "with", "primary", "key", "references", "default", "true", "false",
]);
const BREAK_BEFORE = /^(select|from|where|group|order|having|limit|offset|union|values|set|returning|inner|left|right|full|cross|join)$/i;

export function beautifySql(sqlText: string): string {
  const tokens = sqlText.match(/'(?:[^']|'')*'|"(?:[^"]|"")*"|--[^\n]*|\/\*[\s\S]*?\*\/|\$\w*\$[\s\S]*?\$\w*\$|\w+|\s+|./g) ?? [];
  let out = "";
  let prevWord = "";
  for (const tok of tokens) {
    if (/^\s+$/.test(tok)) {
      if (!out.endsWith("\n") && !out.endsWith(" ")) out += " ";
      continue;
    }
    const isWord = /^\w+$/.test(tok);
    const lower = tok.toLowerCase();
    const text = isWord && KEYWORDS.has(lower) ? tok.toUpperCase() : tok;
    // "LEFT JOIN", "GROUP BY": só a primeira palavra quebra a linha.
    const continues = (lower === "join" && /^(inner|left|right|full|cross|outer)$/.test(prevWord)) || (lower === "outer" && /^(left|right|full)$/.test(prevWord)) || (lower === "by" && /^(group|order)$/.test(prevWord));
    if (isWord && BREAK_BEFORE.test(lower) && !continues && out.trim()) {
      out = out.replace(/ +$/, "") + "\n";
    }
    out += text;
    if (isWord) prevWord = lower;
    if (tok === ";") out += "\n";
  }
  return out
    .split("\n")
    .map((line) => line.trimEnd().replace(/^ +/, ""))
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
