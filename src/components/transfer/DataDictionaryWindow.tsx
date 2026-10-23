import { documentDir } from "@tauri-apps/api/path";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MenuEntry } from "../../commands";
import {
  chooseExportPath, errorText, local, onDictLog, onDictProgress, pg, toAtlasError,
  type DataTransferConn, type DataTransferParams, type DatabaseDetail, type DictCounts, type DictServerSel, type DictSettings, type DictTemplate, type SchemaDetail,
} from "../../backend";
import { useT } from "../../i18n";
import { ProviderIcon, SmallIcon } from "../../icons";
import { MessageDialog } from "../MessageDialog";
import { PopupMenu } from "../PopupMenu";
import { ToolWindow } from "../ToolWindow";
import { LayoutStep, TemplateStep } from "./dictionaryLayout";
import { formatClock } from "./transferModel";

type Params = DataTransferParams & { title: string };
type Category = "tables" | "views" | "functions";
const CATEGORIES: Category[] = ["tables", "views", "functions"];

/** O que está marcado na primeira etapa: conexão → banco → schemas marcados. */
type Selection = Record<string, Record<string, string[]>>;

interface PlanCategory { on: boolean; all: string[]; picked: string[] }
interface PlanSchema { name: string; info: boolean; cats: Record<Category, PlanCategory> }
interface PlanDb { name: string; info: boolean; schemas: PlanSchema[] }
interface PlanServer { connId: string; info: boolean; dbs: PlanDb[] }

type Focus = { kind: "server"; connId: string } | { kind: "db"; connId: string; db: string } | { kind: "schema"; connId: string; db: string; schema: string };

interface Profile {
  name: string;
  selection: Selection;
  plan: PlanServer[];
  settings: DictSettings;
  path: string;
  appendTimestamp: boolean;
  overwrite: boolean;
  continueOnError: boolean;
}

const PROFILES_KEY = "datadictionary.profiles";
const STEP_TITLES = ["dd.step.select", "dd.step.objects", "dd.step.template", "dd.step.layout", "dd.step.path"] as const;

const nowParts = (): DictSettings["now"] => {
  const d = new Date();
  return [d.getFullYear(), d.getMonth() + 1, d.getDate(), d.getHours(), d.getMinutes(), d.getSeconds()];
};

const pad = (n: number, w = 2) => String(n).padStart(w, "0");
const stampOf = (n: DictSettings["now"]) => `${pad(n[0], 4)}${pad(n[1])}${pad(n[2])}${pad(n[3])}${pad(n[4])}${pad(n[5])}`;

export function DataDictionaryWindow() {
  const params = (window as unknown as { __ATLAS_DD__: Params }).__ATLAS_DD__;
  return (
    <ToolWindow title={params.title}>
      <Wizard params={params} />
    </ToolWindow>
  );
}

