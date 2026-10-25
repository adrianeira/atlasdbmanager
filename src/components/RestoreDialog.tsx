import { useEffect, useMemo, useRef, useState } from "react";
import {
  chooseExportFolder, chooseExportPath, chooseOpenFile, defaultExportDir, errorText, onBackupLog, pathSeparator, pg, toAtlasError,
  type BackupContent, type BackupResult, type ToolsInfo,
} from "../backend";
import { useT } from "../i18n";
import { formatBytes } from "../model";
import { SmallIcon } from "../icons";
import { isMssqlId, isSqliteId } from "../providerRoute";
import { Dropdown } from "./Dropdown";

export interface RestoreTarget {
  /** `restore` grava o backup num banco; `extract` só escreve o SQL do backup num arquivo. */
  mode: "restore" | "extract";
  connId: string;
  database: string;
  /** Bancos da conexão (destino da restauração). */
  databases: string[];
  /** Arquivo de backup já escolhido (um da lista da categoria Backup). */
  path?: string;
}

interface Props {
  target: RestoreTarget;
  onClose: () => void;
}

export const TOOLS_KEY = "atlas.pgToolsDir";
export const readToolsDir = (): string => {
  try { return localStorage.getItem(TOOLS_KEY) ?? ""; } catch { return ""; }
};
export const saveToolsDir = (dir: string) => {
  try { localStorage.setItem(TOOLS_KEY, dir); } catch { /* sem armazenamento: vale só nesta janela */ }
};
const fileName = (s: string) => s.replace(/[\\/:*?"<>|]/g, "_");

export function RestoreDialog({ target, onClose }: Props) {
  const t = useT();
  const sep = useMemo(pathSeparator, []);
  const { mode } = target;
  // O SQLite restaura o arquivo inteiro, sem as ferramentas nem as opções do PostgreSQL.
  const sqlite = isSqliteId(target.connId) || isMssqlId(target.connId);
  const [tools, setTools] = useState<ToolsInfo | null>(null);
  const [toolsDir, setToolsDir] = useState(readToolsDir);
  const [content, setContent] = useState<BackupContent>("all");
  const [clean, setClean] = useState(false);
  const [noOwner, setNoOwner] = useState(false);
  const [noPrivileges, setNoPrivileges] = useState(false);
  const [single, setSingle] = useState(false);
  const [destDb, setDestDb] = useState(target.database);
  const [path, setPath] = useState(target.path ?? "");
  const [outPath, setOutPath] = useState("");
  const [running, setRunning] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [log, setLog] = useState<string[]>([]);
  const [result, setResult] = useState<BackupResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const jobId = useRef("");
  const logEnd = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    if (sqlite) { setTools({ dir: "sqlite", version: null, others: [] }); return; }
    setTools(null);
    void pg.toolsLocate(toolsDir).then((r) => { if (alive) setTools(r); }).catch(() => { if (alive) setTools({ dir: null, version: null, others: [] }); });
    return () => { alive = false; };
  }, [toolsDir]);

  useEffect(() => {
    if (mode !== "extract") return;
    let alive = true;
    void defaultExportDir().then((dir) => {
      if (!alive) return;
      const base = path ? fileName(path.split(/[\\/]/).pop()!.replace(/\.[^.]*$/, "")) : fileName(target.database);
      setOutPath((dir ? (dir.endsWith("/") || dir.endsWith("\\") ? dir : dir + sep) : "") + `${base}.sql`);
    });
    return () => { alive = false; };
    // O nome sugerido só acompanha o arquivo de backup escolhido na abertura.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, sep, target.database]);

  useEffect(() => {
    if (!running) return;
    const started = Date.now();
    const id = window.setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 500);
    return () => window.clearInterval(id);
  }, [running]);

  useEffect(() => { logEnd.current?.scrollIntoView({ block: "end" }); }, [log]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !running) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, running]);

  const chooseTools = async () => {
    const dir = await chooseExportFolder();
    if (!dir) return;
    saveToolsDir(dir);
    setToolsDir(dir);
  };

  const browseBackup = async () => {
    const p = await chooseOpenFile(["dump", "backup", "tar", "sql"]);
    if (p) setPath(p);
  };

  const browseOut = async () => {
    const p = await chooseExportPath("backup.sql", ["sql"]);
    if (p) setOutPath(p);
  };

  const start = async () => {
    setError(null);
    setResult(null);
    setLog([]);
    setElapsed(0);
    setRunning(true);
    const id = crypto.randomUUID();
    jobId.current = id;
    const unlisten = await onBackupLog(id, (line) => setLog((l) => (l.length > 2000 ? [...l.slice(-1000), line] : [...l, line])));
    try {
      setResult(await pg.restore({
        jobId: id, sessionId: target.connId, toolsDir: toolsDir || undefined, clean, noOwner, noPrivileges,
        database: destDb, path, content, singleTransaction: single,
        extractTo: mode === "extract" ? outPath : undefined,
      }));
    } catch (e) {
      const err = toAtlasError(e);
      setError(err.kind === "canceled" ? t("bk.canceled") : errorText(err));
    } finally {
      unlisten();
      setRunning(false);
    }
  };

  const cancel = () => { void pg.backupCancel(jobId.current).catch(() => undefined); };
  const title = mode === "restore" ? t("bk.titleRestore") : t("bk.titleExtract");
  const canStart = !running && !!tools?.dir && path.trim() !== "" && (mode !== "extract" || outPath.trim() !== "") && (mode !== "restore" || destDb !== "");
  const doneText = result
    ? t(mode === "restore" ? "bk.doneRestore" : "bk.doneExtract", { size: formatBytes(result.bytes), s: (result.elapsedMs / 1000).toFixed(1) })
    : "";
  // Restaurar um script SQL ignora o conteúdo: o que entra já está decidido no arquivo.
  const showContent = !sqlite && !/\.sql$/i.test(path);

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal dump-dialog backup-dialog" role="dialog" aria-label={title}>
        <div className="modal-header"><span className="modal-title">{title}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose} disabled={running}><SmallIcon name="close" /></button></div>
        <div className="modal-body dump-body">
          {!sqlite && <div className="dump-row">
            <label>{t("bk.tools")}</label>
            <span className={tools?.dir ? "muted" : "bk-missing"}>
              {tools === null ? "…" : tools.dir ? t("bk.toolsFound", { v: tools.version ?? "", dir: tools.dir }) : t("bk.toolsMissing")}
            </span>
            <button type="button" className="btn" disabled={running} onClick={() => void chooseTools()}>{t("bk.toolsChoose")}</button>
          </div>}

          <div className="dump-row dump-path">
            <label>{t("bk.file")}</label>
            <input className="field exp-path-input" value={path} disabled={running} onChange={(e) => setPath(e.target.value)} spellCheck={false} />
            <button type="button" className="btn" disabled={running} onClick={() => void browseBackup()}>{t("exp.browse")}</button>
          </div>

          {mode === "extract" && (
            <div className="dump-row dump-path">
              <label>{t("bk.outFile")}</label>
              <input className="field exp-path-input" value={outPath} disabled={running} onChange={(e) => setOutPath(e.target.value)} spellCheck={false} />
              <button type="button" className="btn" disabled={running} onClick={() => void browseOut()}>{t("exp.browse")}</button>
            </div>
          )}

          {mode === "restore" && !sqlite && (
            <div className="dump-row">
              <label>{t("bk.target")}</label>
              <Dropdown value={destDb} disabled={running} onChange={setDestDb} items={target.databases.map((d) => ({ value: d, label: d }))} />
            </div>
          )}

          {showContent && (
            <div className="dump-row">
              <label>{t("bk.content")}</label>
              {(["all", "structure", "data"] as const).map((c) => (
                <label key={c} className="check-row">
                  <input type="radio" name="bk-content" checked={content === c} disabled={running} onChange={() => setContent(c)} />
                  <span>{t(c === "all" ? "bk.contentAll" : c === "structure" ? "bk.contentStructure" : "bk.contentData")}</span>
                </label>
              ))}
            </div>
          )}

          {!sqlite && <div className="dump-row">
            <label className="check-row"><input type="checkbox" checked={clean} disabled={running} onChange={(e) => setClean(e.target.checked)} /><span>{t("bk.clean")}</span></label>
            <label className="check-row"><input type="checkbox" checked={noOwner} disabled={running} onChange={(e) => setNoOwner(e.target.checked)} /><span>{t("bk.noOwner")}</span></label>
            <label className="check-row"><input type="checkbox" checked={noPrivileges} disabled={running} onChange={(e) => setNoPrivileges(e.target.checked)} /><span>{t("bk.noPrivileges")}</span></label>
            {mode === "restore" && (
              <label className="check-row"><input type="checkbox" checked={single} disabled={running} onChange={(e) => setSingle(e.target.checked)} /><span>{t("bk.single")}</span></label>
            )}
          </div>}
          {mode === "restore" && !sqlite && <div className="muted dump-note">{t("bk.restoreNote")}</div>}

          {running && <div className="dump-progress"><span className="spinner" /> {t("bk.running", { s: elapsed })}</div>}
          {log.length > 0 && (
            <div className="bk-log" aria-label={t("bk.log")}>
              {log.map((l, i) => <div key={i}>{l}</div>)}
              <div ref={logEnd} />
            </div>
          )}
          {result && (
            <div className="dump-result">
              {doneText}
              <div className="muted">{result.path}</div>
            </div>
          )}
          {error && <div className="doc-banner is-warn bk-error">{error}</div>}
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          {running
            ? <button type="button" className="btn" onClick={cancel}>{t("exp.cancel")}</button>
            : <button type="button" className="btn btn-primary" disabled={!canStart} onClick={() => void start()}>{t("bk.start")}</button>}
          <button type="button" className="btn" disabled={running} onClick={onClose}>{t("tab.close")}</button>
        </div>
      </div>
    </div>
  );
}
