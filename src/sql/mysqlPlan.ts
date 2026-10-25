import type { PlannedStatement } from "../backend";
import type { RowChanges } from "../live";
import type { CellValue, ColumnDef } from "../model";

/** Entre crases, como o MySQL quer os nomes. */
const q = (name: string) => `\`${name.replace(/`/g, "``")}\``;

const isBit = (c: ColumnDef) => /^bit$/i.test(c.type);
const isBinary = (c: ColumnDef) => /blob$|binary|^geometry|^point|^linestring|^polygon/i.test(c.type);
const noEquality = (c: ColumnDef) => isBinary(c) || /^(json|text|mediumtext|longtext|tinytext)$/i.test(c.type);

/** A grade mostra `BIT(1)` como `t`/`f`; o MySQL recebe 1/0. */
const stored = (c: ColumnDef, v: CellValue): CellValue => (v !== null && isBit(c) ? (v === "t" || v === "true" || v === "1" ? "1" : "0") : v);

/** O parâmetro de uma coluna: o `BIT` precisa de número, não de texto (o texto `'1'` seria o byte do caractere). */
const ph = (c: ColumnDef) => (isBit(c) ? "CAST(? AS UNSIGNED)" : "?");

/**
 * Os comandos de gravação da grade para o MySQL: parâmetros `?` (o servidor converte o texto para o tipo da coluna) e nomes
 * entre crases. Com chave primária, a linha é achada pela chave (e, no UPDATE, pelo valor original das colunas alteradas).
 * Sem chave, pelo conjunto de todos os valores; linhas iguais marcadas juntas viram um comando só com `LIMIT n`, que mexe em
 * exatamente quantas foram marcadas. No MySQL o schema é o próprio banco.
 */
export function plannedStatementsMysql(c: RowChanges): PlannedStatement[] {
  const target = `${q(c.schema)}.${q(c.table)}`;
  const keys = c.columns.map((col, i) => ({ col, i })).filter((x) => x.col.key).sort((a, b) => (a.col.key ?? 0) - (b.col.key ?? 0));
  const keyless = keys.length === 0;
  const out: PlannedStatement[] = [];
  const loose: { head: string; where: string; params: (string | null)[] }[] = [];

  const whereFor = (row: CellValue[], params: (string | null)[], extra: number[]) => {
    const conds: string[] = [];
    const add = (i: number) => {
      const col = c.columns[i];
      if (row[i] === null) conds.push(`${q(col.name)} IS NULL`);
      else {
        params.push(stored(col, row[i]));
        conds.push(`${q(col.name)} = ${ph(col)}`);
      }
    };
    if (keyless) c.columns.forEach((col, i) => { if (!noEquality(col) && !col.generated) add(i); });
    keys.forEach(({ i }) => add(i));
    if (!keyless) extra.filter((i) => !c.columns[i].key && !noEquality(c.columns[i])).forEach(add);
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
      return `${q(c.columns[i].name)} = ${ph(c.columns[i])}`;
    });
    const head = `UPDATE ${target} SET ${set.join(", ")}`;
    const where = whereFor(c.original[r], params, cols);
    if (keyless) loose.push({ head, where, params });
    else out.push({ sql: `${head} WHERE ${where}`, params, expectOneRow: true });
  }
  for (const r of c.newRows) {
    // Colunas calculadas o servidor preenche; `AUTO_INCREMENT` em branco também.
    const filled = c.columns.map((col, i) => ({ col, v: c.current[r][i] })).filter((x) => x.v !== null && !x.col.generated);
    if (!filled.length) {
      out.push({ sql: `INSERT INTO ${target} () VALUES ()`, params: [], expectOneRow: true });
      continue;
    }
    out.push({
      sql: `INSERT INTO ${target} (${filled.map((x) => q(x.col.name)).join(", ")}) VALUES (${filled.map((x) => ph(x.col)).join(", ")})`,
      params: filled.map((x) => stored(x.col, x.v)),
      expectOneRow: true,
    });
  }
  for (const r of c.deleted) {
    const params: (string | null)[] = [];
    const where = whereFor(c.original[r], params, []);
    if (keyless) loose.push({ head: `DELETE FROM ${target}`, where, params });
    else out.push({ sql: `DELETE FROM ${target} WHERE ${where}`, params, expectOneRow: true });
  }
  const groups = new Map<string, { st: (typeof loose)[number]; n: number }>();
  for (const st of loose) {
    const key = `${st.head} ${st.where} ${JSON.stringify(st.params)}`;
    const g = groups.get(key);
    if (g) g.n++;
    else groups.set(key, { st, n: 1 });
  }
  for (const { st, n } of groups.values()) out.push({ sql: `${st.head} WHERE ${st.where} LIMIT ${n}`, params: st.params, expectOneRow: false, expectRows: n });
  return out;
}
