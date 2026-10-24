import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { errorText, pg, toAtlasError, type MonitorProcess, type MonitorSnapshot } from "../backend";
import { MessageDialog } from "../components/MessageDialog";
import type { StatusCell } from "../components/StatusBar";
import { useT } from "../i18n";
import { ProviderIcon, SmallIcon } from "../icons";

export interface MonitorConn {
  id: string;
  name: string;
  provider: string;
  open: boolean;
}

interface Props {
  conns: MonitorConn[];
  /** Conexão marcada ao abrir. */
  initial?: string;
  /** Abre uma conexão fechada (pede a senha se faltar). */
  onConnect: (id: string) => void;
  active: boolean;
  onStatus: (cells: StatusCell[]) => void;
}

type SubTab = "process" | "variables" | "status";
type Row = MonitorProcess & { server: string; connId: string };

const REFRESH_SECONDS = 5;
const PROCESS_COLUMNS: { key: keyof Row; label: string; width: number }[] = [
  { key: "server", label: "Server", width: 130 },
  { key: "id", label: "Id", width: 66 },
  { key: "user", label: "User", width: 80 },
  { key: "host", label: "Host", width: 70 },
  { key: "port", label: "Port", width: 60 },
  { key: "db", label: "db", width: 70 },
  { key: "command", label: "Command", width: 230 },
  { key: "state", label: "State", width: 230 },
  { key: "time", label: "Time", width: 230 },
  { key: "info", label: "Info", width: 230 },
];

const text = (v: unknown) => (v === null || v === undefined ? "" : String(v));

