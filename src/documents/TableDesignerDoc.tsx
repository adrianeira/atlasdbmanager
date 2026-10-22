import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { EditTable, type EditColumn } from "../components/EditTable";
import { MessageDialog } from "../components/MessageDialog";
import { PopupMenu } from "../components/PopupMenu";
import { SqlEditor } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon } from "../icons";
import { quoteIdent, type ColumnDef, type DbObject } from "../model";
import { columnType, createTableSql } from "../sql/ddl";

/* ---------- Estado de trabalho do designer ---------- */

type Row = Record<string, string | number | boolean>;

interface FieldRow extends ColumnDef {
  _id: number;
  /** Nome original (coluna existente). Ausente = coluna nova. */
  _orig?: string;
  dimensions: number;
}

interface DesignState {
  name: string;
  fields: FieldRow[];
  indexes: Row[];
  fks: Row[];
  uniques: Row[];
  checks: Row[];
  excludes: Row[];
  rules: Row[];
  triggers: Row[];
  unlogged: boolean;
  owner: string;
  tablespace: string;
  inherits: string;
  hasOids: boolean;
  fillFactor: string;
  cluster: string;
  comment: string;
}

const PG_TYPES = [
  "bool", "int2", "int4", "int8", "numeric", "float4", "float8", "money", "varchar", "bpchar", "text", "uuid",
  "date", "time", "timetz", "timestamp", "timestamptz", "interval", "json", "jsonb", "bytea", "inet", "cidr", "xml",
];
const FK_ACTIONS = ["NO ACTION", "RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT"];
const INDEX_METHODS = ["btree", "hash", "gist", "gin", "spgist", "brin"];

let nextId = 1;

function toState(t: DbObject | null): DesignState {
  return {
    name: t?.name ?? "",
    fields: (t?.columns ?? []).map((c) => ({ ...c, _id: nextId++, _orig: c.name, dimensions: 0 })),
    indexes: (t?.indexes ?? []).map((i) => ({ name: i.name, fields: i.fields, method: i.method, unique: i.unique, concurrently: i.concurrently, comment: i.comment ?? "", tablespace: "", fillFactor: "", where: "" })),
    fks: (t?.foreignKeys ?? []).map((f) => ({ name: f.name, fields: f.fields, refSchema: f.refSchema, refTable: f.refTable, refFields: f.refFields, onDelete: f.onDelete, onUpdate: f.onUpdate, comment: f.comment ?? "", matchFull: false, deferrable: "", deferred: "" })),
    uniques: (t?.uniques ?? []).map((u) => ({ name: u.name, fields: u.fields, comment: u.comment ?? "", tablespace: "", fillFactor: "", deferrable: "", deferred: "" })),
    checks: (t?.checks ?? []).map((c) => ({ name: c.name, expression: c.expression, noInherit: c.noInherit, comment: c.comment ?? "" })),
    excludes: [],
    rules: [],
    triggers: (t?.triggers ?? []).map((g) => ({ name: g.name, forEach: g.forEach, fires: g.fires, insert: g.insert, update: g.update, delete: g.delete, truncate: g.truncate, updateOf: "", enable: g.enable, comment: g.comment ?? "", when: "", fn: g.fn, args: "" })),
    unlogged: false,
    owner: t?.owner ?? "",
    tablespace: "",
    inherits: "",
    hasOids: false,
    fillFactor: "",
    cluster: "",
    comment: t?.comment ?? "",
  };
}

const s = (v: string | number | boolean | undefined) => String(v ?? "");

function toObject(d: DesignState): DbObject {
  return {
    name: d.name || "Untitled",
    owner: d.owner || undefined,
    comment: d.comment || undefined,
    columns: d.fields.map(({ _id: _i, _orig: _o, dimensions: _d, ...c }) => c),
    indexes: d.indexes.map((i) => ({ name: s(i.name), fields: s(i.fields), method: s(i.method) || "btree", unique: !!i.unique, concurrently: !!i.concurrently, comment: s(i.comment) })),
    foreignKeys: d.fks.map((f) => ({ name: s(f.name), fields: s(f.fields), refSchema: s(f.refSchema), refTable: s(f.refTable), refFields: s(f.refFields), onDelete: s(f.onDelete) || "NO ACTION", onUpdate: s(f.onUpdate) || "NO ACTION", comment: s(f.comment) })),
    uniques: d.uniques.map((u) => ({ name: s(u.name), fields: s(u.fields), comment: s(u.comment) })),
    checks: d.checks.map((c) => ({ name: s(c.name), expression: s(c.expression), noInherit: !!c.noInherit, comment: s(c.comment) })),
    triggers: d.triggers.map((g) => ({
      name: s(g.name), forEach: (s(g.forEach) || "ROW") as "ROW", fires: (s(g.fires) || "BEFORE") as "BEFORE",
      insert: !!g.insert, update: !!g.update, delete: !!g.delete, truncate: !!g.truncate, enable: !!g.enable, fn: s(g.fn), comment: s(g.comment),
    })),
  };
}

