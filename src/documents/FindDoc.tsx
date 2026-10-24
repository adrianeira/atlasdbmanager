import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import {
  errorText, pg, toAtlasError, type FindDataResult, type FindMatch, type FindMode, type FindObject, type FindOptions, type FindResult, type FindWhat,
} from "../backend";
import type { StatusCell } from "../components/StatusBar";
import { useT, type MessageKey } from "../i18n";
import { ProviderIcon, SmallIcon, type SmallIconName } from "../icons";

export interface FindConn {
  id: string;
  name: string;
  provider: string;
  open: boolean;
}

interface Props {
  conns: FindConn[];
  /** Escopo marcado ao abrir (a partir de um banco ou schema da árvore). */
  initial: { connId?: string; database?: string; schema?: string };
  /** Abre uma conexão fechada (pede a senha se faltar). */
  onConnect: (id: string) => void;
  /** Duplo clique num achado: abre o objeto (`what` diz se foi um campo, para abrir o designer). */
  onOpen: (connId: string, database: string, schema: string, object: { kind: FindObject["kind"] | "table"; name: string; identity: string | null }, what?: FindWhat) => void;
  active: boolean;
  onStatus: (cells: StatusCell[]) => void;
}

type LookIn = "structure" | "data";

const WHAT_LABEL: Record<FindWhat, MessageKey> = {
  tableName: "fd.what.tableName", fieldName: "fd.what.fieldName", foreignKeyName: "fd.what.foreignKeyName", indexName: "fd.what.indexName",
  triggerName: "fd.what.triggerName", viewName: "fd.what.viewName", viewBody: "fd.what.viewBody", matviewName: "fd.what.matviewName",
  matviewBody: "fd.what.matviewBody", functionName: "fd.what.functionName", functionBody: "fd.what.functionBody",
};
const KIND_ICON: Record<FindObject["kind"], SmallIconName> = { table: "table", view: "view", matview: "matview", function: "function" };

/** O texto com os trechos achados em vermelho. */
function Highlighted({ match }: { match: FindMatch }) {
  const chars = [...match.text];
  const parts: React.ReactNode[] = [];
  let at = 0;
  for (const [a, b] of match.ranges) {
    if (a > at) parts.push(chars.slice(at, a).join(""));
    parts.push(<span key={a} className="fd-hit">{chars.slice(a, b).join("")}</span>);
    at = b;
  }
  if (at < chars.length) parts.push(chars.slice(at).join(""));
  return <>{parts}</>;
}

const label = (o: FindObject, withSchema: boolean) => `${withSchema ? `${o.schema}.` : ""}${o.name}`;