function Wizard({ params }: { params: Params }) {
  const t = useT();
  const [step, setStep] = useState(0);
  const [opened, setOpened] = useState<Record<string, { version: string; versionNum: number }>>({});
  const [connecting, setConnecting] = useState<string | null>(null);
  const [pwPrompt, setPwPrompt] = useState<{ connId: string; retry?: string } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const conns: DataTransferConn[] = useMemo(
    () => params.connections.map((c) => (opened[c.id] ? { ...c, open: true, version: opened[c.id].version, versionNum: opened[c.id].versionNum } : c)),
    [params.connections, opened],
  );

  const [dbLists, setDbLists] = useState<Record<string, string[]>>({});
  const [schemaLists, setSchemaLists] = useState<Record<string, string[]>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selection, setSelection] = useState<Selection>({});
  const [focus, setFocus] = useState<Focus | null>(null);
  const [search, setSearch] = useState("");
  const [info, setInfo] = useState<{ title: string; kind: string; rows: [string, string][] } | null>(null);

  const [plan, setPlan] = useState<PlanServer[]>([]);
  const [planBusy, setPlanBusy] = useState(false);
  const [planExpanded, setPlanExpanded] = useState<Set<string>>(new Set());

  const [defaults, setDefaults] = useState<DictSettings | null>(null);
  const [settings, setSettings] = useState<DictSettings | null>(null);
  const [path, setPath] = useState("");
  const [appendTimestamp, setAppendTimestamp] = useState(true);
  const [overwrite, setOverwrite] = useState(false);
  const [continueOnError, setContinueOnError] = useState(false);

  const [running, setRunning] = useState(false);
  const [finished, setFinished] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [counts, setCounts] = useState<DictCounts>({ tables: 0, views: 0, functions: 0, errors: 0, fraction: 0 });
  const [outputPath, setOutputPath] = useState<string | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const runId = useRef("");
  const startedAt = useRef(0);
  const logRef = useRef<HTMLDivElement>(null);

  const [menu, setMenu] = useState<"save" | "load" | "open" | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [namePrompt, setNamePrompt] = useState(false);

  const connOf = useCallback((id: string) => conns.find((c) => c.id === id), [conns]);

  /* ---------- Conexões ---------- */
  const openConn = useCallback(async (id: string, password?: string) => {
    const c = params.connections.find((x) => x.id === id);
    if (!c || c.open || opened[id] || connecting === id) return;
    if (password === undefined && !c.hasPassword) {
      setPwPrompt((p) => p ?? { connId: id });
      return;
    }
    setConnecting(id);
    setMessage(null);
    try {
      const r = await pg.open(id, { host: c.host, port: c.port, database: c.initialDatabase || "postgres", user: c.user, password: password ?? "", sslMode: c.sslMode ?? "disable" }, id);
      setOpened((o) => ({ ...o, [id]: { version: r.version, versionNum: r.versionNum } }));
    } catch (e) {
      const err = toAtlasError(e);
      if (err.kind === "authenticationFailed") setPwPrompt({ connId: id, retry: errorText(err) });
      else setMessage(errorText(err));
    } finally {
      setConnecting(null);
    }
  }, [params.connections, opened, connecting]);

  const loadDbs = useCallback(async (connId: string) => {
    try {
      const r = await pg.listDatabases(connId);
      setDbLists((l) => ({ ...l, [connId]: r.databases }));
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  }, []);

  const loadSchemas = useCallback(async (connId: string, db: string) => {
    try {
      const r = await pg.listSchemas(connId, db);
      setSchemaLists((l) => ({ ...l, [`${connId}/${db}`]: r.map((x) => (typeof x === "string" ? x : (x as { name: string }).name)) }));
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  }, []);

  // Abre as conexões que o usuário expande e lê os bancos quando a conexão está aberta.
  useEffect(() => {
    for (const key of expanded) {
      if (key.startsWith("c:")) {
        const id = key.slice(2);
        const c = connOf(id);
        if (c && !c.open) void openConn(id);
        else if (c?.open && !dbLists[id]) void loadDbs(id);
      } else if (key.startsWith("d:")) {
        const [id, ...rest] = key.slice(2).split("/");
        const db = rest.join("/");
        if (connOf(id)?.open && !schemaLists[`${id}/${db}`]) void loadSchemas(id, db);
      }
    }
  }, [expanded, conns, dbLists, schemaLists, connOf, openConn, loadDbs, loadSchemas]);

  // O que a janela principal tinha selecionado começa marcado e aberto.
  useEffect(() => {
    const s = params.source;
    if (s) {
      setSelection({ [s.connId]: { [s.database]: [s.schema] } });
      setExpanded(new Set([`c:${s.connId}`, `d:${s.connId}/${s.database}`]));
      setFocus({ kind: "schema", connId: s.connId, db: s.database, schema: s.schema });
    } else if (params.connections[0]) {
      setExpanded(new Set([`c:${params.connections[0].id}`]));
    }
    void local.getPref<Profile[]>(PROFILES_KEY).then((p) => setProfiles(Array.isArray(p) ? p : [])).catch(() => undefined);
    void documentDir().then((d) => setPath(`${d.replace(/[\\/]$/, "")}${d.includes("\\") ? "\\" : "/"}DataDictionary.pdf`)).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => setElapsed(performance.now() - startedAt.current), 100);
    return () => window.clearInterval(id);
  }, [running]);
  useEffect(() => { if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight; }, [log]);
  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => { if (!(e.target as Element | null)?.closest?.(".dt-menu-wrap")) setMenu(null); };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  /* ---------- Painel de informações ---------- */
  useEffect(() => {
    if (!focus) { setInfo(null); return; }
    let alive = true;
    const c = connOf(focus.connId);
    if (focus.kind === "server") {
      setInfo({ title: c?.name ?? "", kind: "PostgreSQL", rows: serverInfo(focus.connId) });
    } else if (focus.kind === "db") {
      setInfo({ title: focus.db, kind: "Database", rows: [] });
      if (c?.open) {
        pg.databaseDetail(focus.connId, focus.db).then((d: DatabaseDetail) => alive && setInfo({
          title: focus.db, kind: "Database",
          rows: [[t("dd.i.owner"), d.owner], [t("dd.i.tablespace"), d.tablespace], [t("dd.i.encoding"), d.encoding], [t("dd.i.collation"), d.collate], [t("dd.i.limit"), d.connectionLimit < 0 ? "N/A" : String(d.connectionLimit)], [t("dd.i.comment"), d.comment ?? "--"]],
        })).catch(() => undefined);
      }
    } else {
      setInfo({ title: focus.schema, kind: "Schema", rows: [] });
      if (c?.open) {
        pg.schemaDetail(focus.connId, focus.db, focus.schema).then((d: SchemaDetail) => alive && setInfo({ title: focus.schema, kind: "Schema", rows: [[t("dd.i.owner"), d.owner], [t("dd.i.comment"), d.comment ?? "--"]] })).catch(() => undefined);
      }
    }
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus, conns]);

  const serverInfo = (id: string): [string, string][] => {
    const c = connOf(id);
    return [[t("dd.i.version"), String(c?.versionNum || "--")], [t("dd.i.host"), c?.host ?? "--"], [t("dd.i.port"), String(c?.port ?? "--")], [t("dd.i.initialDb"), c?.initialDatabase || "postgres"], [t("dd.i.user"), c?.user ?? "--"]];
  };

  /* ---------- Etapa 1: bancos e schemas ---------- */
  const toggle = (set: Set<string>, setter: (s: Set<string>) => void, key: string) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key); else next.add(key);
    setter(next);
  };

  const schemasOf = (connId: string, db: string) => schemaLists[`${connId}/${db}`];
  const selectedIn = (connId: string, db: string) => selection[connId]?.[db] ?? [];
  const setSchemas = (connId: string, db: string, list: string[]) =>
    setSelection((sel) => {
      const next = { ...sel, [connId]: { ...(sel[connId] ?? {}) } };
      if (list.length) next[connId][db] = list; else delete next[connId][db];
      if (!Object.keys(next[connId]).length) delete next[connId];
      return next;
    });
  const toggleSchema = (connId: string, db: string, schema: string) => {
    const cur = selectedIn(connId, db);
    const all = schemasOf(connId, db) ?? [];
    const next = cur.includes(schema) ? cur.filter((x) => x !== schema) : all.filter((x) => x === schema || cur.includes(x));
    setSchemas(connId, db, next);
  };
  const toggleDb = async (connId: string, db: string) => {
    const cur = selectedIn(connId, db);
    const all = schemasOf(connId, db);
    if (!all) {
      // Ainda não leu os schemas: lê agora e marca todos.
      try {
        const r = await pg.listSchemas(connId, db);
        const names = r.map((x) => (typeof x === "string" ? x : (x as { name: string }).name));
        setSchemaLists((l) => ({ ...l, [`${connId}/${db}`]: names }));
        setSchemas(connId, db, names);
      } catch (e) {
        setMessage(errorText(toAtlasError(e)));
      }
      return;
    }
    setSchemas(connId, db, cur.length === all.length ? [] : all);
  };
  const matches = (name: string) => !search.trim() || name.toLowerCase().includes(search.trim().toLowerCase());
  const setCheck = (el: HTMLInputElement | null, partial: boolean) => { if (el) el.indeterminate = partial; };

  const selectTree = (
    <div className="dd-tree" role="tree">
      {conns.map((c) => {
        const key = `c:${c.id}`;
        const open = expanded.has(key);
        const dbs = dbLists[c.id];
        const selectedDbs = dbs?.filter((d) => selectedIn(c.id, d).length > 0).length ?? Object.keys(selection[c.id] ?? {}).length;
        const total = dbs?.length;
        const visibleDbs = (dbs ?? []).filter((d) => matches(d) || (schemasOf(c.id, d) ?? []).some(matches) || matches(c.name));
        if (search.trim() && !matches(c.name) && !visibleDbs.length) return null;
        return (
          <div key={c.id}>
            <div className={`dd-node${focus?.kind === "server" && focus.connId === c.id ? " is-focus" : ""}`} onClick={() => setFocus({ kind: "server", connId: c.id })}>
              <button type="button" className="ss-chevron" aria-label={c.name} onClick={(e) => { e.stopPropagation(); toggle(expanded, setExpanded, key); }}><SmallIcon name={open ? "chevronDown" : "chevronRight"} size={10} /></button>
              <ProviderIcon provider={c.provider as never} open={c.open} />
              <span>{c.name}{total !== undefined ? ` (${selectedDbs}/${total})` : ""}</span>
              {connecting === c.id && <span className="spinner" />}
            </div>
            {open && visibleDbs.map((db) => {
              const dkey = `d:${c.id}/${db}`;
              const dopen = expanded.has(dkey);
              const chosen = selectedIn(c.id, db);
              const all = schemasOf(c.id, db);
              const partial = chosen.length > 0 && (!all || chosen.length < all.length);
              return (
                <div key={db}>
                  <div className={`dd-node${focus?.kind === "db" && focus.connId === c.id && focus.db === db ? " is-focus" : ""}`} style={{ paddingLeft: 22 }} onClick={() => setFocus({ kind: "db", connId: c.id, db })}>
                    <button type="button" className="ss-chevron" aria-label={db} onClick={(e) => { e.stopPropagation(); toggle(expanded, setExpanded, dkey); }}><SmallIcon name={dopen ? "chevronDown" : "chevronRight"} size={10} /></button>
                    <input type="checkbox" checked={chosen.length > 0} ref={(el) => setCheck(el, partial)} aria-label={db} onClick={(e) => e.stopPropagation()} onChange={() => void toggleDb(c.id, db)} />
                    <SmallIcon name="database" />
                    <span>{db}{partial ? " (*)" : ""}</span>
                  </div>
                  {dopen && (all ?? []).filter(matches).map((sc) => (
                    <div key={sc} className={`dd-node${focus?.kind === "schema" && focus.connId === c.id && focus.db === db && focus.schema === sc ? " is-focus" : ""}`} style={{ paddingLeft: 60 }} onClick={() => setFocus({ kind: "schema", connId: c.id, db, schema: sc })}>
                      <input type="checkbox" checked={chosen.includes(sc)} aria-label={sc} onClick={(e) => e.stopPropagation()} onChange={() => toggleSchema(c.id, db, sc)} />
                      <SmallIcon name="schema" />
                      <span>{sc}</span>
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        );
      })}
    </div>
  );

  /* ---------- Etapa 2: objetos e ordem ---------- */
  const buildPlan = async () => {
    const previous = plan;
    setPlanBusy(true);
    setMessage(null);
    try {
      const out: PlanServer[] = [];
      for (const c of conns) {
        const dbsSel = selection[c.id];
        if (!dbsSel) continue;
        const old = previous.find((p) => p.connId === c.id);
        const dbs: PlanDb[] = [];
        for (const db of dbLists[c.id]?.filter((d) => dbsSel[d]) ?? Object.keys(dbsSel)) {
          const oldDb = old?.dbs.find((d) => d.name === db);
          const schemas: PlanSchema[] = [];
          for (const schema of dbsSel[db]) {
            const prev = oldDb?.schemas.find((s) => s.name === schema);
            if (prev) { schemas.push(prev); continue; }
            const o = await pg.listObjects(c.id, db, schema);
            const lists: Record<Category, string[]> = {
              tables: o.tables.map((x) => x.name),
              views: [...o.views, ...o.matviews].map((x) => x.name),
              functions: o.functions.map((x) => `${x.name}(${x.args ?? ""})`),
            };
            schemas.push({ name: schema, info: true, cats: Object.fromEntries(CATEGORIES.map((k) => [k, { on: true, all: lists[k], picked: lists[k] }])) as Record<Category, PlanCategory> });
          }
          // A ordem que o usuário escolheu antes vale; os novos vão para o fim.
          const order = oldDb ? oldDb.schemas.map((s) => s.name) : [];
          schemas.sort((a, b) => (order.indexOf(a.name) + 1 || 1e6) - (order.indexOf(b.name) + 1 || 1e6));
          dbs.push({ name: db, info: oldDb?.info ?? true, schemas });
        }
        out.push({ connId: c.id, info: old?.info ?? true, dbs });
      }
      setPlan(out);
      setPlanExpanded(new Set());
      return out;
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
      return null;
    } finally {
      setPlanBusy(false);
    }
  };

  const patchServer = (connId: string, p: (s: PlanServer) => PlanServer) => setPlan((all) => all.map((s) => (s.connId === connId ? p(s) : s)));
  const patchDb = (connId: string, db: string, p: (d: PlanDb) => PlanDb) => patchServer(connId, (s) => ({ ...s, dbs: s.dbs.map((d) => (d.name === db ? p(d) : d)) }));
  const patchSchema = (connId: string, db: string, schema: string, p: (x: PlanSchema) => PlanSchema) => patchDb(connId, db, (d) => ({ ...d, schemas: d.schemas.map((x) => (x.name === schema ? p(x) : x)) }));

  const move = (dir: -1 | 1) => {
    if (!focus) return;
    const swap = <T,>(list: T[], i: number): T[] => {
      const j = i + dir;
      if (i < 0 || j < 0 || j >= list.length) return list;
      const next = [...list];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    };
    if (focus.kind === "server") setPlan((all) => swap(all, all.findIndex((s) => s.connId === focus.connId)));
    else if (focus.kind === "db") patchServer(focus.connId, (s) => ({ ...s, dbs: swap(s.dbs, s.dbs.findIndex((d) => d.name === focus.db)) }));
    else patchDb(focus.connId, focus.db, (d) => ({ ...d, schemas: swap(d.schemas, d.schemas.findIndex((x) => x.name === focus.schema)) }));
  };

  const setCategory = (connId: string, db: string, schema: string, cat: Category, p: (c: PlanCategory) => PlanCategory) =>
    patchSchema(connId, db, schema, (x) => ({ ...x, cats: { ...x.cats, [cat]: p(x.cats[cat]) } }));

  const catLabel = (c: PlanCategory) => (!c.on || c.picked.length === c.all.length ? "(*)" : `(${c.picked.length}/${c.all.length})`);

  const planTree = (
    <div className="dd-tree" role="tree">
      {plan.map((srv) => {
        const c = connOf(srv.connId);
        const isFocus = (f: Focus | null, k: Focus) => JSON.stringify(f) === JSON.stringify(k);
        return (
          <div key={srv.connId}>
            <div className={`dd-node${isFocus(focus, { kind: "server", connId: srv.connId }) ? " is-focus" : ""}`} onClick={() => setFocus({ kind: "server", connId: srv.connId })}>
              <span className="dd-gap" />
              {c && <ProviderIcon provider={c.provider as never} open />}
              <span>{c?.name}</span>
            </div>
            <label className="dd-node dd-sub" style={{ paddingLeft: 22 }}>
              <input type="checkbox" checked={srv.info} onChange={(e) => patchServer(srv.connId, (s) => ({ ...s, info: e.target.checked }))} />
              <SmallIcon name="preview" /><span>{t("dd.serverInfo")}</span>
            </label>
            {srv.dbs.map((db) => (
              <div key={db.name}>
                <div className={`dd-node${isFocus(focus, { kind: "db", connId: srv.connId, db: db.name }) ? " is-focus" : ""}`} style={{ paddingLeft: 22 }} onClick={() => setFocus({ kind: "db", connId: srv.connId, db: db.name })}>
                  <span className="dd-gap" /><SmallIcon name="database" /><span>{db.name}</span>
                </div>
                <label className="dd-node dd-sub" style={{ paddingLeft: 44 }}>
                  <input type="checkbox" checked={db.info} onChange={(e) => patchDb(srv.connId, db.name, (d) => ({ ...d, info: e.target.checked }))} />
                  <SmallIcon name="preview" /><span>{t("dd.databaseInfo")}</span>
                </label>
                {db.schemas.map((sc) => (
                  <div key={sc.name}>
                    <div className={`dd-node${isFocus(focus, { kind: "schema", connId: srv.connId, db: db.name, schema: sc.name }) ? " is-focus" : ""}`} style={{ paddingLeft: 40 }} onClick={() => setFocus({ kind: "schema", connId: srv.connId, db: db.name, schema: sc.name })}>
                      <span className="dd-gap" /><SmallIcon name="schema" /><span>{sc.name}</span>
                    </div>
                    <label className="dd-node dd-sub" style={{ paddingLeft: 62 }}>
                      <input type="checkbox" checked={sc.info} onChange={(e) => patchSchema(srv.connId, db.name, sc.name, (x) => ({ ...x, info: e.target.checked }))} />
                      <SmallIcon name="preview" /><span>{t("dd.schemaInfo")}</span>
                    </label>
                    {CATEGORIES.map((cat) => {
                      const c2 = sc.cats[cat];
                      const key = `${srv.connId}/${db.name}/${sc.name}/${cat}`;
                      const open = planExpanded.has(key);
                      const partial = c2.on && c2.picked.length > 0 && c2.picked.length < c2.all.length;
                      return (
                        <div key={cat}>
                          <div className="dd-node" style={{ paddingLeft: 62 }}>
                            <button type="button" className="ss-chevron" aria-label={cat} onClick={() => toggle(planExpanded, setPlanExpanded, key)}><SmallIcon name={open ? "chevronDown" : "chevronRight"} size={10} /></button>
                            <input type="checkbox" checked={c2.on && c2.picked.length > 0} ref={(el) => setCheck(el, partial)} aria-label={t(`dd.cat.${cat}` as never)}
                              onChange={(e) => setCategory(srv.connId, db.name, sc.name, cat, (x) => ({ ...x, on: e.target.checked, picked: e.target.checked ? x.all : [] }))} />
                            <SmallIcon name={cat === "tables" ? "table" : cat === "views" ? "view" : "function"} />
                            <span>{t(`dd.cat.${cat}` as never)} {catLabel(c2)}</span>
                          </div>
                          {open && c2.all.filter(matches).map((name) => (
                            <label key={name} className="dd-node dd-sub" style={{ paddingLeft: 100 }}>
                              <input type="checkbox" checked={c2.picked.includes(name)}
                                onChange={(e) => setCategory(srv.connId, db.name, sc.name, cat, (x) => {
                                  const picked = e.target.checked ? x.all.filter((n) => n === name || x.picked.includes(n)) : x.picked.filter((n) => n !== name);
                                  return { ...x, picked, on: picked.length > 0 };
                                })} />
                              <SmallIcon name={cat === "tables" ? "table" : cat === "views" ? "view" : "function"} /><span>{name}</span>
                            </label>
                          ))}
                        </div>
                      );
                    })}
                  </div>
                ))}
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );

  /* ---------- O documento ---------- */
  const servers: DictServerSel[] = useMemo(
    () => plan.map((s) => ({
      connId: s.connId,
      name: conns.find((c) => c.id === s.connId)?.name ?? s.connId,
      product: "PostgreSQL",
      info: s.info ? serverInfo(s.connId) : null,
      databases: s.dbs.map((d) => ({
        name: d.name,
        info: d.info,
        schemas: d.schemas.map((x) => ({
          name: x.name,
          info: x.info,
          tables: x.cats.tables.on ? x.cats.tables.picked : null,
          views: x.cats.views.on ? x.cats.views.picked : null,
          functions: x.cats.functions.on ? x.cats.functions.picked : null,
        })),
      })),
    })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [plan, conns],
  );

  const goNext = async () => {
    setMenu(null);
    setMessage(null);
    if (step === 0) {
      if (!Object.keys(selection).length) { setMessage(t("dd.needSchema")); return; }
      const p = await buildPlan();
      if (!p) return;
      const first = p[0]?.dbs[0]?.name ?? "";
      if (!defaults) {
        const d = await pg.dictDefaults(first);
        setDefaults(d);
        setSettings({ ...d, now: nowParts() });
      }
      setStep(1);
    } else if (step === 1) {
      const any = plan.some((s) => s.info || s.dbs.some((d) => d.info || d.schemas.some((x) => x.info || CATEGORIES.some((k) => x.cats[k].on && x.cats[k].picked.length))));
      if (!any) { setMessage(t("dd.needObject")); return; }
      setStep(2);
    } else if (step === 2 || step === 3) {
      setStep(step + 1);
    }
  };

  const chooseFile = async () => {
    try {
      const picked = await chooseExportPath(path.split(/[\\/]/).pop() || "DataDictionary.pdf", ["pdf"]);
      if (picked) setPath(picked);
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  };

  const start = async () => {
    if (!settings) return;
    if (!path.trim()) { setMessage(t("dd.needPath")); return; }
    const id = crypto.randomUUID();
    runId.current = id;
    setLog([]);
    setCounts({ tables: 0, views: 0, functions: 0, errors: 0, fraction: 0 });
    setOutputPath(null);
    setFinished(false);
    setElapsed(0);
    setMessage(null);
    startedAt.current = performance.now();
    setStep(5);
    setRunning(true);
    const unlistenLog = await onDictLog(id, (line) => setLog((l) => [...l, line]));
    const unlistenProgress = await onDictProgress(id, setCounts);
    try {
      const r = await pg.dictGenerate({ runId: id, servers, settings: { ...settings, now: nowParts() }, path, appendTimestamp, overwrite, continueOnError });
      setCounts(r);
      setOutputPath(r.path);
    } catch (e) {
      const err = toAtlasError(e);
      setLog((l) => [...l, err.kind === "canceled" ? `[DDI] ${t("dt.log.canceled")}` : `[DDI] ERROR: ${errorText(err)}`]);
    } finally {
      unlistenLog();
      unlistenProgress();
    }
    setElapsed(performance.now() - startedAt.current);
    setRunning(false);
    setFinished(true);
  };

  const pct = Math.round(counts.fraction * 100);
  useEffect(() => {
    void getCurrentWindow().setTitle(step === 5 ? `${running ? `${pct}% - ` : "* "}${t("dd.title")}` : `* ${t("dd.title")}`).catch(() => undefined);
  }, [step, pct, running, t]);

  /* ---------- Perfis ---------- */
  const persist = (next: Profile[]) => { setProfiles(next); void local.setPref(PROFILES_KEY, next).catch(() => undefined); };
  const saveProfile = (name: string) => {
    if (!settings) { setNamePrompt(false); return; }
    persist([...profiles.filter((x) => x.name !== name), { name, selection, plan, settings, path, appendTimestamp, overwrite, continueOnError }].sort((a, b) => a.name.localeCompare(b.name)));
    setNamePrompt(false);
  };
  const loadProfile = (p: Profile) => {
    setSelection(p.selection);
    setPlan(p.plan);
    setSettings({ ...p.settings, now: nowParts() });
    if (!defaults) void pg.dictDefaults(p.settings.coverTitle).then(setDefaults).catch(() => undefined);
    setPath(p.path);
    setAppendTimestamp(p.appendTimestamp);
    setOverwrite(p.overwrite);
    setContinueOnError(p.continueOnError);
    setExpanded(new Set(Object.keys(p.selection).flatMap((id) => [`c:${id}`, ...Object.keys(p.selection[id]).map((d) => `d:${id}/${d}`)])));
    setStep(0);
  };

  const saveItems: MenuEntry[] = [
    { kind: "item", text: t("dt.saveAs"), run: () => setNamePrompt(true) },
    ...(profiles.length ? [{ kind: "sep" } as MenuEntry, ...profiles.map((p): MenuEntry => ({ kind: "item", text: t("exp.profileDelete", { name: p.name }), run: () => persist(profiles.filter((x) => x.name !== p.name)) }))] : []),
  ];
  const loadItems: MenuEntry[] = profiles.length ? profiles.map((p): MenuEntry => ({ kind: "item", text: p.name, run: () => loadProfile(p) })) : [{ kind: "item", text: t("exp.profileNone") }];
  const openItems: MenuEntry[] = [
    { kind: "item", text: t("dd.openFile"), run: () => outputPath && void pg.dictOpen(outputPath, false).catch((e) => setMessage(errorText(toAtlasError(e)))) },
    { kind: "item", text: t("dd.openFolder"), run: () => outputPath && void pg.dictOpen(outputPath, true).catch((e) => setMessage(errorText(toAtlasError(e)))) },
  ];

  const infoPanel = (
    <aside className="dd-info">
      <div className="dd-info-icon"><SmallIcon name="preview" size={14} /></div>
      {info && (
        <>
          <div className="dd-info-head">
            {focus?.kind === "server" ? <ProviderIcon provider={(connOf(focus.connId)?.provider ?? "postgresql") as never} open /> : <SmallIcon name={focus?.kind === "schema" ? "schema" : "database"} size={44} />}
            <div><strong>{info.title}</strong><div className="muted">{info.kind}</div></div>
          </div>
          <dl className="dd-info-list">
            {info.rows.map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{v}</dd></div>)}
          </dl>
        </>
      )}
    </aside>
  );

  const bottomBar = (
    <div className="dd-treebar">
      {step === 1 && (
        <>
          <button type="button" className="icon-button" aria-label="↑" disabled={!focus} onClick={() => move(-1)}>↑</button>
          <button type="button" className="icon-button" aria-label="↓" disabled={!focus} onClick={() => move(1)}>↓</button>
        </>
      )}
      <SmallIcon name="search" size={12} />
      <input className="dd-search" placeholder={t("dd.search")} value={search} onChange={(e) => setSearch(e.target.value)} spellCheck={false} />
    </div>
  );

  const heading = step <= 4 ? <div className="dd-title">{t(STEP_TITLES[step])}</div> : null;
  const templateStep = settings && <TemplateStep servers={servers} settings={settings} onPick={(tp: DictTemplate) => setSettings({ ...settings, template: tp })} />;
  const layoutStep = settings && defaults && <LayoutStep servers={servers} settings={settings} setSettings={setSettings} defaults={defaults} />;
  const stamp = stampOf(nowParts());
  const baseName = (path.split(/[\\/]/).pop() ?? "DataDictionary.pdf").replace(/\.pdf$/i, "");

  const pathStep = (
    <div className="dd-path">
      <label className="dd-path-row">
        <span>{t("dd.exportTo")}</span>
        <input className="field" value={path} spellCheck={false} onChange={(e) => setPath(e.target.value)} />
        <button type="button" className="btn" onClick={() => void chooseFile()}>…</button>
      </label>
      <label className="check-row"><input type="checkbox" checked={appendTimestamp} onChange={(e) => setAppendTimestamp(e.target.checked)} /><span>{t("dd.appendStamp")}</span><span className="muted"> ({baseName}_{stamp})</span></label>
      <label className="check-row"><input type="checkbox" checked={overwrite} onChange={(e) => setOverwrite(e.target.checked)} /><span>{t("dd.overwrite")}</span></label>
      <label className="check-row dd-gap-top"><input type="checkbox" checked={continueOnError} onChange={(e) => setContinueOnError(e.target.checked)} /><span>{t("ds.opt.continue")}</span></label>
    </div>
  );

  const runStep = (
    <div className="dd-run">
      <dl className="dt-stats">
        <dt>{t("dd.objects")}</dt><dd>{t("dd.objectsValue", { tables: counts.tables, views: counts.views, functions: counts.functions })}</dd>
        <dt>{t("dd.errors")}</dt><dd>{counts.errors}</dd>
        <dt>{t("exp.time").replace(":", "")}:</dt><dd>{formatClock(elapsed)}</dd>
      </dl>
      <div className="exp-log dt-log dd-log" ref={logRef} role="log" aria-live="polite">
        {log.map((l, i) => <div key={i} className={l.includes("ERROR") ? "is-error" : undefined}>{l}</div>)}
      </div>
      <div className="exp-progress dd-progress"><div className="exp-progress-bar" style={{ width: `${pct}%` }} /></div>
    </div>
  );

  return (
    <div className="dt-window dd-window">
      {heading}
      <div className="dt-body dd-body">
        {step === 0 && (
          <div className="dd-split">
            <div className="dd-tree-wrap">{selectTree}{bottomBar}</div>
            {infoPanel}
          </div>
        )}
        {step === 1 && (
          <div className="dd-split">
            <div className="dd-tree-wrap">
              {planBusy ? <div className="muted dt-loading"><span className="spinner" />{t("dt.reading")}</div> : planTree}
              {bottomBar}
            </div>
            {infoPanel}
          </div>
        )}
        {step === 2 && templateStep}
        {step === 3 && layoutStep}
        {step === 4 && pathStep}
        {step === 5 && runStep}
        {message && <div className="msg-error dt-message" role="alert">{message}</div>}
      </div>
      <div className="dt-footer">
        <div className="dt-menu-wrap split-button">
          <button type="button" className="btn dt-wide" disabled={running} onClick={() => setNamePrompt(true)}>{t("exp.saveProfile")}</button>
          <button type="button" className="btn dt-arrow-btn" aria-label={t("exp.saveProfile")} disabled={running} onClick={() => setMenu((m) => (m === "save" ? null : "save"))}><SmallIcon name="chevronDown" size={10} /></button>
          {menu === "save" && <PopupMenu items={saveItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%" }} />}
        </div>
        <div className="dt-menu-wrap">
          <button type="button" className="btn dt-wide" disabled={running} onClick={() => setMenu((m) => (m === "load" ? null : "load"))}>{t("dt.loadProfile")}</button>
          {menu === "load" && <PopupMenu items={loadItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%", left: 0 }} />}
        </div>
        <span className="spacer" />
        {step === 5 && finished && outputPath && (
          <div className="dt-menu-wrap split-button">
            <button type="button" className="btn dt-wide" onClick={() => void pg.dictOpen(outputPath, false).catch((e) => setMessage(errorText(toAtlasError(e))))}>{t("dd.open")}</button>
            <button type="button" className="btn dt-arrow-btn" aria-label={t("dd.open")} onClick={() => setMenu((m) => (m === "open" ? null : "open"))}><SmallIcon name="chevronDown" size={10} /></button>
            {menu === "open" && <PopupMenu items={openItems} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%", right: 0, left: "auto" }} />}
          </div>
        )}
        {step > 0 && <button type="button" className="btn dt-wide" disabled={running} onClick={() => { setMessage(null); setStep(step === 5 ? 4 : step - 1); }}>{t("exp.back").replace("< ", "")}</button>}
        {step <= 3 && <button type="button" className="btn btn-primary dt-wide" disabled={planBusy || !!connecting} onClick={() => void goNext()}>{t("exp.next").replace(" >", "")}</button>}
        {step === 4 && <button type="button" className="btn btn-primary dt-wide" onClick={() => void start()}>{t("exp.start")}</button>}
        {step === 5 && (running
          ? <button type="button" className="btn btn-primary dt-wide" onClick={() => void pg.dictCancel(runId.current).catch(() => undefined)}>{t("exp.stop")}</button>
          : <button type="button" className="btn btn-primary dt-wide" onClick={() => void getCurrentWindow().close()}>{t("common.close")}</button>)}
      </div>
      {pwPrompt && (
        <MessageDialog
          title={t("dd.title")}
          message={[t("dt.passwordFor", { name: connOf(pwPrompt.connId)?.name ?? "" }), pwPrompt.retry].filter(Boolean).join("\n\n")}
          prompt={{ label: t("dt.password"), initial: "" }}
          secret
          allowEmpty
          onOk={(pw) => { const id = pwPrompt.connId; setPwPrompt(null); void openConn(id, pw ?? ""); }}
          onCancel={() => setPwPrompt(null)}
        />
      )}
      {namePrompt && <MessageDialog title={t("exp.saveProfile")} prompt={{ label: t("exp.profileName"), initial: "" }} okLabel={t("exp.saveProfile")} onOk={(n) => { if (n) saveProfile(n); }} onCancel={() => setNamePrompt(false)} />}
    </div>
  );
}
