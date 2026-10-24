import { useEffect, useMemo, useRef, useState } from "react";
import { pg, type SqlLogEntry } from "../backend";
import type { StatusCell } from "../components/StatusBar";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";

interface Props {
  active: boolean;
  onStatus: (cells: StatusCell[]) => void;
}

const MAX_KEPT = 4000;
const MAX_LINES = 8000;
const pad = (n: number, w = 2) => String(n).padStart(w, "0");

/** `[2026-10-23 16:56:45.377][PostgreSQL 9.6][32600][PGSQL]` na hora local. */
function header(e: SqlLogEntry): string {
  const d = new Date(e.atMs);
  const stamp = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
  return `[${stamp}][${e.server}][${e.pid}][PGSQL]`;
}

function linesOf(e: SqlLogEntry): string[] {
  return [header(e), ...e.text.split("\n"), ...(e.secs === null ? [] : [`Time: ${e.secs.toFixed(3)}s`]), ""];
}

export function HistoryLogDoc({ active, onStatus }: Props) {
  const t = useT();
  const [entries, setEntries] = useState<SqlLogEntry[]>([]);
  const [wrap, setWrap] = useState(true);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [paused, setPaused] = useState(false);
  const [search, setSearch] = useState("");
  const [refreshed, setRefreshed] = useState("");
  const last = useRef(0);
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  // Lê o que o servidor registrou desde a última leitura.
  useEffect(() => {
    if (!active) return;
    let alive = true;
    const tick = async () => {
      try {
        const page = await pg.sqlLogRead(last.current);
        if (!alive) return;
        last.current = page.last;
        setPaused(page.paused);
        if (page.entries.length) setEntries((old) => [...old, ...page.entries].slice(-MAX_KEPT));
        const d = new Date();
        setRefreshed(`${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`);
      } catch {
        /* a próxima leitura tenta de novo */
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 1000);
    return () => { alive = false; window.clearInterval(id); };
  }, [active]);

  useEffect(() => {
    if (active) onStatus([{ text: "", grow: true }, { text: refreshed ? `${t("hl.lastRefresh")} ${refreshed}` : "", width: 200 }, { text: "", width: 60 }]);
  }, [active, refreshed, t, onStatus]);

  const shown = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return entries.filter((e) => (!errorsOnly || e.failed) && (!needle || e.text.toLowerCase().includes(needle)));
  }, [entries, errorsOnly, search]);

  const lines = useMemo(() => {
    const all = shown.flatMap(linesOf);
    return all.length > MAX_LINES ? all.slice(-MAX_LINES) : all;
  }, [shown]);

  // Fica no fim enquanto o usuário não rolou para cima.
  useEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const onScroll = () => {
    const el = box.current;
    if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };

  const clear = () => {
    void pg.sqlLogClear().catch(() => undefined);
    setEntries([]);
  };
  const togglePause = () => {
    const next = !paused;
    setPaused(next);
    void pg.sqlLogSetPaused(next).catch(() => undefined);
  };

  const gutter = String(lines.length).length;
  return (
    <div className="hl-doc">
      <div className="hl-toolbar">
        <button type="button" className="mon-btn" onClick={clear}><SmallIcon name="delete" /><span>{t("hl.clear")}</span></button>
        <span className="hl-sep" />
        <button type="button" className={`mon-btn${wrap ? " is-on" : ""}`} aria-pressed={wrap} onClick={() => setWrap(!wrap)}><SmallIcon name="beautify" /><span>{t("hl.wrap")}</span></button>
        <button type="button" className={`mon-btn${errorsOnly ? " is-on" : ""}`} aria-pressed={errorsOnly} onClick={() => setErrorsOnly(!errorsOnly)}><SmallIcon name="warning" /><span>{t("hl.errorsOnly")}</span></button>
        <button type="button" className={`mon-btn${paused ? " is-on" : ""}`} aria-pressed={paused} onClick={togglePause}><span className="hl-pause">▮▮</span><span>{t("hl.pause")}</span></button>
        <span className="spacer" />
        <label className="mon-search"><SmallIcon name="search" size={12} /><input value={search} spellCheck={false} onChange={(e) => setSearch(e.target.value)} aria-label={t("mon.search")} /></label>
      </div>
      <div className={`hl-body${wrap ? " is-wrap" : ""}`} ref={box} onScroll={onScroll}>
        {lines.map((l, i) => (
          <div key={i} className="hl-line">
            <span className="hl-no" style={{ minWidth: `${gutter + 1}ch` }}>{i + 1}</span>
            <span className="hl-text">{l || " "}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
