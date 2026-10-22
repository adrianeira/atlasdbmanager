/*
 * Integração da interface com o banco real: conversões e geração de comandos parametrizados.
 */
import type { ConstraintInfo, ObjectInfo, PlannedStatement, SchemaObjects, TableDetail } from "./backend";
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

export function schemaFromServer(name: string, objs: SchemaObjects): SchemaMock {
  return {
    name,
    loaded: true,
    objects: {
      table: objs.tables.map(objInfo),
      view: objs.views.map(objInfo),
      matview: objs.matviews.map(objInfo),
      function: objs.functions.map(objInfo),
      sequence: objs.sequences.map(objInfo),
      type: [],
      query: [],
      backup: [],
    },
  };
}

function parseTrigger(name: string, def: string, enabled: boolean): TriggerDef {
  const up = def.toUpperCase();
  const fires = up.includes(" INSTEAD OF ") ? "INSTEAD OF" : up.includes(" BEFORE ") ? "BEFORE" : "AFTER";
  const fn = /EXECUTE (?:FUNCTION|PROCEDURE) (.+?)\(/i.exec(def)?.[1] ?? "";
  return {
    name, fires, enable: enabled, fn,
    forEach: up.includes("FOR EACH ROW") ? "ROW" : "STATEMENT",
    insert: / INSERT( |$)/.test(up), update: / UPDATE( |$)/.test(up), delete: / DELETE( |$)/.test(up), truncate: / TRUNCATE( |$)/.test(up),
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
    triggers: d.triggers.map((t) => parseTrigger(t.name, t.definition, t.enabled)),
  };
}

/* ---------- Comandos parametrizados para a edição de dados ---------- */

// Tipos sem operador de igualdade: não entram na comparação otimista.
const NO_EQUALITY = new Set(["json", "xml", "point", "line", "lseg", "box", "path", "polygon", "circle"]);

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
  const cast = (col: ColumnDef) => col.castType ?? col.type;
  const out: PlannedStatement[] = [];

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
    keys.forEach(({ i }) => add(i));
    extra.filter((i) => !c.columns[i].key && !NO_EQUALITY.has(c.columns[i].type)).forEach(add);
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
    out.push({ sql: `UPDATE ${target} SET ${set.join(", ")} WHERE ${whereFor(c.original[r], params, cols)}`, params, expectOneRow: true });
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
    out.push({ sql: `DELETE FROM ${target} WHERE ${whereFor(c.original[r], params, [])}`, params, expectOneRow: true });
  }
  return out;
}