export function FindDoc({ conns, initial, onConnect, onOpen, active, onStatus }: Props) {
  const t = useT();
  const [connId, setConnId] = useState(initial.connId ?? "");
  const [database, setDatabase] = useState(initial.database ?? "");
  const [schema, setSchema] = useState(initial.schema ?? "");
  const [databases, setDatabases] = useState<string[]>([]);
  const [schemas, setSchemas] = useState<string[]>([]);
  const [text, setText] = useState("");
  const [lookIn, setLookIn] = useState<LookIn>("structure");
  const [mode, setMode] = useState<FindMode>("contains");
  const [caseInsensitive, setCaseInsensitive] = useState(false);
  const [inc, setInc] = useState({ tables: true, views: true, functions: true, indexes: true, triggers: true, matviews: true });
  const [structure, setStructure] = useState<FindResult | null>(null);
  const [data, setData] = useState<FindDataResult | null>(null);
  /** O que foi procurado (o texto do rótulo dos resultados não muda enquanto se digita outro). */
  const [searched, setSearched] = useState<{ text: string; lookIn: LookIn; schema: string } | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const findId = useRef("");
  const seq = useRef(0);

  const conn = conns.find((c) => c.id === connId);
  const open = !!conn?.open;

  // A conexão marcada que ainda está fechada é aberta.
  const asked = useRef(new Set<string>());
  useEffect(() => {
    if (conn && !conn.open && !asked.current.has(conn.id)) {
      asked.current.add(conn.id);
      onConnect(conn.id);
    }
  }, [conn, onConnect]);

  useEffect(() => {
    if (!connId || !open) return setDatabases([]);
    let alive = true;
    pg.listDatabases(connId)
      .then((r) => alive && setDatabases(r.databases))
      .catch(() => alive && setDatabases([]));
    return () => { alive = false; };
  }, [connId, open]);

  useEffect(() => {
    if (!connId || !open || !database) return setSchemas([]);
    let alive = true;
    pg.listSchemas(connId, database)
      .then((r) => alive && setSchemas(r.map((s) => s.name)))
      .catch(() => alive && setSchemas([]));
    return () => { alive = false; };
  }, [connId, open, database]);

  const canFind = open && database !== "" && text.trim() !== "" && !busy;
  const structureOk = inc.tables || inc.views || inc.functions || inc.indexes || inc.triggers || inc.matviews;

  const run = () => {
    if (!canFind || (lookIn === "structure" && !structureOk)) return;
    const mine = ++seq.current;
    setBusy(true);
    setError(null);
    findId.current = `find-${mine}-${Date.now()}`;
    const options: FindOptions = {
      text, schema: schema || null, mode, caseInsensitive,
      tables: inc.tables, views: inc.views, matviews: inc.matviews, functions: inc.functions, indexes: inc.indexes, triggers: inc.triggers,
    };
    const scope = { text, lookIn, schema };
    const done = () => mine === seq.current && setBusy(false);
    if (lookIn === "structure") {
      pg.find(connId, database, options)
        .then((r) => {
          if (mine !== seq.current) return;
          setStructure(r);
          setData(null);
          setSearched(scope);
          setCollapsed(new Set());
          setSelected(null);
        })
        .catch((e) => mine === seq.current && setError(errorText(toAtlasError(e))))
        .finally(done);
    } else {
      pg.findData(connId, database, findId.current, options)
        .then((r) => {
          if (mine !== seq.current) return;
          setData(r);
          setStructure(null);
          setSearched(scope);
          setSelected(null);
        })
        .catch((e) => mine === seq.current && setError(errorText(toAtlasError(e))))
        .finally(done);
    }
  };
  const stop = () => void pg.exportCancel(findId.current).catch(() => undefined);

  // Rodapé: o resultado da última busca.
  const status = useCallback((): string => {
    if (error) return "";
    if (busy) return t("fd.searching");
    if (structure && searched?.lookIn === "structure") {
      const extra = structure.truncated ? ` ${t("fd.truncated")}` : "";
      return `${t("fd.found", { objects: structure.objects.length, n: structure.occurrences })}${extra}`;
    }
    if (data && searched?.lookIn === "data") {
      const records = data.tables.reduce((n, x) => n + x.count, 0);
      const extra = data.cancelled ? ` ${t("fd.stopped", { scanned: data.scanned, total: data.total })}` : data.skipped > 0 ? ` ${t("fd.skipped", { n: data.skipped })}` : "";
      return `${t("fd.foundData", { tables: data.tables.length, n: records })}${extra}`;
    }
    return t("fd.ready");
  }, [error, busy, structure, data, searched, t]);
  const statusText = status();
  useEffect(() => {
    if (active) onStatus([{ text: statusText, grow: true }]);
  }, [active, statusText, onStatus]);

  const withSchema = schema === "";
  const toggle = (key: string) => setCollapsed((c) => { const n = new Set(c); if (n.has(key)) n.delete(key); else n.add(key); return n; });
  const openObject = (o: FindObject, what?: FindWhat) => onOpen(connId, database, o.schema, { kind: o.kind, name: o.name, identity: o.identity }, what);

  const incBox = (key: keyof typeof inc, text: MessageKey, disabled = false) => (
    <label className={`check-row${disabled ? " is-disabled" : ""}`} title={disabled ? t("fd.queriesNote") : undefined}>
      <input type="checkbox" checked={disabled ? true : inc[key]} disabled={disabled} onChange={(e) => setInc((x) => ({ ...x, [key]: e.target.checked }))} />
      <span>{t(text)}</span>
    </label>
  );

  const resultTitle = searched?.lookIn === "structure" ? t("fd.resultsFor", { text: searched.text }) : t("fd.results");

  return (
    <div className="fd-doc">
      <div className="fd-left">
        <div className="fd-field">
          <label htmlFor="fd-conn">{t("fd.connection")}:</label>
          <div className="fd-select">
            {conn && <ProviderIcon provider={conn.provider as never} open={conn.open} />}
            <select id="fd-conn" className="field" value={connId} onChange={(e) => { setConnId(e.target.value); setDatabase(""); setSchema(""); }}>
              <option value="" />
              {conns.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
            </select>
          </div>
        </div>
        <div className="fd-field">
          <label htmlFor="fd-db">{t("fd.database")}:</label>
          <div className="fd-select">
            {database && <SmallIcon name="database" />}
            <select id="fd-db" className="field" value={database} onChange={(e) => { setDatabase(e.target.value); setSchema(""); }}>
              <option value="" />
              {database && !databases.includes(database) && <option value={database}>{database}</option>}
              {databases.map((d) => <option key={d} value={d}>{d}</option>)}
            </select>
          </div>
        </div>
        <div className="fd-field">
          <label htmlFor="fd-schema">{t("fd.schema")}:</label>
          <div className="fd-select">
            {schema && <SmallIcon name="schema" />}
            <select id="fd-schema" className="field" value={schema} onChange={(e) => setSchema(e.target.value)}>
              <option value="" />
              {schema && !schemas.includes(schema) && <option value={schema}>{schema}</option>}
              {schemas.map((s) => <option key={s} value={s}>{s}</option>)}
            </select>
          </div>
        </div>

        <div className="fd-field fd-gap">
          <label htmlFor="fd-text">{t("fd.findWhat")}:</label>
          <input id="fd-text" className="field" value={text} spellCheck={false} autoComplete="off" onChange={(e) => setText(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") run(); }} autoFocus />
        </div>
        <div className="fd-field">
          <div className="fd-label-row">
            <label htmlFor="fd-look">{t("fd.lookIn")}:</label>
            {lookIn === "data" && <span className="muted">{t("fd.escapeNote")}</span>}
          </div>
          <select id="fd-look" className="field fd-short" value={lookIn} onChange={(e) => setLookIn(e.target.value as LookIn)}>
            <option value="data">{t("fd.lookData")}</option>
            <option value="structure">{t("fd.lookStructure")}</option>
          </select>
        </div>
        <div className="fd-field">
          <label htmlFor="fd-mode">{t("fd.searchMode")}:</label>
          <select id="fd-mode" className="field fd-short" value={mode} onChange={(e) => setMode(e.target.value as FindMode)}>
            <option value="contains">{t("fd.modeContains")}</option>
            <option value="whole">{t("fd.modeWhole")}</option>
            <option value="prefix">{t("fd.modePrefix")}</option>
            <option value="regex">{t("fd.modeRegex")}</option>
          </select>
        </div>
        <label className="check-row fd-ci"><input type="checkbox" checked={caseInsensitive} onChange={(e) => setCaseInsensitive(e.target.checked)} /><span>{t("fd.caseInsensitive")}</span></label>

        {lookIn === "structure" && (
          <div className="fd-include">
            <div className="fd-include-title">{t("fd.include")}:</div>
            <div className="fd-include-grid">
              {incBox("tables", "fd.incTables")}
              {incBox("views", "fd.incViews")}
              {incBox("functions", "fd.incFunctions")}
              {incBox("tables", "fd.incQueries", true)}
              {incBox("indexes", "fd.incIndexes")}
              {incBox("triggers", "fd.incTriggers")}
              {incBox("matviews", "fd.incMatviews")}
            </div>
          </div>
        )}

        <div className="fd-actions">
          {busy && lookIn === "data"
            ? <button type="button" className="btn" onClick={stop}>{t("fd.stop")}</button>
            : <button type="button" className="btn" disabled={!canFind || (lookIn === "structure" && !structureOk)} onClick={run}>{t("fd.find")}</button>}
        </div>
      </div>

      <div className="fd-right">
        <div className="fd-results-title">{resultTitle}</div>
        {error && <div className="doc-banner is-warn">{error}</div>}
        <div className="fd-results" role="tree" aria-label={resultTitle}>
          <div className="fd-grid fd-head">
            <span>{t("fd.colName")}</span>
            <span>{searched?.lookIn === "data" || (!searched && lookIn === "data") ? t("fd.colCount") : t("fd.colMatched")}</span>
          </div>
          {busy && <div className="hist-empty fd-busy"><span className="spinner" />{t("fd.searching")}</div>}
          {!busy && structure && searched?.lookIn === "structure" && structure.objects.length === 0 && <div className="hist-empty">{t("fd.none", { text: searched.text })}</div>}
          {!busy && data && searched?.lookIn === "data" && data.tables.length === 0 && <div className="hist-empty">{t("fd.none", { text: searched.text })}</div>}
          {!busy && structure && searched?.lookIn === "structure" && structure.objects.map((o) => {
            const key = `${o.schema}.${o.kind}.${o.name}(${o.identity ?? ""})`;
            const closed = collapsed.has(key);
            return (
              <Fragment key={key}>
                <div
                  className={`fd-grid fd-row fd-group${selected === key ? " is-selected" : ""}`}
                  role="treeitem"
                  aria-expanded={!closed}
                  onClick={() => setSelected(key)}
                  onDoubleClick={() => openObject(o)}
                >
                  <span className="fd-name">
                    <button type="button" className="fd-chevron" aria-label={closed ? "+" : "−"} onClick={(e) => { e.stopPropagation(); toggle(key); }}>
                      <SmallIcon name={closed ? "chevronRight" : "chevronDown"} size={12} />
                    </button>
                    <SmallIcon name={KIND_ICON[o.kind]} />
                    <span>{label(o, withSchema)} ({o.matches.length})</span>
                  </span>
                  <span />
                </div>
                {!closed && o.matches.map((m, i) => {
                  const mk = `${key}#${i}`;
                  return (
                    <div key={mk} className={`fd-grid fd-row fd-child${selected === mk ? " is-selected" : ""}`} role="treeitem" onClick={() => setSelected(mk)} onDoubleClick={() => openObject(o, m.what)}>
                      <span className="fd-name fd-indent">{t(WHAT_LABEL[m.what])}</span>
                      <span className="fd-content"><Highlighted match={m} /></span>
                    </div>
                  );
                })}
              </Fragment>
            );
          })}
          {!busy && data && searched?.lookIn === "data" && data.tables.map((x) => {
            const key = `${x.schema}.${x.table}`;
            return (
              <div
                key={key}
                className={`fd-grid fd-row fd-group${selected === key ? " is-selected" : ""}`}
                role="treeitem"
                onClick={() => setSelected(key)}
                onDoubleClick={() => onOpen(connId, database, x.schema, { kind: "table", name: x.table, identity: null })}
              >
                <span className="fd-name"><SmallIcon name="table" /><span>{withSchema ? key : x.table}</span></span>
                <span>{x.count}</span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
