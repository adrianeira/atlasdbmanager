/*
 * Integração da interface com o banco real: conversões e geração de comandos parametrizados.
 */
import type { ConstraintInfo, ObjectInfo, PlannedStatement, SchemaObjects, TableDetail, TriggerInfo } from "./backend";
import { quoteIdent, type CellValue, type ColumnDef, type DbObject, type SchemaMock, type TriggerDef } from "./model";

/**
 * Senhas da sessão, só em memória (somem ao fechar o app). Nunca vão para localStorage,
 * arquivo ou log. O cofre do sistema operacional substitui isto no próximo passo (SEG-001).
 */
export const sessionPasswords = new Map<string, string>();

const objInfo = (o: ObjectInfo): DbObject => ({
  name: o.name,
  rows: o.rows ?? undefined,
  comment: o.comment ?? undefined,
  owner: o.owner ?? undefined,
  args: o.args ?? undefined,
  returnType: o.returnType ?? undefined,
  language: o.language ?? undefined,
  dataType: o.dataType ?? undefined,
});

/** Função do servidor: a chave é "nome(argumentos)" para as sobrecargas não colidirem na árvore. */
const functionInfo = (o: ObjectInfo): DbObject => ({
  ...objInfo(o),
  name: `${o.name}(${o.args ?? ""})`,
  fnName: o.name,
  procedure: o.procedure ?? false,
});

export function schemaFromServer(name: string, objs: SchemaObjects): SchemaMock {
  return {
    name,
    loaded: true,
    objects: {
      table: objs.tables.map(objInfo),
      view: objs.views.map(objInfo),
      matview: objs.matviews.map(objInfo),
      function: objs.functions.map(functionInfo),
      sequence: objs.sequences.map(objInfo),
      type: objs.types.map(objInfo),
      extension: objs.extensions.map((e) => ({ name: e.name, version: e.version, extSchema: e.schema, owner: e.owner, comment: e.comment ?? undefined })),
      foreignServer: objs.foreignServers.map((f) => ({ name: f.name, wrapper: f.wrapper, owner: f.owner, comment: f.comment ?? undefined })),
      tablespace: objs.tablespaces.map((x) => ({ name: x.name, location: x.location, owner: x.owner, comment: x.comment ?? undefined })),
      query: [],
      backup: [],
    },
  };
}

/** Argumentos do gatilho como entram nos parênteses do `EXECUTE PROCEDURE`: cada um é um literal de texto. */
const triggerArgs = (args: string[]) => args.map((a) => `'${a.replace(/'/g, "''")}'`).join(", ");

function triggerFromServer(g: TriggerInfo): TriggerDef {
  return {
    name: g.name, fires: g.timing, enable: g.enabled, fn: g.function, forEach: g.forEachRow ? "ROW" : "STATEMENT",
    insert: g.events.includes("INSERT"), update: g.events.includes("UPDATE"), delete: g.events.includes("DELETE"), truncate: g.events.includes("TRUNCATE"),
    comment: g.comment ?? undefined, updateOf: g.updateColumns.join(", "), when: g.when ?? "", args: triggerArgs(g.args), readOnly: g.readOnly,
  };
}

/** Detalhe do servidor → modelo do designer/grid. `formattedType` vai em `castType` para os casts dos parâmetros. */
export function tableFromServer(base: DbObject, d: TableDetail): DbObject {
  const cons = (k: ConstraintInfo["kind"]) => d.constraints.filter((c) => c.kind === k);
  return {
    ...base,
    owner: d.owner ?? undefined,
    comment: d.comment ?? undefined,
    columns: d.columns.map<ColumnDef>((c) => ({
      name: c.name,
      type: c.dataType,
      castType: c.formattedType,
      length: c.length,
      decimal: c.decimal,
      notNull: c.notNull,
      key: c.key ?? undefined,
      default: c.default ?? undefined,
      comment: c.comment ?? undefined,
      collation: c.collation ?? undefined,
      identity: c.identity === "a" || c.identity === "d" ? c.identity : undefined,
      generated: c.generated === "s" ? "s" : undefined,
    })),
    indexes: d.indexes.map((i) => ({ name: i.name, fields: i.fields, method: i.method, unique: i.unique, concurrently: false, comment: i.comment ?? "" })),
    foreignKeys: cons("f").map((c) => ({
      name: c.name, fields: c.fields, refSchema: c.refSchema ?? "", refTable: c.refTable ?? "", refFields: c.refFields ?? "",
      onDelete: c.onDelete ?? "NO ACTION", onUpdate: c.onUpdate ?? "NO ACTION", comment: c.comment ?? "",
    })),
    uniques: cons("u").map((c) => ({ name: c.name, fields: c.fields, comment: c.comment ?? "" })),
    checks: cons("c").map((c) => ({
      name: c.name,
      expression: c.definition.replace(/^CHECK\s*\(\(?/i, "").replace(/\)?\)(\s+NO INHERIT)?$/i, ""),
      noInherit: /NO INHERIT/i.test(c.definition),
      comment: c.comment ?? "",
    })),
    triggers: d.triggers.map(triggerFromServer),
  };
}