/* ---------- Geração do ALTER (prévia de "Save") ---------- */

interface AlterPlan { sql: string; destructive: boolean }

function alterSql(schema: string, original: DbObject, d: DesignState): AlterPlan {
  const target = `${quoteIdent(schema)}.${quoteIdent(original.name)}`;
  const out: string[] = [];
  let destructive = false;
  const origCols = original.columns ?? [];
  const keptOrig = new Set(d.fields.filter((f) => f._orig).map((f) => f._orig));

  for (const c of origCols) {
    if (!keptOrig.has(c.name)) {
      out.push(`ALTER TABLE ${target} DROP COLUMN ${quoteIdent(c.name)};`);
      destructive = true;
    }
  }
  for (const f of d.fields) {
    if (!f._orig) {
      let line = `ALTER TABLE ${target} ADD COLUMN ${quoteIdent(f.name)} ${columnType(f)}`;
      if (f.notNull) line += " NOT NULL";
      if (f.default) line += ` DEFAULT ${f.default}`;
      out.push(`${line};`);
      continue;
    }
    const o = origCols.find((c) => c.name === f._orig);
    if (!o) continue;
    if (o.name !== f.name) out.push(`ALTER TABLE ${target} RENAME COLUMN ${quoteIdent(o.name)} TO ${quoteIdent(f.name)};`);
    const col = quoteIdent(f.name);
    if (columnType(o) !== columnType(f)) {
      out.push(`ALTER TABLE ${target} ALTER COLUMN ${col} TYPE ${columnType(f)} USING ${col}::${columnType(f)};`);
      destructive = true;
    }
    if (o.notNull !== f.notNull) out.push(`ALTER TABLE ${target} ALTER COLUMN ${col} ${f.notNull ? "SET" : "DROP"} NOT NULL;`);
    if ((o.default ?? "") !== (f.default ?? "")) out.push(f.default ? `ALTER TABLE ${target} ALTER COLUMN ${col} SET DEFAULT ${f.default};` : `ALTER TABLE ${target} ALTER COLUMN ${col} DROP DEFAULT;`);
    if ((o.comment ?? "") !== (f.comment ?? "")) out.push(`COMMENT ON COLUMN ${target}.${col} IS ${f.comment ? `'${f.comment.replace(/'/g, "''")}'` : "NULL"};`);
  }

  const pkOf = (cols: ColumnDef[]) => cols.filter((c) => c.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0)).map((c) => c.name).join(",");
  const origPk = pkOf(origCols);
  const newPk = pkOf(d.fields);
  if (origPk !== newPk) {
    if (origPk) out.push(`ALTER TABLE ${target} DROP CONSTRAINT ${quoteIdent(`${original.name}_pkey`)};`);
    if (newPk) out.push(`ALTER TABLE ${target} ADD PRIMARY KEY (${newPk.split(",").map(quoteIdent).join(", ")});`);
  }

  // Constraints/índices: removidos ou alterados viram DROP + CREATE (comparação por nome).
  const next = toObject(d);
  const diffNamed = <T extends { name: string }>(before: T[], after: T[], drop: (x: T) => string, create: (x: T) => string) => {
    for (const b of before) {
      const a = after.find((x) => x.name === b.name);
      if (!a || JSON.stringify(a) !== JSON.stringify(b)) out.push(drop(b));
    }
    for (const a of after) {
      const b = before.find((x) => x.name === a.name);
      if (a.name && (!b || JSON.stringify(a) !== JSON.stringify(b))) out.push(create(a));
    }
  };
  const fieldList = (f: string) => f.split(",").map((x) => quoteIdent(x.trim())).join(", ");
  diffNamed(
    (original.indexes ?? []).map((i) => ({ ...i, comment: i.comment ?? "" })), next.indexes ?? [],
    (i) => `DROP INDEX ${quoteIdent(schema)}.${quoteIdent(i.name)};`,
    (i) => `CREATE ${i.unique ? "UNIQUE " : ""}INDEX ${i.concurrently ? "CONCURRENTLY " : ""}${quoteIdent(i.name)} ON ${target} USING ${i.method} (${fieldList(i.fields)});`,
  );
  diffNamed(
    (original.foreignKeys ?? []).map((f) => ({ ...f, comment: f.comment ?? "" })), next.foreignKeys ?? [],
    (f) => `ALTER TABLE ${target} DROP CONSTRAINT ${quoteIdent(f.name)};`,
    (f) => `ALTER TABLE ${target} ADD CONSTRAINT ${quoteIdent(f.name)} FOREIGN KEY (${fieldList(f.fields)}) REFERENCES ${quoteIdent(f.refSchema)}.${quoteIdent(f.refTable)} (${fieldList(f.refFields)}) ON DELETE ${f.onDelete} ON UPDATE ${f.onUpdate};`,
  );
  diffNamed(
    (original.uniques ?? []).map((u) => ({ ...u, comment: u.comment ?? "" })), next.uniques ?? [],
    (u) => `ALTER TABLE ${target} DROP CONSTRAINT ${quoteIdent(u.name)};`,
    (u) => `ALTER TABLE ${target} ADD CONSTRAINT ${quoteIdent(u.name)} UNIQUE (${fieldList(u.fields)});`,
  );
  diffNamed(
    (original.checks ?? []).map((c) => ({ ...c, comment: c.comment ?? "" })), next.checks ?? [],
    (c) => `ALTER TABLE ${target} DROP CONSTRAINT ${quoteIdent(c.name)};`,
    (c) => `ALTER TABLE ${target} ADD CONSTRAINT ${quoteIdent(c.name)} CHECK (${c.expression})${c.noInherit ? " NO INHERIT" : ""};`,
  );
  for (const e of d.excludes) if (s(e.name)) out.push(`ALTER TABLE ${target} ADD CONSTRAINT ${quoteIdent(s(e.name))} EXCLUDE USING ${s(e.method) || "gist"} (${s(e.element)});`);
  for (const r of d.rules) if (s(r.name)) out.push(`CREATE RULE ${quoteIdent(s(r.name))} AS ON ${s(r.event) || "INSERT"} TO ${target}${r.where ? ` WHERE ${s(r.where)}` : ""} DO ${r.instead === "INSTEAD" ? "INSTEAD " : ""}${s(r.definition) || "NOTHING"};`);

  if ((original.owner ?? "") !== d.owner && d.owner) out.push(`ALTER TABLE ${target} OWNER TO ${quoteIdent(d.owner)};`);
  if ((original.comment ?? "") !== d.comment) out.push(`COMMENT ON TABLE ${target} IS ${d.comment ? `'${d.comment.replace(/'/g, "''")}'` : "NULL"};`);
  return { sql: out.join("\n"), destructive };
}

