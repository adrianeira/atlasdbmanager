import type { PlannedStatement } from "../backend";
import type { RowChanges } from "../live";
import type { CellValue, ColumnDef } from "../model";
import { quoteIdent } from "../model";

const isBool = (c: ColumnDef) => /bool/i.test(c.castType ?? c.type);
const isBlob = (c: ColumnDef) => /blob/i.test(c.castType ?? c.type);

/** A grade mostra booleano como `t`/`f`; o Firebird (3 em diante) recebe `true`/`false`. */
const stored = (c: ColumnDef, v: CellValue): CellValue => (v !== null && isBool(c) ? (v === "t" || v === "true" || v === "1" ? "true" : "false") : v);

/**
 * Os comandos de gravação da grade para o Firebird: parâmetros `?` (o servidor converte o texto para o tipo da coluna) e sem schema.
 * Com chave primária, a linha é achada pela chave (e, no UPDATE, pelo valor original das colunas alteradas, como no PostgreSQL).
 * Sem chave, pelo conjunto de todos os valores; linhas iguais marcadas juntas viram um comando só que mexe em exatamente quantas foram
 * marcadas, escolhendo as linhas pelo `RDB$DB_KEY` (o identificador interno da linha) dentro do próprio comando.
 */
export function plannedStatementsFirebird(c: RowChanges): PlannedStatement[] {
  const target = quoteIdent(c.table);
  const keys = c.columns.map((col, i) => ({ col, i })).filter((x) => x.col.key).sort((a, b) => (a.col.key ?? 0) - (b.col.key ?? 0));
  const keyless = keys.length === 0;
  const out: PlannedStatement[] = [];
  const loose: { head: string; where: string; params: (string | null)[] }[] = [];

  const whereFor = (row: CellValue[], params: (string | null)[], extra: number[]) => {
    const conds: string[] = [];
    const add = (i: number) => {
      const col = c.columns[i];
      if (row[i] === null) conds.push(`${quoteIdent(col.name)} IS NULL`);
      else {
        params.push(stored(col, row[i]));
        conds.push(`${quoteIdent(col.name)} = ?`);
      }
    };
    if (keyless) c.columns.forEach((col, i) => { if (!isBlob(col)) add(i); });
    keys.forEach(({ i }) => add(i));
    if (!keyless) extra.filter((i) => !c.columns[i].key && !isBlob(c.columns[i])).forEach(add);
    return conds.join(" AND ");
  };

  const byRow = new Map<number, number[]>();
  for (const k of c.edited) {
    const [r, col] = k.split(":").map(Number);
    if (!c.deleted.has(r) && !c.newRows.has(r)) byRow.set(r, [...(byRow.get(r) ?? []), col]);
  }
  for (const [r, cols] of byRow) {
    const params: (string | null)[] = [];
    const set = cols.map((i) => {
      params.push(stored(c.columns[i], c.current[r][i]));
      return `${quoteIdent(c.columns[i].name)} = ?`;
    });
    const head = `UPDATE ${target} SET ${set.join(", ")}`;
    if (keyless) loose.push({ head, where: whereFor(c.original[r], params, cols), params });
    else out.push({ sql: `${head} WHERE ${whereFor(c.original[r], params, cols)}`, params, expectOneRow: true });
  }
  for (const r of c.newRows) {
    const filled = c.columns.map((col, i) => ({ col, v: c.current[r][i] })).filter((x) => x.v !== null && !x.col.generated);
    if (!filled.length) {
      out.push({ sql: `INSERT INTO ${target} DEFAULT VALUES`, params: [], expectOneRow: true });
      continue;
    }
    out.push({
      sql: `INSERT INTO ${target} (${filled.map((x) => quoteIdent(x.col.name)).join(", ")}) VALUES (${filled.map(() => "?").join(", ")})`,
      params: filled.map((x) => stored(x.col, x.v)),
      expectOneRow: true,
    });
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
    out.push({ sql: `${st.head} WHERE RDB$DB_KEY IN (SELECT FIRST ${n} RDB$DB_KEY FROM ${target} WHERE ${st.where})`, params: st.params, expectOneRow: false, expectRows: n });
  }
  return out;
}