/* ---------- Comandos parametrizados para a edição de dados ---------- */

// Tipos sem operador de igualdade: não entram na comparação otimista.
export const NO_EQUALITY = new Set(["json", "xml", "point", "line", "lseg", "box", "path", "polygon", "circle"]);

export interface RowChanges {
  schema: string;
  table: string;
  columns: ColumnDef[];
  original: CellValue[][];
  current: CellValue[][];
  edited: Set<string>;
  newRows: Set<number>;
  deleted: Set<number>;
}

/**
 * UPDATE/DELETE identificam a linha pela PK (spec §31.3) e, no UPDATE, comparam também o valor
 * original das colunas alteradas (concorrência otimista — ADR-027). Valores sempre como
 * parâmetros em texto com cast para o tipo da coluna; nada é concatenado no SQL.
 */
export function plannedStatements(c: RowChanges): PlannedStatement[] {
  const target = `${quoteIdent(c.schema)}.${quoteIdent(c.table)}`;
  const keys = c.columns.map((col, i) => ({ col, i })).filter((x) => x.col.key).sort((a, b) => (a.col.key ?? 0) - (b.col.key ?? 0));
  // Sem chave primária, a linha é achada por todos os valores (menos os de tipos sem igualdade). Linhas iguais marcadas juntas viram um
  // comando só que mexe em exatamente quantas foram marcadas (o banco escolhe quais, entre as iguais, pelo identificador interno da linha,
  // dentro do próprio comando).
  const keyless = keys.length === 0;
  const cast = (col: ColumnDef) => col.castType ?? col.type;
  const out: PlannedStatement[] = [];
  const loose: { head: string; where: string; params: (string | null)[] }[] = [];

  const whereFor = (row: CellValue[], params: (string | null)[], extra: number[]) => {
    const conds: string[] = [];
    const add = (i: number) => {
      const col = c.columns[i];
      if (row[i] === null) conds.push(`${quoteIdent(col.name)} IS NULL`);
      else {
        params.push(row[i]);
        conds.push(`${quoteIdent(col.name)} = $${params.length}::text::${cast(col)}`);
      }
    };
    if (keyless) c.columns.forEach((col, i) => { if (!NO_EQUALITY.has(col.type)) add(i); });
    keys.forEach(({ i }) => add(i));
    if (!keyless) extra.filter((i) => !c.columns[i].key && !NO_EQUALITY.has(c.columns[i].type)).forEach(add);
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
      params.push(c.current[r][i]);
      return `${quoteIdent(c.columns[i].name)} = $${params.length}::text::${cast(c.columns[i])}`;
    });
    if (keyless) loose.push({ head: `UPDATE ${target} SET ${set.join(", ")}`, where: whereFor(c.original[r], params, cols), params });
    else out.push({ sql: `UPDATE ${target} SET ${set.join(", ")} WHERE ${whereFor(c.original[r], params, cols)}`, params, expectOneRow: true });
  }
  for (const r of c.newRows) {
    const filled = c.columns.map((col, i) => ({ col, v: c.current[r][i] })).filter((x) => x.v !== null);
    if (!filled.length) {
      out.push({ sql: `INSERT INTO ${target} DEFAULT VALUES`, params: [], expectOneRow: true });
      continue;
    }
    const params = filled.map((x) => x.v);
    out.push({
      sql: `INSERT INTO ${target} (${filled.map((x) => quoteIdent(x.col.name)).join(", ")}) VALUES (${filled.map((x, i) => `$${i + 1}::text::${cast(x.col)}`).join(", ")})`,
      params,
      expectOneRow: true,
    });
  }
  for (const r of c.deleted) {
    const params: (string | null)[] = [];
    if (keyless) loose.push({ head: `DELETE FROM ${target}`, where: whereFor(c.original[r], params, []), params });
    else out.push({ sql: `DELETE FROM ${target} WHERE ${whereFor(c.original[r], params, [])}`, params, expectOneRow: true });
  }
  // Sem chave: comandos idênticos (linhas iguais marcadas juntas) viram um, com a contagem de linhas esperada.
  const groups = new Map<string, { st: { head: string; where: string; params: (string | null)[] }; n: number }>();
  for (const st of loose) {
    const key = `${st.head} ${st.where} ${JSON.stringify(st.params)}`;
    const g = groups.get(key);
    if (g) g.n++;
    else groups.set(key, { st, n: 1 });
  }
  for (const { st, n } of groups.values()) {
    out.push({ sql: `${st.head} WHERE ctid = ANY(ARRAY(SELECT ctid FROM ${target} WHERE ${st.where} LIMIT ${n}))`, params: st.params, expectOneRow: false, expectRows: n });
  }
  return out;
}