/* ---------- Componente ---------- */

type SubTab = "fields" | "indexes" | "fks" | "uniques" | "checks" | "excludes" | "rules" | "triggers" | "options" | "comment" | "sql";

const SUBTABS: { id: SubTab; label: MessageKey }[] = [
  { id: "fields", label: "des.fields" },
  { id: "indexes", label: "des.indexes" },
  { id: "fks", label: "des.fks" },
  { id: "uniques", label: "des.uniques" },
  { id: "checks", label: "des.checks" },
  { id: "excludes", label: "des.excludes" },
  { id: "rules", label: "des.rules" },
  { id: "triggers", label: "des.triggers" },
  { id: "options", label: "des.options" },
  { id: "comment", label: "des.comment" },
  { id: "sql", label: "des.sqlPreview" },
];

interface Props {
  schema: string;
  /** null = nova tabela. */
  table: DbObject | null;
  owners: string[];
  otherTables: string[];
  serverVersionNum: number;
  active: boolean;
  production: boolean;
  onStatus: (cells: StatusCell[]) => void;
  onSaved: (table: DbObject, previousName: string | null) => void;
}

export function TableDesignerDoc({ schema, table, owners, otherTables, serverVersionNum, active, production, onStatus, onSaved }: Props) {
  const t = useT();
  const [original, setOriginal] = useState<DbObject | null>(table);
  const [d, setD] = useState<DesignState>(() => toState(table));
  const [tab, setTab] = useState<SubTab>("fields");
  const [sel, setSel] = useState<Record<string, number>>({});
  const [pkMenu, setPkMenu] = useState(false);
  const [previewMode, setPreviewMode] = useState<"save" | "saveAs">(table ? "save" : "saveAs");
  const [dialog, setDialog] = useState<null | { kind: "name" } | { kind: "review"; sql: string; destructive: boolean; newName?: string }>(null);
  const [triggerTab, setTriggerTab] = useState<"general" | "constraint">("general");
  const pkRef = useRef<HTMLDivElement>(null);

  const patch = (p: Partial<DesignState>) => setD((x) => ({ ...x, ...p }));
  const selected = sel[tab] ?? 0;
  const setSelected = (i: number) => setSel((m) => ({ ...m, [tab]: i }));

  const isNew = !original;
  const plan = useMemo(() => (original ? alterSql(schema, original, d) : null), [original, d, schema]);
  const dirty = isNew ? d.fields.length > 0 : !!plan?.sql;
  const createSql = useMemo(() => createTableSql(schema, toObject(d), isNew ? "Untitled" : undefined), [schema, d, isNew]);

  useEffect(() => {
    if (!pkMenu) return;
    const close = (e: MouseEvent) => !pkRef.current?.contains(e.target as Node) && setPkMenu(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [pkMenu]);

  const countLabel: Partial<Record<SubTab, [MessageKey, number]>> = {
    indexes: ["des.countIndexes", d.indexes.length],
    fks: ["des.countFks", d.fks.length],
    uniques: ["des.countUniques", d.uniques.length],
    checks: ["des.countChecks", d.checks.length],
    excludes: ["des.countExcludes", d.excludes.length],
    rules: ["des.countRules", d.rules.length],
    triggers: ["des.countTriggers", d.triggers.length],
  };

  useEffect(() => {
    if (!active) return;
    const cells: StatusCell[] = [{ text: t("des.countFields", { n: d.fields.length }), width: 200 }];
    const extra = countLabel[tab];
    if (extra) cells.push({ text: t(extra[0], { n: extra[1] }), width: 200 });
    if (dirty) cells.push({ text: t("des.unsaved"), grow: true });
    else cells.push({ text: "", grow: true });
    onStatus(cells);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, d, tab, dirty, t, onStatus]);

  /* ----- Ações por sub-aba ----- */
  type ListKey = "indexes" | "fks" | "uniques" | "checks" | "excludes" | "rules" | "triggers";
  const blank: Record<ListKey, Row> = {
    indexes: { name: "", fields: "", method: "btree", unique: false, concurrently: false, comment: "", tablespace: "", fillFactor: "", where: "" },
    fks: { name: "", fields: "", refSchema: schema, refTable: "", refFields: "", onDelete: "NO ACTION", onUpdate: "NO ACTION", comment: "", matchFull: false, deferrable: "", deferred: "" },
    uniques: { name: "", fields: "", comment: "", tablespace: "", fillFactor: "", deferrable: "", deferred: "" },
    checks: { name: "", expression: "", noInherit: false, comment: "" },
    excludes: { name: "", method: "gist", element: "", comment: "", tablespace: "", fillFactor: "", where: "" },
    rules: { name: "", event: "INSERT", instead: "ALSO", enable: true, comment: "", where: "", definition: "" },
    triggers: { name: "", forEach: "ROW", fires: "BEFORE", insert: false, update: false, delete: false, truncate: false, updateOf: "", enable: true, comment: "", when: "", fn: "", args: "" },
  };

  const addItem = () => {
    if (tab === "fields") {
      const f: FieldRow = { _id: nextId++, name: "", type: "", length: 0, decimal: 0, notNull: false, dimensions: 0 };
      patch({ fields: [...d.fields, f] });
      setSelected(d.fields.length);
      return;
    }
    const k = tab as ListKey;
    patch({ [k]: [...d[k], { ...blank[k] }] } as Partial<DesignState>);
    setSelected(d[k].length);
  };

  const deleteItem = () => {
    if (tab === "fields") {
      if (!d.fields.length) return;
      patch({ fields: d.fields.filter((_, i) => i !== selected) });
      setSelected(Math.max(0, selected - 1));
      return;
    }
    const k = tab as ListKey;
    if (!d[k].length) return;
    patch({ [k]: d[k].filter((_, i) => i !== selected) } as Partial<DesignState>);
    setSelected(Math.max(0, selected - 1));
  };

  const togglePk = () => {
    const f = d.fields[selected];
    if (!f) return;
    const maxKey = Math.max(0, ...d.fields.map((x) => x.key ?? 0));
    const fields = d.fields.map((x, i) => {
      if (i !== selected) return x;
      return x.key ? { ...x, key: undefined } : { ...x, key: maxKey + 1, notNull: true };
    });
    // Renumera a ordem da chave após remover um campo da PK.
    const ordered = fields.filter((x) => x.key).sort((a, b) => (a.key ?? 0) - (b.key ?? 0));
    patch({ fields: fields.map((x) => (x.key ? { ...x, key: ordered.indexOf(x) + 1 } : x)) });
  };

  const save = () => {
    if (isNew) {
      if (!d.fields.length) return;
      setDialog({ kind: "name" });
      return;
    }
    if (!plan?.sql) return;
    setDialog({ kind: "review", sql: plan.sql, destructive: plan.destructive });
  };

  const commit = (newName?: string) => {
    const obj = toObject(newName ? { ...d, name: newName } : d);
    const previous = original?.name ?? null;
    onSaved(obj, previous);
    setOriginal(obj);
    setD(toState(obj));
    setPreviewMode("save");
    setDialog(null);
  };

  const listTabs: Record<string, { add: MessageKey; del: MessageKey }> = {
    fields: { add: "des.addField", del: "des.deleteField" },
    indexes: { add: "des.addIndex", del: "des.deleteIndex" },
    fks: { add: "des.addFk", del: "des.deleteFk" },
    uniques: { add: "des.addUnique", del: "des.deleteUnique" },
    checks: { add: "des.addCheck", del: "des.deleteCheck" },
    excludes: { add: "des.addExclude", del: "des.deleteExclude" },
    rules: { add: "des.addRule", del: "des.deleteRule" },
    triggers: { add: "des.addTrigger", del: "des.deleteTrigger" },
  };

  /* ----- Grades ----- */
  const colsFor = (k: SubTab): EditColumn<Row>[] => {
    switch (k) {
      case "indexes":
        return [
          { key: "name", label: t("col.name"), width: 150, kind: "text" },
          { key: "fields", label: t("des.fieldsCol"), width: 320, kind: "select", free: true, options: d.fields.map((f) => f.name) },
          { key: "method", label: t("des.indexMethod"), width: 100, kind: "select", options: INDEX_METHODS },
          { key: "unique", label: t("des.unique"), width: 60, kind: "check" },
          { key: "concurrently", label: t("des.concurrently"), width: 80, kind: "check" },
          { key: "comment", label: t("col.comment"), width: 300, kind: "text" },
        ];
      case "fks":
        return [
          { key: "name", label: t("col.name"), width: 90, kind: "text" },
          { key: "fields", label: t("des.fieldsCol"), width: 130, kind: "select", free: true, options: d.fields.map((f) => f.name) },
          { key: "refSchema", label: t("des.refSchema"), width: 130, kind: "text" },
          { key: "refTable", label: t("des.refTable"), width: 130, kind: "select", free: true, options: otherTables },
          { key: "refFields", label: t("des.refFields"), width: 130, kind: "text" },
          { key: "onDelete", label: t("des.onDelete"), width: 80, kind: "select", options: FK_ACTIONS },
          { key: "onUpdate", label: t("des.onUpdate"), width: 80, kind: "select", options: FK_ACTIONS },
          { key: "comment", label: t("col.comment"), width: 300, kind: "text" },
        ];
      case "uniques":
        return [
          { key: "name", label: t("col.name"), width: 150, kind: "text" },
          { key: "fields", label: t("des.fieldsCol"), width: 400, kind: "select", free: true, options: d.fields.map((f) => f.name) },
          { key: "comment", label: t("col.comment"), width: 300, kind: "text" },
        ];
      case "checks":
        return [
          { key: "name", label: t("col.name"), width: 150, kind: "text" },
          { key: "expression", label: t("des.expression"), width: 400, kind: "text" },
          { key: "noInherit", label: t("des.noInherit"), width: 80, kind: "check" },
          { key: "comment", label: t("col.comment"), width: 300, kind: "text" },
        ];
      case "excludes":
        return [
          { key: "name", label: t("col.name"), width: 150, kind: "text" },
          { key: "method", label: t("des.indexMethod"), width: 100, kind: "select", options: INDEX_METHODS },
          { key: "element", label: t("des.element"), width: 400, kind: "text" },
          { key: "comment", label: t("col.comment"), width: 300, kind: "text" },
        ];
      case "rules":
        return [
          { key: "name", label: t("col.name"), width: 250, kind: "text" },
          { key: "event", label: t("des.event"), width: 80, kind: "select", options: ["SELECT", "INSERT", "UPDATE", "DELETE"] },
          { key: "instead", label: t("des.doInstead"), width: 80, kind: "select", options: ["ALSO", "INSTEAD"] },
          { key: "enable", label: t("des.enable"), width: 50, kind: "check" },
          { key: "comment", label: t("col.comment"), width: 300, kind: "text" },
        ];
      case "triggers":
        return [
          { key: "name", label: t("col.name"), width: 120, kind: "text" },
          { key: "forEach", label: t("des.forEach"), width: 100, kind: "select", options: ["ROW", "STATEMENT"] },
          { key: "fires", label: t("des.fires"), width: 70, kind: "select", options: ["BEFORE", "AFTER", "INSTEAD OF"] },
          { key: "insert", label: "Insert", width: 60, kind: "check" },
          { key: "update", label: "Update", width: 60, kind: "check" },
          { key: "delete", label: "Delete", width: 60, kind: "check" },
          { key: "truncate", label: "Truncate", width: 60, kind: "check" },
          { key: "updateOf", label: t("des.updateOf"), width: 95, kind: "text" },
          { key: "enable", label: t("des.enable"), width: 50, kind: "check" },
          { key: "comment", label: t("col.comment"), width: 300, kind: "text" },
        ];
      default:
        return [];
    }
  };

  const fieldCols: EditColumn<FieldRow>[] = [
    { key: "name", label: t("col.name"), width: 200, kind: "text" },
    { key: "type", label: t("des.type"), width: 120, kind: "select", free: true, options: PG_TYPES },
    { key: "length", label: t("des.length"), width: 60, kind: "number" },
    { key: "decimal", label: t("des.decimal"), width: 60, kind: "number" },
    { key: "notNull", label: t("des.notNull"), width: 60, kind: "check" },
    { key: "key", label: t("des.key"), width: 60, kind: "key" },
    { key: "comment", label: t("col.comment"), width: 300, kind: "text" },
  ];

  const changeList = (k: ListKey) => (i: number, key: string, v: string | number | boolean) =>
    patch({ [k]: d[k].map((r, j) => (j === i ? { ...r, [key]: v } : r)) } as Partial<DesignState>);

  /* ----- Painel de propriedades inferior ----- */
  const prop = (label: MessageKey, control: ReactNode, disabled = false) => (
    <div className={`prop-row${disabled ? " is-disabled" : ""}`}>
      <label>{t(label)}:</label>
      {control}
    </div>
  );
  const listRow = (k: ListKey) => d[k][selected];
  const propInput = (k: ListKey, key: string, disabled = false, browse = false) => {
    const r = listRow(k);
    const input = <input className="field prop-field" disabled={disabled || !r} value={s(r?.[key])} onChange={(e) => changeList(k)(selected, key, e.target.value)} />;
    return browse ? <div className="field-with-button prop-field-wrap">{input}<button type="button" className="btn btn-ellipsis" disabled>…</button></div> : input;
  };
  const propSelect = (k: ListKey, key: string, options: string[], disabled = false) => {
    const r = listRow(k);
    return (
      <select className="field prop-field" disabled={disabled || !r} value={s(r?.[key])} onChange={(e) => changeList(k)(selected, key, e.target.value)}>
        <option value="" />
        {options.map((o) => <option key={o}>{o}</option>)}
      </select>
    );
  };

  const field = d.fields[selected];
  const isTextType = !!field && /(char|text)/.test(field.type);
  const bottom: Partial<Record<SubTab, ReactNode>> = {
    fields: field && (
      <>
        {prop("des.default", (
          <>
            <input className="field prop-field" list="des-defaults" value={field.default ?? ""} onChange={(e) => patch({ fields: d.fields.map((f, i) => (i === selected ? { ...f, default: e.target.value || undefined } : f)) })} spellCheck={false} />
            <datalist id="des-defaults">{["NULL", "now()", "CURRENT_TIMESTAMP", "gen_random_uuid()", "''::text", "0", "true", "false"].map((o) => <option key={o} value={o} />)}</datalist>
          </>
        ))}
        {prop("des.collation", (
          <span className="prop-pair">
            <select className="field" disabled={!isTextType} value={isTextType ? "pg_catalog" : ""} onChange={() => undefined}>
              <option value="" />
              <option value="pg_catalog">pg_catalog</option>
            </select>
            <select className="field" disabled={!isTextType} value={isTextType ? "default" : ""} onChange={() => undefined}>
              <option value="" />
              <option value="default">default</option>
              <option value="C">C</option>
              <option value="POSIX">POSIX</option>
            </select>
          </span>
        ), !isTextType)}
        {prop("des.dimensions", <input className="field prop-field is-num" value={field.dimensions} onChange={(e) => patch({ fields: d.fields.map((f, i) => (i === selected ? { ...f, dimensions: Number(e.target.value) || 0 } : f)) })} />)}
      </>
    ),
    indexes: (
      <>
        {prop("des.tablespace", propSelect("indexes", "tablespace", ["pg_default"]), !listRow("indexes"))}
        {prop("des.fillFactor", propInput("indexes", "fillFactor"), !listRow("indexes"))}
        {prop("des.buffering", propSelect("indexes", "buffering", ["auto", "on", "off"], true), true)}
        {prop("des.fastUpdate", propSelect("indexes", "fastUpdate", ["on", "off"], true), true)}
        {prop("des.pendingList", propInput("indexes", "pendingList", true), true)}
        {prop("des.pagesPerRange", propInput("indexes", "pagesPerRange", true), true)}
        {prop("des.constraint", propInput("indexes", "where", false, true), !listRow("indexes"))}
      </>
    ),
    fks: (
      <>
        <label className={`check-row${listRow("fks") ? "" : " is-disabled"}`}>
          <input type="checkbox" disabled={!listRow("fks")} checked={!!listRow("fks")?.matchFull} onChange={(e) => changeList("fks")(selected, "matchFull", e.target.checked)} />
          <span>{t("des.matchFull")}</span>
        </label>
        {prop("des.deferrable", propSelect("fks", "deferrable", ["DEFERRABLE", "NOT DEFERRABLE"]), !listRow("fks"))}
        {prop("des.deferred", propSelect("fks", "deferred", ["INITIALLY DEFERRED", "INITIALLY IMMEDIATE"]), !listRow("fks"))}
      </>
    ),
    uniques: (
      <>
        {prop("des.tablespace", propSelect("uniques", "tablespace", ["pg_default"]), !listRow("uniques"))}
        {prop("des.fillFactor", propInput("uniques", "fillFactor"), !listRow("uniques"))}
        {prop("des.deferrable", propSelect("uniques", "deferrable", ["DEFERRABLE", "NOT DEFERRABLE"]), !listRow("uniques"))}
        {prop("des.deferred", propSelect("uniques", "deferred", ["INITIALLY DEFERRED", "INITIALLY IMMEDIATE"]), !listRow("uniques"))}
      </>
    ),
    excludes: (
      <>
        {prop("des.tablespace", propSelect("excludes", "tablespace", ["pg_default"]), !listRow("excludes"))}
        {prop("des.fillFactor", propInput("excludes", "fillFactor"), !listRow("excludes"))}
        {prop("des.buffering", propSelect("excludes", "buffering", ["auto", "on", "off"]), !listRow("excludes"))}
        {prop("des.where", propInput("excludes", "where", false, true), !listRow("excludes"))}
        {prop("des.deferrable", propSelect("excludes", "deferrable", ["DEFERRABLE", "NOT DEFERRABLE"], true), true)}
        {prop("des.deferred", propSelect("excludes", "deferred", ["INITIALLY DEFERRED", "INITIALLY IMMEDIATE"], true), true)}
      </>
    ),
    rules: (
      <>
        {prop("des.where", propInput("rules", "where", false, true), !listRow("rules"))}
        {prop("des.definition", propInput("rules", "definition", false, true), !listRow("rules"))}
      </>
    ),
    triggers: (
      <>
        <div className="dlg-tabs dlg-tabs-small">
          <button type="button" className={`dlg-tab${triggerTab === "general" ? " is-active" : ""}`} onClick={() => setTriggerTab("general")}>{t("conn.tab.general")}</button>
          <button type="button" className={`dlg-tab${triggerTab === "constraint" ? " is-active" : ""}`} onClick={() => setTriggerTab("constraint")}>{t("des.constraintTab")}</button>
        </div>
        {triggerTab === "general" ? (
          <>
            {prop("des.when", propInput("triggers", "when"), !listRow("triggers"))}
            {prop("des.triggerFunction", (
              <span className="prop-pair">
                <select className="field" disabled={!listRow("triggers")} value={schema} onChange={() => undefined}><option>{schema}</option></select>
                {propSelect("triggers", "fn", ["public.fn_audit_trigger"])}
              </span>
            ), !listRow("triggers"))}
            {prop("des.arguments", propInput("triggers", "args"), !listRow("triggers"))}
          </>
        ) : (
          <>
            {prop("des.deferrable", propSelect("triggers", "deferrable", ["DEFERRABLE", "NOT DEFERRABLE"], true), true)}
            {prop("des.deferred", propSelect("triggers", "deferred", ["INITIALLY DEFERRED", "INITIALLY IMMEDIATE"], true), true)}
          </>
        )}
      </>
    ),
  };

  /* ----- Render ----- */
  const toolBtn = (icon: Parameters<typeof SmallIcon>[0]["name"], label: MessageKey, onClick: (() => void) | undefined) => (
    <button type="button" className="ot-button" disabled={!onClick} onClick={onClick}>
      <SmallIcon name={icon} />
      <span>{t(label)}</span>
    </button>
  );

  const body = () => {
    if (tab === "fields") {
      return (
        <EditTable<FieldRow>
          columns={fieldCols}
          rows={d.fields}
          selected={selected}
          onSelect={setSelected}
          onChange={(i, key, v) => patch({ fields: d.fields.map((f, j) => (j === i ? { ...f, [key]: v } : f)) })}
        />
      );
    }
    if (tab === "options") {
      return (
        <div className="options-form">
          <label className="check-row"><input type="checkbox" checked={d.unlogged} onChange={(e) => patch({ unlogged: e.target.checked })} /><span>{t("des.unlogged")}</span></label>
          {prop("des.owner", <select className="field prop-field" value={d.owner} onChange={(e) => patch({ owner: e.target.value })}><option value="" />{owners.map((o) => <option key={o}>{o}</option>)}</select>)}
          {prop("des.tablespace", <select className="field prop-field" value={d.tablespace} onChange={(e) => patch({ tablespace: e.target.value })}><option value="" /><option>pg_default</option></select>)}
          {prop("des.inherits", <div className="field-with-button prop-field-wrap"><input className="field prop-field" value={d.inherits} onChange={(e) => patch({ inherits: e.target.value })} /><button type="button" className="btn btn-ellipsis" disabled>…</button></div>)}
          {/* WITH OIDS existe só até o PostgreSQL 11. */}
          <label className={`check-row${serverVersionNum >= 120000 ? " is-disabled" : ""}`}><input type="checkbox" disabled={serverVersionNum >= 120000} checked={d.hasOids} onChange={(e) => patch({ hasOids: e.target.checked })} /><span>{t("des.hasOids")}</span></label>
          {prop("des.fillFactor", <input className="field prop-field" value={d.fillFactor} onChange={(e) => patch({ fillFactor: e.target.value })} />)}
          {prop("des.cluster", <select className="field prop-field" value={d.cluster} onChange={(e) => patch({ cluster: e.target.value })}><option value="" />{d.indexes.map((i) => <option key={s(i.name)}>{s(i.name)}</option>)}</select>)}
        </div>
      );
    }
    if (tab === "comment") return <textarea className="comment-area" value={d.comment} onChange={(e) => patch({ comment: e.target.value })} aria-label={t("des.comment")} />;
    if (tab === "sql") {
      const text = previewMode === "saveAs" ? createSql : plan?.sql || t("des.noChanges");
      return <SqlEditor value={text} readOnly />;
    }
    const k = tab as ListKey;
    return <EditTable<Row> columns={colsFor(k)} rows={d[k]} selected={selected} onSelect={setSelected} onChange={changeList(k)} />;
  };

  const actions = listTabs[tab];

  return (
    <div className="doc">
      <div className="object-toolbar" role="toolbar">
        {toolBtn("save", "des.save", dirty ? save : undefined)}
        {actions && (
          <>
            {toolBtn("new", actions.add, addItem)}
            {toolBtn("delete", actions.del, (tab === "fields" ? d.fields.length : d[tab as ListKey].length) ? deleteItem : undefined)}
          </>
        )}
        {tab === "fields" && (
          <div className="split-button" ref={pkRef}>
            <button type="button" className="ot-button" disabled={!field} onClick={togglePk}>
              <SmallIcon name="key" />
              <span>{t("des.primaryKey")}</span>
            </button>
            <button type="button" className="ot-button split-arrow" onClick={() => setPkMenu((o) => !o)} aria-label={t("des.primaryKey")}>
              <SmallIcon name="chevronDown" size={10} />
            </button>
            {pkMenu && (
              <PopupMenu
                commands={{}}
                onClose={() => setPkMenu(false)}
                items={[
                  { kind: "item", text: t("des.primaryKey"), icon: "key", run: field ? togglePk : undefined },
                  { kind: "item", text: t("des.pkProperties") },
                ]}
              />
            )}
          </div>
        )}
        <div className="ot-spacer" />
        <button type="button" className="icon-button" title={t("notImplemented")} disabled><SmallIcon name="fullscreen" /></button>
      </div>

      <div className="dlg-tabs des-tabs" role="tablist">
        {SUBTABS.map((x) => (
          <button key={x.id} type="button" role="tab" aria-selected={tab === x.id} className={`dlg-tab${tab === x.id ? " is-active" : ""}`} onClick={() => setTab(x.id)}>
            {t(x.label)}
          </button>
        ))}
      </div>

      <div className={`des-body${bottom[tab] !== undefined ? " has-bottom" : ""}`}>
        <div className="des-main">{body()}</div>
        {bottom[tab] !== undefined && <div className="des-props">{bottom[tab]}</div>}
        {tab === "sql" && (
          <div className="des-sql-footer">
            <select className="field" value={previewMode} onChange={(e) => setPreviewMode(e.target.value as "save" | "saveAs")}>
              {!isNew && <option value="save">{t("des.previewSave")}</option>}
              <option value="saveAs">{t("des.previewSaveAs")}</option>
            </select>
          </div>
        )}
      </div>

      {dialog?.kind === "name" && (
        <MessageDialog
          title={t("des.saveTitle")}
          prompt={{ label: t("des.tableName"), initial: "" }}
          onOk={(name) => {
            if (!name) return;
            setDialog({ kind: "review", sql: createTableSql(schema, toObject({ ...d, name })), destructive: false, newName: name });
          }}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "review" && (
        <MessageDialog
          title={t("des.reviewTitle")}
          message={dialog.destructive ? t("des.destructiveWarning") : t("des.reviewMessage")}
          sql={dialog.sql}
          danger={dialog.destructive}
          confirmText={production && dialog.destructive ? (original?.name ?? dialog.newName) : undefined}
          okLabel={t("des.save")}
          onOk={() => commit(dialog.newName)}
          onCancel={() => setDialog(null)}
        />
      )}
    </div>
  );
}
