import { useEffect, useMemo, useRef, useState } from "react";
import {
  chooseExportPath, defaultExportDir, errorText, onExportProgress, pathSeparator, pg, toAtlasError,
  type DumpResult, type DumpScope,
} from "../backend";
import { useT } from "../i18n";
import { formatBytes } from "../model";
import { SmallIcon } from "../icons";

export interface DumpTarget {
  connId: string;
  database: string;
  scope: DumpScope;
  /** Nome para o arquivo e para o título (banco, schema ou tabela). */
  label: string;
  /** Aberto por "Somente estrutura": começa sem os dados. */
  structureOnly: boolean;
}

interface Props {
  target: DumpTarget;
  onClose: () => void;
}

const fileName = (s: string) => s.replace(/[\\/:*?"<>|]/g, "_");

export function DumpDialog({ target, onClose }: Props) {
  const t = useT();
  const sep = useMemo(pathSeparator, []);
  const [structure, setStructure] = useState(true);
  const [data, setData] = useState(!target.structureOnly);
  const [format, setFormat] = useState<"copy" | "insert">("copy");
  const [rowsPerInsert, setRowsPerInsert] = useState(100);
  const [clean, setClean] = useState(false);
  const [owner, setOwner] = useState(false);
  const [path, setPath] = useState("");
  const [running, setRunning] = useState(false);
  const [rows, setRows] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [result, setResult] = useState<DumpResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const dumpId = useRef("");

  useEffect(() => {
    let alive = true;
    void defaultExportDir().then((dir) => {
      if (!alive) return;
      const name = `${fileName(target.label)}.sql`;
      setPath(dir ? (dir.endsWith("/") || dir.endsWith("\\") ? dir : dir + sep) + name : name);
    });
    return () => { alive = false; };
  }, [target.label, sep]);

  useEffect(() => {
    if (!running) return;
    const started = Date.now();
    const id = window.setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 500);
    return () => window.clearInterval(id);
  }, [running]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !running) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, running]);

  const browse = async () => {
    const chosen = await chooseExportPath(`${fileName(target.label)}.sql`, ["sql"]);
    if (chosen) setPath(chosen);
  };

  const start = async () => {
    setError(null);
    setResult(null);
    setRows(0);
    setElapsed(0);
    setRunning(true);
    const id = crypto.randomUUID();
    dumpId.current = id;
    const unlisten = await onExportProgress(id, (n) => setRows(n));
    try {
      const r = await pg.dumpSql({
        dumpId: id, sessionId: target.connId, database: target.database,
        options: { scope: target.scope, structure, data, clean, includeOwner: owner },
        dataFormat: format, rowsPerInsert: Math.max(1, Math.min(1000, Math.floor(rowsPerInsert) || 1)), path,
      });
      setResult(r);
    } catch (e) {
      const err = toAtlasError(e);
      setError(err.kind === "canceled" ? t("dump.canceled") : errorText(err));
    } finally {
      unlisten();
      setRunning(false);
    }
  };

  const cancel = () => { void pg.exportCancel(dumpId.current).catch(() => undefined); };
  const scopeText = target.scope.kind === "database" ? t("dump.scopeDatabase", { name: target.label })
    : target.scope.kind === "schema" ? t("dump.scopeSchema", { name: target.label })
    : t("dump.scopeTable", { name: target.label });
  const canStart = !running && (structure || data) && path.trim() !== "";

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal dump-dialog" role="dialog" aria-label={t("dump.title")}>
        <div className="modal-header"><span className="modal-title">{t("dump.title")} — {scopeText}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose} disabled={running}><SmallIcon name="close" /></button></div>
        <div className="modal-body dump-body">
          <div className="dump-row">
            <label className="check-row"><input type="checkbox" checked={structure} disabled={running} onChange={(e) => setStructure(e.target.checked)} /><span>{t("dump.structure")}</span></label>
            <label className="check-row"><input type="checkbox" checked={data} disabled={running} onChange={(e) => setData(e.target.checked)} /><span>{t("dump.data")}</span></label>
          </div>
          {data && (
            <div className="dump-row">
              <label>{t("dump.format")}</label>
              <select className="field" value={format} disabled={running} onChange={(e) => setFormat(e.target.value as "copy" | "insert")}>
                <option value="copy">{t("dump.formatCopy")}</option>
                <option value="insert">{t("dump.formatInsert")}</option>
              </select>
              {format === "insert" && (
                <>
                  <label>{t("exp.rowsPerInsert")}</label>
                  <input className="field exp-num" inputMode="numeric" value={rowsPerInsert} disabled={running} onChange={(e) => setRowsPerInsert(Number(e.target.value.replace(/\D/g, "")) || 0)} />
                </>
              )}
            </div>
          )}
          <div className="dump-row">
            <label className="check-row" title={t("dump.cleanHint")}><input type="checkbox" checked={clean} disabled={running || !structure} onChange={(e) => setClean(e.target.checked)} /><span>{t("dump.clean")}</span></label>
            <label className="check-row"><input type="checkbox" checked={owner} disabled={running || !structure} onChange={(e) => setOwner(e.target.checked)} /><span>{t("dump.owner")}</span></label>
          </div>
          <div className="dump-row dump-path">
            <label>{t("dump.file")}</label>
            <input className="field exp-path-input" value={path} disabled={running} onChange={(e) => setPath(e.target.value)} spellCheck={false} />
            <button type="button" className="btn" disabled={running} onClick={() => void browse()}>{t("exp.browse")}</button>
          </div>
          <div className="muted dump-note">{t("dump.note")}</div>
          {running && (
            <div className="dump-progress"><span className="spinner" /> {t("dump.running", { n: rows.toLocaleString(), s: elapsed })}</div>
          )}
          {result && (
            <div className="dump-result">
              {t("dump.done", { objects: result.objects, tables: result.tables, rows: result.rows.toLocaleString(), size: formatBytes(result.bytes), s: (result.elapsedMs / 1000).toFixed(1) })}
              <div className="muted">{result.path}</div>
            </div>
          )}
          {error && <div className="doc-banner is-warn">{error}</div>}
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          {running
            ? <button type="button" className="btn" onClick={cancel}>{t("exp.cancel")}</button>
            : <button type="button" className="btn btn-primary" disabled={!canStart} onClick={() => void start()}>{t("dump.start")}</button>}
          <button type="button" className="btn" disabled={running} onClick={onClose}>{t("tab.close")}</button>
        </div>
      </div>
    </div>
  );
}
