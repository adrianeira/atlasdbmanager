import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { errorText, pg, toAtlasError, type ObjectInfo } from "../../backend";
import { useT } from "../../i18n";
import { SmallIcon } from "../../icons";
import {
  AGGREGATES, JOIN_TYPES, NO_VALUE, OPERATORS, buildSql, emptyState, foreignKeyJoins, uid,
  type Aggregate, type JoinType, type Operator, type QbCond, type QbRef, type QbState, type QbTable,
} from "../../sql/querybuilder";

interface Props {
  connId: string;
  database: string;
  /** Schema da aba de consulta: as tabelas dele saem sem prefixo no SQL. */
  defaultSchema: string;
  onBuild: (sql: string, run: boolean) => void;
  onClose: () => void;
}

type Tab = "SELECT" | "FROM" | "WHERE" | "GROUP BY" | "HAVING" | "ORDER BY" | "LIMIT";
const TABS: Tab[] = ["SELECT", "FROM", "WHERE", "GROUP BY", "HAVING", "ORDER BY", "LIMIT"];

/** Geometria do cartão de tabela (usada também para desenhar as ligações). */
const CARD_W = 160;
const HEAD_H = 26;
const ROW_H = 20;
const PAD = 4;

interface Objects {
  tables: string[];
  views: string[];
}

const keyOf = (r: QbRef) => `${r.table}|${r.column}`;
const parseKey = (k: string): QbRef | null => {
  const i = k.indexOf("|");
  return i < 0 ? null : { table: k.slice(0, i), column: k.slice(i + 1) };
};

const KEYWORDS = /\b(SELECT|DISTINCT|FROM|INNER|LEFT|RIGHT|FULL|CROSS|JOIN|ON|WHERE|GROUP BY|HAVING|ORDER BY|LIMIT|OFFSET|AND|OR|AS|DESC|NOT|IN|LIKE|ILIKE|BETWEEN|IS|NULL|COUNT|SUM|AVG|MIN|MAX)\b/g;

/** O SQL com as palavras-chave coloridas (aspas e textos ficam como estão). */
function Highlighted({ sql }: { sql: string }) {
  const parts = sql.split(/('(?:[^']|'')*'|"(?:[^"]|"")*")/);
  return (
    <>
      {parts.map((p, i) =>
        i % 2 === 1 ? <Fragment key={i}>{p}</Fragment> : p.split(KEYWORDS).map((q, j) => (j % 2 === 1 ? <span key={`${i}-${j}`} className="qb-kw">{q}</span> : <Fragment key={`${i}-${j}`}>{q}</Fragment>)),
      )}
    </>
  );
}

function RefSelect({ tables, value, onChange, star = true, placeholder }: {
  tables: QbTable[]; value: QbRef | null; onChange: (r: QbRef | null) => void; star?: boolean; placeholder: string;
}) {
  return (
    <select className="field qb-sel" value={value ? keyOf(value) : ""} onChange={(e) => onChange(parseKey(e.target.value))}>
      <option value="">{placeholder}</option>
      {tables.map((t) => (
        <optgroup key={t.id} label={t.alias.trim() || t.name}>
          {(star ? ["*", ...t.columns] : t.columns).map((c) => <option key={c} value={keyOf({ table: t.id, column: c })}>{c}</option>)}
        </optgroup>
      ))}
    </select>
  );
}