export function ServerMonitorDoc({ conns, initial, onConnect, active, onStatus }: Props) {
  const t = useT();
  const [checked, setChecked] = useState<Set<string>>(() => {
    const first = conns.find((c) => c.id === initial && c.open) ?? conns.find((c) => c.open) ?? conns.find((c) => c.id === initial) ?? conns[0];
    return new Set(first ? [first.id] : []);
  });
  // A conexão marcada que ainda está fechada é aberta.
  const asked = useRef(new Set<string>());
  useEffect(() => {
    for (const c of conns) {
      if (checked.has(c.id) && !c.open && !asked.current.has(c.id)) {
        asked.current.add(c.id);
        onConnect(c.id);
      }
    }
  }, [conns, checked, onConnect]);
  const [tab, setTab] = useState<SubTab>("process");
  const [data, setData] = useState<Record<string, MonitorSnapshot>>({});
  const [errors, setErrors] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [sort, setSort] = useState<{ key: keyof Row; dir: 1 | -1 } | null>(null);
  const [column, setColumn] = useState<keyof Row>("id");
  const [search, setSearch] = useState("");
  const [killing, setKilling] = useState<Row | null>(null);
  const [killError, setKillError] = useState<string | null>(null);
  const busy = useRef(false);

  const ids = useMemo(() => conns.filter((c) => checked.has(c.id) && c.open).map((c) => c.id), [conns, checked]);

  const refresh = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    try {
      const results = await Promise.allSettled(ids.map((id) => pg.monitorSnapshot(id)));
      const next: Record<string, MonitorSnapshot> = {};
      const problems: string[] = [];
      results.forEach((r, i) => {
        if (r.status === "fulfilled") next[ids[i]] = r.value;
        else problems.push(`${conns.find((c) => c.id === ids[i])?.name}: ${errorText(toAtlasError(r.reason))}`);
      });
      setData(next);
      setErrors(problems.length ? problems.join("\n") : null);
    } finally {
      busy.current = false;
    }
  }, [ids, conns]);

  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => {
    if (!active) return;
    const id = window.setInterval(() => void refresh(), REFRESH_SECONDS * 1000);
    return () => window.clearInterval(id);
  }, [active, refresh]);

  const nameOf = (id: string) => conns.find((c) => c.id === id)?.name ?? id;
  const rows: Row[] = useMemo(() => {
    const all = ids.flatMap((id) => (data[id]?.processes ?? []).map((p) => ({ ...p, server: nameOf(id), connId: id })));
    if (!sort) return all;
    return [...all].sort((a, b) => {
      const x = a[sort.key], y = b[sort.key];
      if (x === y) return 0;
      if (x === null || x === undefined) return 1;
      if (y === null || y === undefined) return -1;
      return (typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y), undefined, { numeric: true })) * sort.dir;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data, ids, sort]);

  useEffect(() => {
    if (!active) return;
    onStatus([
      { text: t("mon.selected", { n: ids.length }), width: 150 },
      { text: tab === "process" ? t("mon.processes", { n: rows.length }) : "", width: 160 },
      { text: tab === "process" ? t("mon.autoRefresh", { n: REFRESH_SECONDS }) : "", grow: true },
    ]);
  }, [active, ids.length, rows.length, tab, t, onStatus]);

  const rowKey = (r: Row, i: number) => `${r.connId}:${r.id}:${i}`;

  const endProcess = async () => {
    if (!killing) return;
    try {
      await pg.monitorEndProcess(killing.connId, killing.id);
      setKilling(null);
      setKillError(null);
      void refresh();
    } catch (e) {
      setKillError(errorText(toAtlasError(e)));
    }
  };

  const toggle = (id: string) => setChecked((s) => {
    const next = new Set(s);
    if (next.has(id)) next.delete(id);
    else {
      next.add(id);
      asked.current.delete(id);
    }
    return next;
  });

  const keyValueTable = (kind: "variables" | "status") => {
    const names = Array.from(new Set(ids.flatMap((id) => (data[id]?.[kind] ?? []).map(([n]) => n))));
    // A ordem é a do primeiro servidor; nomes que só outro servidor tem vão para o fim.
    const first = data[ids[0]]?.[kind]?.map(([n]) => n) ?? [];
    names.sort((a, b) => (first.indexOf(a) + 1 || 1e9) - (first.indexOf(b) + 1 || 1e9));
    const needle = search.trim().toLowerCase();
    const shown = names.filter((n) => !needle || n.toLowerCase().includes(needle) || ids.some((id) => (data[id]?.[kind].find(([k]) => k === n)?.[1] ?? "").toLowerCase().includes(needle)));
    return (
      <div className="mon-grid-wrap">
        <table className="mon-grid">
          <thead><tr><th className="is-key">{t(kind === "variables" ? "mon.variable" : "mon.status")}</th>{ids.map((id) => <th key={id}>{nameOf(id)}</th>)}</tr></thead>
          <tbody>
            {shown.map((n) => (
              <tr key={n} className={selected === `${kind}:${n}` ? "is-sel" : undefined} onClick={() => setSelected(`${kind}:${n}`)}>
                <td className="is-key">{n}</td>
                {ids.map((id) => <td key={id}>{data[id]?.[kind].find(([k]) => k === n)?.[1] ?? ""}</td>)}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  };

  const selectedRow = tab === "process" ? rows.find((r, i) => rowKey(r, i) === selected) : undefined;

  return (
    <div className="mon-doc">
      <div className="mon-toolbar">
        <button type="button" className="mon-btn" onClick={() => void refresh()}><SmallIcon name="refresh" /><span>{t("mon.refresh")}</span></button>
        {tab === "process" && (
          <>
            <button type="button" className="mon-btn" disabled={!selectedRow} onClick={() => selectedRow && setKilling(selectedRow)}><span className="mon-x">✕</span><span>{t("mon.endProcess")}</span></button>
            <button type="button" className="mon-btn" onClick={() => setSort({ key: column, dir: 1 })}><span className="mon-az">A↓</span><span>{t("mon.sortAsc")}</span></button>
            <button type="button" className="mon-btn" onClick={() => setSort({ key: column, dir: -1 })}><span className="mon-az">Z↓</span><span>{t("mon.sortDesc")}</span></button>
          </>
        )}
        <span className="spacer" />
        {tab !== "process" && (
          <label className="mon-search"><SmallIcon name="search" size={12} /><input value={search} placeholder={t("mon.search")} spellCheck={false} onChange={(e) => setSearch(e.target.value)} /></label>
        )}
      </div>
      <div className="mon-body">
        <div className="mon-conns">
          {conns.map((c) => (
            <label key={c.id} className={`mon-conn${c.open ? "" : " is-closed"}`}>
              <input type="checkbox" checked={checked.has(c.id)} onChange={() => toggle(c.id)} />
              <ProviderIcon provider={c.provider as never} open={c.open} />
              <span>{c.name}</span>
            </label>
          ))}
        </div>
        <div className="mon-main">
          <div className="mon-tabs" role="tablist">
            {(["process", "variables", "status"] as SubTab[]).map((x) => (
              <button key={x} type="button" role="tab" aria-selected={tab === x} className={`mon-tab${tab === x ? " is-active" : ""}`} onClick={() => { setTab(x); setSelected(null); setSearch(""); }}>{t(`mon.tab.${x}` as never)}</button>
            ))}
          </div>
          {errors && <div className="msg-error mon-error">{errors}</div>}
          {tab === "process" && (
            <div className="mon-grid-wrap">
              <table className="mon-grid">
                <colgroup>{PROCESS_COLUMNS.map((c) => <col key={c.key} style={{ width: c.width }} />)}</colgroup>
                <thead>
                  <tr>
                    {PROCESS_COLUMNS.map((c) => (
                      <th key={c.key} className={column === c.key ? "is-key" : undefined} onClick={() => setColumn(c.key)}>
                        {c.label}{sort?.key === c.key ? (sort.dir === 1 ? " ▲" : " ▼") : ""}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {rows.map((r, i) => (
                    <tr key={rowKey(r, i)} className={selected === rowKey(r, i) ? "is-sel" : undefined} onClick={() => setSelected(rowKey(r, i))}>
                      {PROCESS_COLUMNS.map((c) => {
                        const v = r[c.key];
                        const empty = (c.key === "state" || c.key === "info") && (v === null || v === undefined);
                        return <td key={c.key} className={`${column === c.key ? "is-key" : ""}${empty ? " is-null" : ""}`} title={text(v)}>{empty ? "(Null)" : text(v)}</td>;
                      })}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {tab === "variables" && keyValueTable("variables")}
          {tab === "status" && keyValueTable("status")}
        </div>
      </div>
      {killing && (
        <MessageDialog
          title={t("mon.endProcess")}
          message={t("mon.endConfirm", { id: killing.id, server: killing.server })}
          danger
          error={killError ?? undefined}
          okLabel={t("mon.endProcess")}
          onOk={() => void endProcess()}
          onCancel={() => { setKilling(null); setKillError(null); }}
        />
      )}
    </div>
  );
}
