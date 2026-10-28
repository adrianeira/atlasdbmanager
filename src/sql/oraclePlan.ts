import type { PlannedStatement } from "../backend";
import type { RowChanges } from "../live";
import type { CellValue, ColumnDef } from "../model";
import { quoteIdent } from "../model";

const typeOf = (c: ColumnDef) => (c.castType ?? c.type).toLowerCase();
const isBool = (c: ColumnDef) => /^bool/.test(typeOf(c));
/** LOBs e LONG não se comparam com `=`: ficam de fora da busca da linha. */
const isLob = (c: ColumnDef) => /(^|\s)(clob|nclob|blob|bfile|long)/.test(typeOf(c)) || typeOf(c) === "json" || typeOf(c) === "xmltype";

/**
 * A sessão do Atlas fixa `YYYY-MM-DD HH24:MI:SS` (data) e `YYYY-MM-DD HH24:MI:SS.FF6` (data e hora com fração); o campo da grade pode
 * trazer só a data, ou a hora sem fração. Completa o que falta para o servidor aceitar o texto sem `TO_DATE`.
 */
function stored(c: ColumnDef, v: CellValue): CellValue {
  if (v === null) return v;
  if (isBool(c)) return v === "t" || v === "true" || v === "1" ? "true" : "false";
  const t = typeOf(c);
  const text = v.trim().replace("T", " ");
  if (/^date\b/.test(t)) {
    if (/^\d{4}-\d{2}-\d{2}$/.test(text)) return `${text} 00:00:00`;
    return text.replace(/(\d{2}:\d{2}:\d{2})\.\d+$/, "$1");
  }
  if (/^timestamp/.test(t)) {
    let out = /^\d{4}-\d{2}-\d{2}$/.test(text) ? `${text} 00:00:00` : text;
    const m = /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})(\.\d+)?(\s*[+-]\d{2}:\d{2})?$/.exec(out);
    if (m) {
      out = `${m[1]}${m[2] ?? ".0"}`;
      if (/with(\s+local)?\s+time\s+zone/.test(t) && !/local/.test(t)) out += (m[3] ?? " +00:00").replace(/\s+/, " ");
    }
    return out;
  }
  return v;
}

/**
 * Os comandos de gravação da grade para o Oracle: parâmetros `:1`, `:2`... (o servidor converte o texto para o tipo da coluna) e a
 * tabela sempre com o schema (`"SCHEMA"."TABELA"`: a gravação não leva o schema à parte). Com chave primária, a linha é achada pela
 * chave (e, no UPDATE, pelo valor original das colunas alteradas, como no PostgreSQL). Sem chave, pelo conjunto de todos os valores;
 * linhas iguais marcadas juntas viram um comando só que mexe em exatamente quantas foram marcadas, escolhendo as linhas pelo `ROWID`
 * dentro do próprio comando.
 */
export function plannedStatementsOracle(c: RowChanges): PlannedStatement[] {
  const target = `${quoteIdent(c.schema)}.${quoteIdent(c.table)}`;
  const keys = c.columns.map((col, i) => ({ col, i })).filter((x) => x.col.key).sort((a, b) => (a.col.key ?? 0) - (b.col.key ?? 0));
  const keyless = keys.length === 0;
  const out: PlannedStatement[] = [];
  const loose: { head: string; where: string; params: (string | null)[] }[] = [];

  /** Acrescenta o valor aos parâmetros e devolve o marcador `:n` dele. */
  const bind = (params: (string | null)[], value: string | null) => {
    params.push(value);
    return `:${params.length}`;
  };

  const whereFor = (row: CellValue[], params: (string | null)[], extra: number[]) => {
    const conds: string[] = [];
    const add = (i: number) => {
      const col = c.columns[i];
      if (row[i] === null) conds.push(`${quoteIdent(col.name)} IS NULL`);
      else conds.push(`${quoteIdent(col.name)} = ${bind(params, stored(col, row[i]))}`);
    };
    if (keyless) c.columns.forEach((col, i) => { if (!isLob(col)) add(i); });
    keys.forEach(({ i }) => add(i));
    if (!keyless) extra.filter((i) => !c.columns[i].key && !isLob(c.columns[i])).forEach(add);
    return conds.join(" AND ");
  };

  const byRow = new Map<number, number[]>();
  for (const k of c.edited) {
    const [r, col] = k.split(":").map(Number);
    if (!c.deleted.has(r) && !c.newRows.has(r)) byRow.set(r, [...(byRow.get(r) ?? []), col]);
  }
  for (const [r, cols] of byRow) {
    const params: (string | null)[] = [];
    const set = cols.map((i) => `${quoteIdent(c.columns[i].name)} = ${bind(params, stored(c.columns[i], c.current[r][i]))}`);
    const head = `UPDATE ${target} SET ${set.join(", ")}`;
    if (keyless) loose.push({ head, where: whereFor(c.original[r], params, cols), params });
    else out.push({ sql: `${head} WHERE ${whereFor(c.original[r], params, cols)}`, params, expectOneRow: true });
  }
  for (const r of c.newRows) {
    const filled = c.columns.map((col, i) => ({ col, v: c.current[r][i] })).filter((x) => x.v !== null && !x.col.generated);
    if (!filled.length) {
      // `DEFAULT VALUES` só existe no Oracle 23ai: o `DEFAULT` na primeira coluna vale em todas as versões.
      out.push({ sql: `INSERT INTO ${target} (${quoteIdent(c.columns[0].name)}) VALUES (DEFAULT)`, params: [], expectOneRow: true });
      continue;
    }
    const params: (string | null)[] = [];
    const marks = filled.map((x) => bind(params, stored(x.col, x.v)));
    out.push({ sql: `INSERT INTO ${target} (${filled.map((x) => quoteIdent(x.col.name)).join(", ")}) VALUES (${marks.join(", ")})`, params, expectOneRow: true });
  }
  for (const r of c.deleted) {
    const params: (string | null)[] = [];
    const head = `DELETE FROM ${target}`;
    if (keyless) loose.push({ head, where: whereFor(c.original[r], params, []), params });
    else out.push({ sql: `${head} WHERE ${whereFor(c.original[r], params, [])}`, params, expectOneRow: true });
  }
  const groups = new Map<string, { st: (typeof loose)[number]; n: number }>();
  for (const st of loose) {
    const key = `${st.head} ${st.where} ${JSON.stringify(st.params)}`;
    const g = groups.get(key);
    if (g) g.n++;
    else groups.set(key, { st, n: 1 });
  }
  for (const { st, n } of groups.values()) {
    out.push({ sql: `${st.head} WHERE ROWID IN (SELECT ROWID FROM ${target} WHERE ${st.where} FETCH FIRST ${n} ROWS ONLY)`, params: st.params, expectOneRow: false, expectRows: n });
  }
  return out;
}