/** Janela do Query Builder: monta um SELECT arrastando tabelas, marcando campos e ligando-os; o SQL aparece ao vivo. */
export function QueryBuilderDialog({ connId, database, defaultSchema, onBuild, onClose }: Props) {
  const t = useT();
  const [st, setSt] = useState<QbState>(emptyState);
  const [schemas, setSchemas] = useState<string[]>([]);
  const [objects, setObjects] = useState<Record<string, Objects | "loading">>({});
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [pickedObject, setPickedObject] = useState("");
  const [tab, setTab] = useState<Tab>("SELECT");
  const [mode, setMode] = useState<"field" | "table">("field");
  const [panes, setPanes] = useState({ left: true, right: true });
  const [selField, setSelField] = useState("");
  const [selJoin, setSelJoin] = useState("");
  const [selTable, setSelTable] = useState("");
  const [adding, setAdding] = useState(false);
  const [draftJoin, setDraftJoin] = useState<{ left: QbRef | null; right: QbRef | null }>({ left: null, right: null });
  const [error, setError] = useState<string | null>(null);
  const [drag, setDrag] = useState<{ id: string; dx: number; dy: number } | null>(null);
  const [link, setLink] = useState<{ from: QbRef; x: number; y: number } | null>(null);
  const [treeDrag, setTreeDrag] = useState<{ schema: string; name: string; view: boolean; x: number; y: number; sx: number; sy: number; moved: boolean } | null>(null);
  const canvasRef = useRef<HTMLDivElement>(null);

  const sql = useMemo(() => buildSql(st, defaultSchema), [st, defaultSchema]);

  /* ---------- Árvore ---------- */
  const loadObjects = useCallback(async (schema: string) => {
    setObjects((o) => (o[schema] ? o : { ...o, [schema]: "loading" }));
    try {
      const o = await pg.listObjects(connId, database, schema);
      const names = (list: ObjectInfo[]) => list.map((x) => x.name);
      setObjects((m) => ({ ...m, [schema]: { tables: names(o.tables), views: [...names(o.views), ...names(o.matviews)] } }));
    } catch (e) {
      setObjects((m) => ({ ...m, [schema]: { tables: [], views: [] } }));
      setError(errorText(toAtlasError(e)));
    }
  }, [connId, database]);

  useEffect(() => {
    let alive = true;
    pg.listSchemas(connId, database)
      .then((list) => {
        if (!alive) return;
        setSchemas(list);
        const first = list.includes(defaultSchema) ? defaultSchema : list[0];
        if (first) {
          setOpen(new Set([first]));
          void loadObjects(first);
        }
      })
      .catch((e) => alive && setError(errorText(toAtlasError(e))));
    return () => {
      alive = false;
    };
    // Só na abertura da janela.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggleSchema = (s: string) => {
    setOpen((o) => {
      const n = new Set(o);
      if (n.has(s)) n.delete(s);
      else {
        n.add(s);
        void loadObjects(s);
      }
      return n;
    });
  };

  /* ---------- Tabelas do desenho ---------- */
  const addTable = (schema: string, name: string, view: boolean, at?: { x: number; y: number }) => {
    const id = uid("t");
    setSt((s) => {
      const n = s.tables.length;
      const dup = s.tables.filter((x) => x.schema === schema && x.name === name).length;
      const table: QbTable = {
        id, schema, name, view, alias: dup ? `${name}_${dup + 1}` : "", columns: [], fks: [],
        x: Math.max(0, at?.x ?? 24 + (n % 4) * 200), y: Math.max(0, at?.y ?? 24 + Math.floor(n / 4) * 220),
      };
      return { ...s, tables: [...s.tables, table] };
    });
    setSelTable(id);
    pg.tableDetail(connId, database, schema, name)
      .then((d) => {
        const columns = d.columns.map((c) => c.name);
        const fks = d.constraints
          .filter((c) => c.kind === "f" && c.refTable)
          .map((c) => ({ fields: c.fields.split(", "), refSchema: c.refSchema ?? schema, refTable: c.refTable ?? "", refFields: (c.refFields ?? "").split(", ") }));
        setSt((s) => {
          const mine = s.tables.find((x) => x.id === id);
          if (!mine) return s;
          const loaded = { ...mine, columns, fks };
          return {
            ...s,
            tables: s.tables.map((x) => (x.id === id ? loaded : x)),
            joins: [...s.joins, ...foreignKeyJoins(loaded, s.tables.filter((x) => x.id !== id), s.joins)],
          };
        });
      })
      .catch((e) => setError(errorText(toAtlasError(e))));
  };

  const removeTable = (id: string) => {
    setSt((s) => {
      const hit = (r: QbRef | null) => r?.table === id;
      return {
        ...s,
        tables: s.tables.filter((x) => x.id !== id),
        fields: s.fields.filter((f) => !hit(f.ref)),
        joins: s.joins.filter((j) => !hit(j.left) && !hit(j.right)),
        where: s.where.filter((c) => !hit(c.ref)),
        groupBy: s.groupBy.filter((r) => !hit(r)),
        having: s.having.filter((c) => !hit(c.ref)),
        orderBy: s.orderBy.filter((o) => !hit(o.ref)),
      };
    });
    setSelTable("");
  };

  const toggleColumn = (table: string, column: string, on: boolean) =>
    setSt((s) => ({
      ...s,
      fields: on
        ? s.fields.some((f) => f.ref.table === table && f.ref.column === column) ? s.fields : [...s.fields, { id: uid("f"), ref: { table, column }, aggregate: "", alias: "" }]
        : s.fields.filter((f) => !(f.ref.table === table && f.ref.column === column)),
    }));

  const addJoin = (left: QbRef, right: QbRef, type: JoinType = "INNER") =>
    setSt((s) => (s.joins.some((j) => (keyOf(j.left) === keyOf(left) && keyOf(j.right) === keyOf(right)) || (keyOf(j.left) === keyOf(right) && keyOf(j.right) === keyOf(left)))
      ? s
      : { ...s, joins: [...s.joins, { id: uid("j"), type, left, right }] }));

  /* ---------- Arrastar cartões, ligar campos e soltar da árvore (tudo por mouse: não depende do drag-and-drop do navegador) ---------- */
  const canvasPoint = (e: { clientX: number; clientY: number }) => {
    const el = canvasRef.current;
    if (!el) return { x: 0, y: 0 };
    const r = el.getBoundingClientRect();
    return { x: e.clientX - r.left + el.scrollLeft, y: e.clientY - r.top + el.scrollTop };
  };

  useEffect(() => {
    if (!drag && !link && !treeDrag) return;
    const move = (e: MouseEvent) => {
      if (drag) {
        const p = canvasPoint(e);
        setSt((s) => ({ ...s, tables: s.tables.map((x) => (x.id === drag.id ? { ...x, x: Math.max(0, p.x - drag.dx), y: Math.max(0, p.y - drag.dy) } : x)) }));
      }
      if (link) setLink((l) => (l ? { ...l, ...canvasPoint(e) } : l));
      if (treeDrag) setTreeDrag((d) => (d ? { ...d, x: e.clientX, y: e.clientY, moved: d.moved || Math.abs(e.clientX - d.sx) + Math.abs(e.clientY - d.sy) > 4 } : d));
    };
    const up = (e: MouseEvent) => {
      if (link) {
        const row = (document.elementFromPoint(e.clientX, e.clientY) as Element | null)?.closest<HTMLElement>("[data-qb-table]");
        const to = row ? { table: row.dataset.qbTable ?? "", column: row.dataset.qbColumn ?? "" } : null;
        if (to && to.table !== link.from.table && to.column !== "*" && link.from.column !== "*") addJoin(link.from, to);
      }
      if (treeDrag?.moved) {
        const el = canvasRef.current;
        const r = el?.getBoundingClientRect();
        if (r && e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom) addTable(treeDrag.schema, treeDrag.name, treeDrag.view, canvasPoint(e));
      }
      setDrag(null);
      setLink(null);
      setTreeDrag(null);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    // As funções usadas leem o estado mais recente por atualização funcional.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag, link, treeDrag]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      const tag = (e.target as Element | null)?.tagName;
      if (e.key === "Delete" && selTable && tag !== "INPUT" && tag !== "SELECT") removeTable(selTable);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, selTable]);

  /* ---------- Ligações no desenho ---------- */
  const anchor = (r: QbRef, otherX: number) => {
    const tb = st.tables.find((x) => x.id === r.table);
    if (!tb) return { x: 0, y: 0 };
    const idx = r.column === "*" ? 0 : tb.columns.indexOf(r.column) + 1;
    const y = mode === "table" ? tb.y + HEAD_H / 2 : tb.y + HEAD_H + PAD + idx * ROW_H + ROW_H / 2;
    const right = tb.x + CARD_W / 2 < otherX;
    return { x: right ? tb.x + CARD_W : tb.x, y };
  };
  const centerX = (r: QbRef) => (st.tables.find((x) => x.id === r.table)?.x ?? 0) + CARD_W / 2;

  const size = useMemo(() => ({
    w: Math.max(600, ...st.tables.map((x) => x.x + CARD_W + 40)),
    h: Math.max(300, ...st.tables.map((x) => x.y + HEAD_H + PAD * 2 + (x.columns.length + 1) * ROW_H + 40)),
  }), [st.tables]);

  /* ---------- Etapas de edição ---------- */
  const labelOf = (r: QbRef) => {
    const tb = st.tables.find((x) => x.id === r.table);
    return `${tb ? tb.alias.trim() || `${tb.schema}.${tb.name}` : "?"}.${r.column}`;
  };
  const tbls = st.tables;

  const moveField = (dir: -1 | 1) =>
    setSt((s) => {
      const i = s.fields.findIndex((f) => f.id === selField);
      const j = i + dir;
      if (i < 0 || j < 0 || j >= s.fields.length) return s;
      const fields = [...s.fields];
      [fields[i], fields[j]] = [fields[j], fields[i]];
      return { ...s, fields };
    });

  const patchCond = (key: "where" | "having", id: string, patch: Partial<QbCond>) =>
    setSt((s) => ({ ...s, [key]: s[key].map((c) => (c.id === id ? { ...c, ...patch } : c)) }));
  const addCond = (key: "where" | "having") =>
    setSt((s) => ({ ...s, [key]: [...s[key], { id: uid("c"), connector: "AND", ref: null, aggregate: key === "having" ? "COUNT" : "", op: "=", value: "", value2: "" } satisfies QbCond] }));

  const addBtn = (onClick: () => void) => (
    <button type="button" className="qb-add" aria-label="+" onClick={onClick}><SmallIcon name="plusSquare" size={16} /></button>
  );
  const rm = (onClick: () => void) => (
    <button type="button" className="qb-rm" aria-label={t("qb.remove")} title={t("qb.remove")} onClick={onClick}><SmallIcon name="close" size={12} /></button>
  );

  const condEditor = (key: "where" | "having") => (
    <div className="qb-rows">
      {st[key].length === 0 && <div className="qb-hint">{t("qb.noConds")}</div>}
      {st[key].map((c, i) => (
        <div key={c.id} className="qb-row">
          {i === 0 ? <span className="qb-conn" /> : (
            <select className="field qb-conn" value={c.connector} onChange={(e) => patchCond(key, c.id, { connector: e.target.value as "AND" | "OR" })}>
              <option>AND</option><option>OR</option>
            </select>
          )}
          {key === "having" && (
            <select className="field qb-agg" value={c.aggregate} onChange={(e) => patchCond(key, c.id, { aggregate: e.target.value as Aggregate })}>
              {AGGREGATES.filter(Boolean).map((a) => <option key={a}>{a}</option>)}
            </select>
          )}
          <RefSelect tables={tbls} value={c.ref} star={key === "having"} placeholder={t("qb.pickField")} onChange={(r) => patchCond(key, c.id, { ref: r })} />
          <select className="field qb-op" value={c.op} onChange={(e) => patchCond(key, c.id, { op: e.target.value as Operator })}>
            {OPERATORS.map((o) => <option key={o}>{o}</option>)}
          </select>
          {!NO_VALUE.includes(c.op) && <input className="field qb-val" value={c.value} placeholder={t("qb.value")} spellCheck={false} onChange={(e) => patchCond(key, c.id, { value: e.target.value })} />}
          {c.op === "BETWEEN" && <input className="field qb-val" value={c.value2} placeholder={t("qb.value")} spellCheck={false} onChange={(e) => patchCond(key, c.id, { value2: e.target.value })} />}
          {rm(() => setSt((s) => ({ ...s, [key]: s[key].filter((x) => x.id !== c.id) })))}
        </div>
      ))}
      <div className="qb-row">{addBtn(() => addCond(key))}</div>
    </div>
  );

  const selectTab = (
    <div className="qb-rows">
      {st.fields.length === 0 && !adding && <div className="qb-hint">{t("qb.addField")}</div>}
      {st.fields.map((f) => (
        <div key={f.id} className={`qb-row qb-field-row${selField === f.id ? " is-selected" : ""}`} onClick={() => setSelField(f.id)}>
          <span className="qb-ref" title={labelOf(f.ref)}>{labelOf(f.ref)}</span>
          <select className="field qb-agg" value={f.aggregate} onChange={(e) => setSt((s) => ({ ...s, fields: s.fields.map((x) => (x.id === f.id ? { ...x, aggregate: e.target.value as Aggregate } : x)) }))}>
            {AGGREGATES.map((a) => <option key={a} value={a}>{a || t("qb.noAgg")}</option>)}
          </select>
          <input className="field qb-alias" value={f.alias} placeholder={t("qb.alias")} spellCheck={false} onChange={(e) => setSt((s) => ({ ...s, fields: s.fields.map((x) => (x.id === f.id ? { ...x, alias: e.target.value } : x)) }))} />
          {rm(() => setSt((s) => ({ ...s, fields: s.fields.filter((x) => x.id !== f.id) })))}
        </div>
      ))}
      <div className="qb-row">
        {adding
          ? <RefSelect tables={tbls} value={null} placeholder={t("qb.pickField")} onChange={(r) => { if (r) toggleColumn(r.table, r.column, true); setAdding(false); }} />
          : addBtn(() => setAdding(true))}
      </div>
    </div>
  );

  const fromTab = (
    <div className="qb-rows">
      {tbls.length === 0 && <div className="qb-hint">{t("qb.dropHere")}</div>}
      {tbls.map((tb, i) => (
        <div key={tb.id} className="qb-row">
          <SmallIcon name={tb.view ? "view" : "table"} />
          <span className="qb-ref" title={`${tb.schema}.${tb.name}`}>{tb.schema}.{tb.name}{i === 0 ? "" : ""}</span>
          <input className="field qb-alias" value={tb.alias} placeholder={t("qb.alias")} spellCheck={false}
            onChange={(e) => setSt((s) => ({ ...s, tables: s.tables.map((x) => (x.id === tb.id ? { ...x, alias: e.target.value } : x)) }))} />
          {rm(() => removeTable(tb.id))}
        </div>
      ))}
      {st.joins.length > 0 && <div className="qb-subtitle">{t("qb.relations")}</div>}
      {st.joins.map((j) => (
        <div key={j.id} className={`qb-row${selJoin === j.id ? " is-selected" : ""}`} onClick={() => setSelJoin(j.id)}>
          <RefSelect tables={tbls} value={j.left} star={false} placeholder={t("qb.pickField")} onChange={(r) => r && setSt((s) => ({ ...s, joins: s.joins.map((x) => (x.id === j.id ? { ...x, left: r } : x)) }))} />
          <select className="field qb-agg" value={j.type} onChange={(e) => setSt((s) => ({ ...s, joins: s.joins.map((x) => (x.id === j.id ? { ...x, type: e.target.value as JoinType } : x)) }))}>
            {JOIN_TYPES.map((x) => <option key={x}>{x}</option>)}
          </select>
          <RefSelect tables={tbls} value={j.right} star={false} placeholder={t("qb.pickField")} onChange={(r) => r && setSt((s) => ({ ...s, joins: s.joins.map((x) => (x.id === j.id ? { ...x, right: r } : x)) }))} />
          {rm(() => setSt((s) => ({ ...s, joins: s.joins.filter((x) => x.id !== j.id) })))}
        </div>
      ))}
      {tbls.length > 1 && (
        <div className="qb-row">
          {adding ? (
            <>
              <RefSelect tables={tbls} value={draftJoin.left} star={false} placeholder={t("qb.pickField")} onChange={(r) => setDraftJoin((d) => ({ ...d, left: r }))} />
              <RefSelect tables={tbls} value={draftJoin.right} star={false} placeholder={t("qb.pickField")}
                onChange={(r) => {
                  if (r && draftJoin.left && r.table !== draftJoin.left.table) {
                    addJoin(draftJoin.left, r);
                    setAdding(false);
                    setDraftJoin({ left: null, right: null });
                  } else setDraftJoin((d) => ({ ...d, right: r }));
                }} />
            </>
          ) : addBtn(() => setAdding(true))}
        </div>
      )}
    </div>
  );

  const groupTab = (
    <div className="qb-rows">
      {st.groupBy.length === 0 && !adding && <div className="qb-hint">{t("qb.addField")}</div>}
      {st.groupBy.map((r, i) => (
        <div key={`${keyOf(r)}-${i}`} className="qb-row">
          <span className="qb-ref">{labelOf(r)}</span>
          {rm(() => setSt((s) => ({ ...s, groupBy: s.groupBy.filter((_, k) => k !== i) })))}
        </div>
      ))}
      <div className="qb-row">
        {adding
          ? <RefSelect tables={tbls} value={null} star={false} placeholder={t("qb.pickField")} onChange={(r) => { if (r) setSt((s) => ({ ...s, groupBy: [...s.groupBy, r] })); setAdding(false); }} />
          : addBtn(() => setAdding(true))}
      </div>
    </div>
  );

  const orderTab = (
    <div className="qb-rows">
      {st.orderBy.length === 0 && !adding && <div className="qb-hint">{t("qb.addField")}</div>}
      {st.orderBy.map((o) => (
        <div key={o.id} className="qb-row">
          <span className="qb-ref">{labelOf(o.ref)}</span>
          <select className="field qb-agg" value={o.desc ? "DESC" : "ASC"} onChange={(e) => setSt((s) => ({ ...s, orderBy: s.orderBy.map((x) => (x.id === o.id ? { ...x, desc: e.target.value === "DESC" } : x)) }))}>
            <option>ASC</option><option>DESC</option>
          </select>
          {rm(() => setSt((s) => ({ ...s, orderBy: s.orderBy.filter((x) => x.id !== o.id) })))}
        </div>
      ))}
      <div className="qb-row">
        {adding
          ? <RefSelect tables={tbls} value={null} star={false} placeholder={t("qb.pickField")} onChange={(r) => { if (r) setSt((s) => ({ ...s, orderBy: [...s.orderBy, { id: uid("o"), ref: r, desc: false }] })); setAdding(false); }} />
          : addBtn(() => setAdding(true))}
      </div>
    </div>
  );

  const limitTab = (
    <div className="qb-rows">
      <div className="qb-row"><label className="qb-label">{t("qb.limit")}</label>
        <input className="field qb-val" type="number" min={0} value={st.limit} onChange={(e) => setSt((s) => ({ ...s, limit: e.target.value }))} /></div>
      <div className="qb-row"><label className="qb-label">{t("qb.offset")}</label>
        <input className="field qb-val" type="number" min={0} value={st.offset} onChange={(e) => setSt((s) => ({ ...s, offset: e.target.value }))} /></div>
    </div>
  );

  /* ---------- Render ---------- */
  const lines = st.joins.map((j) => {
    const a = anchor(j.left, centerX(j.right));
    const b = anchor(j.right, centerX(j.left));
    return { j, a, b };
  });

  return (
    <div className="modal-overlay" role="presentation">
      <div className="window qb-window" role="dialog" aria-modal="true" aria-label={t("qb.title")}>
        <div className="window-title">
          <SmallIcon name="queryBuilder" />
          <span>{t("qb.title")}</span>
          <button type="button" className="window-close" aria-label={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button>
        </div>

        <div className={`qb-body${panes.left ? "" : " no-left"}${panes.right ? "" : " no-right"}`}>
          {panes.left && (
            <div className="qb-tree" role="tree">
              {schemas.map((s) => {
                const o = objects[s];
                return (
                  <div key={s}>
                    <div className="qb-tree-item qb-schema" onClick={() => toggleSchema(s)}>
                      <SmallIcon name={open.has(s) ? "chevronDown" : "chevronRight"} size={10} />
                      <SmallIcon name={s === defaultSchema ? "schema" : "schemaGray"} />
                      <span>{s}</span>
                    </div>
                    {open.has(s) && o === "loading" && <div className="qb-tree-item qb-leaf muted">{t("qb.loading")}</div>}
                    {open.has(s) && o && o !== "loading" && [...o.tables.map((n) => [n, false] as const), ...o.views.map((n) => [n, true] as const)].map(([n, view]) => (
                      <div
                        key={`${view ? "v" : "t"}:${n}`}
                        className={`qb-tree-item qb-leaf${pickedObject === `${s}.${n}` ? " is-selected" : ""}`}
                        onMouseDown={(e) => {
                          if (e.button !== 0) return;
                          setPickedObject(`${s}.${n}`);
                          setTreeDrag({ schema: s, name: n, view, x: e.clientX, y: e.clientY, sx: e.clientX, sy: e.clientY, moved: false });
                        }}
                        onDoubleClick={() => addTable(s, n, view)}
                      >
                        <SmallIcon name={view ? "view" : "table"} />
                        <span>{n}</span>
                      </div>
                    ))}
                  </div>
                );
              })}
            </div>
          )}

          <div className="qb-center">
            <div className="qb-canvas-wrap">
              <div className="qb-modes">
                <label><input type="radio" name="qb-mode" checked={mode === "table"} onChange={() => setMode("table")} />{t("qb.tableRelation")}</label>
                <label><input type="radio" name="qb-mode" checked={mode === "field"} onChange={() => setMode("field")} />{t("qb.fieldRelation")}</label>
              </div>
              <div className="qb-canvas" ref={canvasRef} onMouseDown={(e) => { if (e.target === e.currentTarget || (e.target as Element).tagName === "svg") { setSelTable(""); setSelJoin(""); } }}>
                <div className="qb-canvas-inner" style={{ width: size.w, height: size.h }}>
                  {st.tables.length === 0 && <div className="qb-empty">{t("qb.dropHere")}</div>}
                  <svg className="qb-lines" width={size.w} height={size.h}>
                    <defs>
                      <marker id="qb-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M0 0L8 4L0 8z" fill="currentColor" /></marker>
                    </defs>
                    {lines.map(({ j, a, b }) => (
                      <g key={j.id} className={`qb-line${selJoin === j.id ? " is-selected" : ""}`} onClick={() => { setSelJoin(j.id); setTab("FROM"); }}>
                        <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} className="qb-hit" />
                        <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} markerEnd="url(#qb-arrow)" />
                        <title>{`${j.type} JOIN: ${labelOf(j.left)} = ${labelOf(j.right)}`}</title>
                      </g>
                    ))}
                    {link && (() => {
                      const a = anchor(link.from, link.x);
                      return <line x1={a.x} y1={a.y} x2={link.x} y2={link.y} className="qb-rubber" />;
                    })()}
                  </svg>
                  {st.tables.map((tb) => (
                    <div key={tb.id} className={`qb-card${selTable === tb.id ? " is-selected" : ""}${tb.view ? " is-view" : ""}`} style={{ left: tb.x, top: tb.y, width: CARD_W }}
                      onMouseDown={() => setSelTable(tb.id)}>
                      <div className="qb-card-head" title={`${tb.schema}.${tb.name}`}
                        onMouseDown={(e) => {
                          if (e.button !== 0) return;
                          const p = canvasPoint(e);
                          setDrag({ id: tb.id, dx: p.x - tb.x, dy: p.y - tb.y });
                        }}>
                        <span>{tb.alias.trim() || tb.name}</span>
                      </div>
                      <div className="qb-card-body" style={{ padding: PAD }}>
                        {tb.columns.length === 0 ? <div className="qb-row-col muted">{t("qb.loading")}</div> : ["*", ...tb.columns].map((c) => (
                          <label key={c} className="qb-row-col" data-qb-table={tb.id} data-qb-column={c}>
                            <input type="checkbox" checked={st.fields.some((f) => f.ref.table === tb.id && f.ref.column === c)} onChange={(e) => toggleColumn(tb.id, c, e.target.checked)} />
                            <span
                              onMouseDown={(e) => {
                                if (e.button !== 0 || c === "*") return;
                                e.preventDefault();
                                setLink({ from: { table: tb.id, column: c }, ...canvasPoint(e) });
                              }}
                            >{c}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>

            <div className="qb-editor">
              <div className="qb-tabs" role="tablist">
                {TABS.map((x) => (
                  <button key={x} type="button" role="tab" aria-selected={tab === x} className={`qb-tab${tab === x ? " is-active" : ""}`} onClick={() => { setTab(x); setAdding(false); }}>{x}</button>
                ))}
              </div>
              <div className="qb-panel">
                {tab === "SELECT" && selectTab}
                {tab === "FROM" && fromTab}
                {tab === "WHERE" && condEditor("where")}
                {tab === "GROUP BY" && groupTab}
                {tab === "HAVING" && condEditor("having")}
                {tab === "ORDER BY" && orderTab}
                {tab === "LIMIT" && limitTab}
              </div>
              <div className="qb-panel-foot">
                <button type="button" className="icon-button" aria-label={t("qb.moveUp")} title={t("qb.moveUp")} disabled={tab !== "SELECT" || !selField} onClick={() => moveField(-1)}>↑</button>
                <button type="button" className="icon-button" aria-label={t("qb.moveDown")} title={t("qb.moveDown")} disabled={tab !== "SELECT" || !selField} onClick={() => moveField(1)}>↓</button>
                <label className="check-row"><input type="checkbox" checked={st.distinct} onChange={(e) => setSt((s) => ({ ...s, distinct: e.target.checked }))} /><span>DISTINCT</span></label>
                {error && <span className="msg-error qb-error" role="alert" title={error}>{error}</span>}
              </div>
            </div>
          </div>

          {panes.right && <pre className="qb-sql" aria-label="SQL"><Highlighted sql={sql} /></pre>}
        </div>

        <div className="qb-footer">
          <button type="button" className={`icon-button${panes.left ? " is-active" : ""}`} title={t("qb.toggleTree")} aria-pressed={panes.left} onClick={() => setPanes((p) => ({ ...p, left: !p.left }))}><SmallIcon name="sidebar" /></button>
          <button type="button" className={`icon-button qb-flip${panes.right ? " is-active" : ""}`} title={t("qb.toggleSql")} aria-pressed={panes.right} onClick={() => setPanes((p) => ({ ...p, right: !p.right }))}><SmallIcon name="sidebar" /></button>
          <span className="spacer" />
          <button type="button" className="btn" disabled={st.tables.length === 0} onClick={() => onBuild(sql, false)}>{t("qb.build")}</button>
          <button type="button" className="btn btn-primary" disabled={st.tables.length === 0} onClick={() => onBuild(sql, true)}>{t("qb.buildRun")}</button>
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
        </div>
      </div>
      {treeDrag?.moved && <div className="qb-ghost" style={{ left: treeDrag.x + 10, top: treeDrag.y + 6 }}><SmallIcon name={treeDrag.view ? "view" : "table"} />{treeDrag.name}</div>}
    </div>
  );
}
