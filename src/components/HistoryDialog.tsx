import { useCallback, useEffect, useRef, useState } from "react";
import { errorText, local, toAtlasError, type HistoryEntry } from "../backend";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import { MessageDialog } from "./MessageDialog";

interface Props {
  /** Conexões para o filtro (as salvas; a atual vem marcada). */
  connections: { id: string; name: string }[];
  currentConnectionId?: string;
  lang?: string;
  /** Devolve o SQL ao editor: `replace` troca o conteúdo, `append` acrescenta no fim. */
  onUse: (sql: string, mode: "replace" | "append") => void;
  onClose: () => void;
}

const PAGE = 200;

/** Primeiras linhas do SQL para a lista (o texto completo aparece ao selecionar). */
const preview = (sql: string) => {
  const lines = sql.trim().split("\n").slice(0, 3).join("\n");
  return lines.length > 400 ? `${lines.slice(0, 400)}…` : lines;
};

export function HistoryDialog({ connections, currentConnectionId, lang, onUse, onClose }: Props) {
  const t = useT();
  const [search, setSearch] = useState("");
  const [connectionId, setConnectionId] = useState<string>("");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [recording, setRecording] = useState(true);
  const [confirmClear, setConfirmClear] = useState(false);
  const seq = useRef(0);

  useEffect(() => {
    void local.getPref<boolean>("history.enabled").then((v) => setRecording(v !== false)).catch(() => undefined);
  }, []);

  const load = useCallback(() => {
    const mine = ++seq.current;
    setLoading(true);
    local
      .historyList({ search, connectionId: connectionId || null, favoritesOnly, errorsOnly, limit: PAGE, offset: 0 })
      .then((list) => {
        if (mine !== seq.current) return;
        setEntries(list);
        setSelected((s) => (s && list.some((e) => e.id === s) ? s : list[0]?.id ?? null));
        setError(null);
      })
      .catch((e) => mine === seq.current && setError(errorText(toAtlasError(e))))
      .finally(() => mine === seq.current && setLoading(false));
  }, [search, connectionId, favoritesOnly, errorsOnly]);

  // A busca espera o usuário parar de digitar; os filtros recarregam na hora.
  useEffect(() => {
    const id = window.setTimeout(load, search ? 200 : 0);
    return () => window.clearTimeout(id);
  }, [load, search]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !confirmClear) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, confirmClear]);

  const current = entries.find((e) => e.id === selected) ?? null;
  const fail = (e: unknown) => setError(errorText(toAtlasError(e)));

  const toggleFavorite = (entry: HistoryEntry) => {
    void local.historyFavorite(entry.id, !entry.favorite).then(load, fail);
  };
  const remove = (entry: HistoryEntry) => {
    void local.historyDelete(entry.id).then(load, fail);
  };
  const copy = (entry: HistoryEntry) => { void navigator.clipboard?.writeText(entry.sql).catch(() => undefined); };
  const toggleRecording = (on: boolean) => {
    setRecording(on);
    void local.setPref("history.enabled", on).catch(fail);
  };

  const when = (iso: string) => new Date(iso).toLocaleString(lang);
  const outcome = (e: HistoryEntry) => {
    if (e.status === "error") return <span className="hist-tag is-error" title={e.error ?? ""}>{t("hist.error")}</span>;
    return <span className="hist-tag">{e.rows === null ? t("hist.ok") : t("hist.rows", { n: e.rows })}</span>;
  };

  return (
    <div className="modal-overlay" role="presentation" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal hist-dialog" role="dialog" aria-label={t("hist.title")}>
        <div className="modal-header">{t("hist.title")}</div>
        <div className="hist-filters">
          <input className="field hist-search" value={search} placeholder={t("hist.search")} onChange={(e) => setSearch(e.target.value)} spellCheck={false} autoFocus />
          <select className="field" value={connectionId} onChange={(e) => setConnectionId(e.target.value)} aria-label={t("hist.connection")}>
            <option value="">{t("hist.allConnections")}</option>
            {connections.map((c) => <option key={c.id} value={c.id}>{c.name}{c.id === currentConnectionId ? ` (${t("hist.current")})` : ""}</option>)}
          </select>
          <label className="check-row"><input type="checkbox" checked={favoritesOnly} onChange={(e) => setFavoritesOnly(e.target.checked)} /><span>{t("hist.favoritesOnly")}</span></label>
          <label className="check-row"><input type="checkbox" checked={errorsOnly} onChange={(e) => setErrorsOnly(e.target.checked)} /><span>{t("hist.errorsOnly")}</span></label>
        </div>
        <div className="hist-body">
          <div className="hist-list" role="listbox" aria-label={t("hist.title")}>
            {entries.map((e) => (
              <div
                key={e.id}
                role="option"
                aria-selected={e.id === selected}
                className={`hist-row${e.id === selected ? " is-selected" : ""}`}
                onClick={() => setSelected(e.id)}
                onDoubleClick={() => onUse(e.sql, "replace")}
              >
                <button type="button" className={`hist-star${e.favorite ? " is-on" : ""}`} title={t(e.favorite ? "hist.unfavorite" : "hist.favorite")} onClick={(ev) => { ev.stopPropagation(); toggleFavorite(e); }}>
                  {e.favorite ? "★" : "☆"}
                </button>
                <div className="hist-main">
                  <div className="hist-meta">
                    <span>{when(e.executedUtc)}</span>
                    <span className="hist-where">{[e.connectionName, e.database, e.schema].filter(Boolean).join(" · ")}</span>
                    {e.durationMs !== null && <span>{t("query.time", { s: (e.durationMs / 1000).toFixed(3) })}</span>}
                    {outcome(e)}
                  </div>
                  <pre className="hist-sql">{preview(e.sql)}</pre>
                </div>
              </div>
            ))}
            {!loading && entries.length === 0 && <div className="hist-empty">{t(search || favoritesOnly || errorsOnly || connectionId ? "hist.noMatch" : "hist.empty")}</div>}
            {loading && entries.length === 0 && <div className="hist-empty"><span className="spinner" /></div>}
          </div>
          <pre className="hist-detail" aria-label={t("hist.sql")}>{current ? `${current.sql}${current.error ? `\n\n-- ${current.error}` : ""}` : ""}</pre>
        </div>
        {error && <div className="doc-banner is-warn">{error}</div>}
        <div className="modal-footer hist-footer">
          <label className="check-row" title={t("hist.recordingHint")}>
            <input type="checkbox" checked={recording} onChange={(e) => toggleRecording(e.target.checked)} />
            <span>{t("hist.recording")}</span>
          </label>
          <span className="spacer" />
          <button type="button" className="btn" disabled={!current} onClick={() => current && onUse(current.sql, "replace")}>{t("hist.use")}</button>
          <button type="button" className="btn" disabled={!current} onClick={() => current && onUse(current.sql, "append")}>{t("hist.append")}</button>
          <button type="button" className="btn" disabled={!current} onClick={() => current && copy(current)}>{t("hist.copy")}</button>
          <button type="button" className="btn" disabled={!current} onClick={() => current && remove(current)}><SmallIcon name="removeRow" /></button>
          <button type="button" className="btn" disabled={entries.length === 0} onClick={() => setConfirmClear(true)}>{t("hist.clear")}</button>
          <button type="button" className="btn btn-primary" onClick={onClose}>{t("tab.close")}</button>
        </div>
      </div>
      {confirmClear && (
        <MessageDialog
          title={t("hist.clear")}
          message={t("hist.clearMessage")}
          danger
          okLabel={t("hist.clear")}
          onOk={() => { setConfirmClear(false); void local.historyClear(true).then(load, fail); }}
          onCancel={() => setConfirmClear(false)}
        />
      )}
    </div>
  );
}
