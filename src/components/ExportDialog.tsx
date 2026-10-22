import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  chooseExportPath, errorText, onExportProgress, pg, toAtlasError,
  type ExportEncoding, type ExportFormat, type ExportResult, type OrderBy, type TableFilter,
} from "../backend";
import { useT } from "../i18n";

/** O que exportar: a tabela/view e, quando vem do grid, o filtro, a ordenação e as colunas em uso. */
export interface ExportTarget {
  connId: string;
  database: string;
  schema: string;
  table: string;
  allColumns: string[];
  visibleColumns: string[];
  orderBy: OrderBy[];
  filter: TableFilter | null;
}

interface Props {
  target: ExportTarget;
  onClose: () => void;
}

type Kind = "csv" | "json" | "sql" | "xlsx";
const KINDS: Kind[] = ["csv", "json", "sql", "xlsx"];
/** Extensões aceitas por formato; a primeira é a sugerida. */
const EXTENSIONS: Record<Kind, string[]> = { csv: ["csv", "txt", "tsv"], json: ["json"], sql: ["sql"], xlsx: ["xlsx"] };
const ALL_EXTENSIONS = new Set(Object.values(EXTENSIONS).flat());

interface Options {
  kind: Kind;
  csv: { delimiter: string; qualifier: string; header: boolean; crlf: boolean; nullText: string; encoding: ExportEncoding };
  json: { pretty: boolean };
  sql: { rowsPerInsert: number };
  xlsx: { header: boolean; sheetName: string };
}

const STORAGE_KEY = "atlas.export.options";

function defaults(): Options {
  // Excel em português espera ";" e UTF-8 com BOM para abrir o CSV sem passar pela importação.
  const pt = typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("pt");
  return {
    kind: "csv",
    csv: { delimiter: pt ? ";" : ",", qualifier: '"', header: true, crlf: true, nullText: "", encoding: pt ? "utf8Bom" : "utf8" },
    json: { pretty: true },
    sql: { rowsPerInsert: 1 },
    xlsx: { header: true, sheetName: "" },
  };
}

function loadOptions(): Options {
  const base = defaults();
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as Partial<Options> | null;
    if (!saved) return base;
    return {
      kind: KINDS.includes(saved.kind as Kind) ? (saved.kind as Kind) : base.kind,
      csv: { ...base.csv, ...saved.csv },
      json: { ...base.json, ...saved.json },
      sql: { ...base.sql, ...saved.sql },
      xlsx: { ...base.xlsx, ...saved.xlsx },
    };
  } catch {
    return base;
  }
}

function saveOptions(o: Options) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(o));
  } catch {
    /* armazenamento indisponível: só não lembra */
  }
}

/** Troca a extensão do caminho se ela for de outro formato do assistente; senão mantém. */
function swapExtension(path: string, kind: Kind): string {
  const m = /\.([A-Za-z0-9]+)$/.exec(path);
  if (!m) return path;
  const ext = m[1].toLowerCase();
  if (!ALL_EXTENSIONS.has(ext) || EXTENSIONS[kind].includes(ext)) return path;
  return `${path.slice(0, -m[0].length)}.${EXTENSIONS[kind][0]}`;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function buildFormat(o: Options, table: string): ExportFormat {
  switch (o.kind) {
    case "csv": return { kind: "delimited", ...o.csv };
    case "json": return { kind: "json", ...o.json };
    case "sql": return { kind: "sql", rowsPerInsert: Math.max(1, Math.min(1000, Math.floor(o.sql.rowsPerInsert) || 1)) };
    case "xlsx": return { kind: "xlsx", header: o.xlsx.header, sheetName: o.xlsx.sheetName.trim() || table };
  }
}

type Phase =
  | { kind: "idle" }
  | { kind: "running"; rows: number; canceling: boolean }
  | { kind: "done"; result: ExportResult }
  | { kind: "canceled" }
  | { kind: "error"; message: string };

/** Assistente de exportação: escolhe formato e opções, o arquivo de destino e grava em streaming. */
export function ExportDialog({ target, onClose }: Props) {
  const t = useT();
  const [opts, setOpts] = useState<Options>(loadOptions);
  const [path, setPath] = useState<string | null>(null);
  const [onlyVisible, setOnlyVisible] = useState(true);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const exportId = useRef("");
  const running = phase.kind === "running";
  const hasHidden = target.visibleColumns.length < target.allColumns.length;
  const filtered = !!target.filter?.expr.trim();
  const sorted = target.orderBy.length > 0;

  const patch = <K extends keyof Options>(k: K, v: Partial<Options[K]>) => setOpts((o) => ({ ...o, [k]: { ...(o[k] as object), ...v } }));

  const setKind = (kind: Kind) => {
    setOpts((o) => ({ ...o, kind }));
    setPath((p) => (p ? swapExtension(p, kind) : p));
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !running) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [running, onClose]);

  const browse = async () => {
    try {
      const chosen = await chooseExportPath(`${target.table}.${EXTENSIONS[opts.kind][0]}`, EXTENSIONS[opts.kind]);
      if (chosen) setPath(chosen);
    } catch (e) {
      setPhase({ kind: "error", message: errorText(toAtlasError(e)) });
    }
  };

  const start = async () => {
    if (!path?.trim() || running) return;
    saveOptions(opts);
    exportId.current = crypto.randomUUID();
    setPhase({ kind: "running", rows: 0, canceling: false });
    const unlisten = await onExportProgress(exportId.current, (rows) => setPhase((p) => (p.kind === "running" ? { ...p, rows } : p)));
    try {
      const result = await pg.export({
        exportId: exportId.current,
        sessionId: target.connId,
        database: target.database,
        schema: target.schema,
        table: target.table,
        columns: hasHidden && onlyVisible ? target.visibleColumns : [],
        orderBy: target.orderBy,
        filter: target.filter,
        format: buildFormat(opts, target.table),
        path: path.trim(),
      });
      setPhase({ kind: "done", result });
    } catch (e) {
      const err = toAtlasError(e);
      setPhase(err.kind === "canceled" ? { kind: "canceled" } : { kind: "error", message: errorText(err) });
    } finally {
      unlisten();
    }
  };

  const cancel = () => {
    if (phase.kind !== "running" || phase.canceling) return;
    setPhase({ ...phase, canceling: true });
    void pg.exportCancel(exportId.current).catch(() => undefined);
  };

  const scope = [t(filtered ? "exp.scopeFiltered" : "exp.scopeAll"), sorted ? t("exp.scopeSorted") : null].filter(Boolean).join(" · ");

  const select = <T extends string>(value: T, items: [T, string][], onChange: (v: T) => void) => (
    <select className="field" value={value} onChange={(e) => onChange(e.target.value as T)}>
      {items.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
    </select>
  );
  const row = (label: string, control: ReactNode) => (
    <div className="exp-row">
      <label>{label}</label>
      {control}
    </div>
  );

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget && !running) onClose(); }}>
      <div className="modal exp-dialog" role="dialog" aria-label={t("exp.title")}>
        <div className="modal-header">{t("exp.title")}</div>
        <div className="modal-body">
          <fieldset className="exp-fields" disabled={running}>
            <div className="exp-source">
              <strong>{target.schema}.{target.table}</strong>
              <span className="muted">{scope}</span>
            </div>

            <div className="exp-kinds" role="radiogroup" aria-label={t("exp.format")}>
              {KINDS.map((k) => (
                <button
                  key={k}
                  type="button"
                  role="radio"
                  aria-checked={opts.kind === k}
                  className={`exp-kind${opts.kind === k ? " is-active" : ""}`}
                  onClick={() => setKind(k)}
                >
                  <span className="exp-kind-name">{t(`exp.kind.${k}`)}</span>
                  <span className="muted">{t(`exp.kind.${k}.hint`)}</span>
                </button>
              ))}
            </div>

            <div className="exp-options">
              {opts.kind === "csv" && (
                <>
                  {row(t("exp.delimiter"), select(opts.csv.delimiter, [[",", t("exp.delim.comma")], [";", t("exp.delim.semicolon")], ["\t", t("exp.delim.tab")], ["|", t("exp.delim.pipe")], [" ", t("exp.delim.space")]], (v) => patch("csv", { delimiter: v })))}
                  {row(t("exp.qualifier"), select(opts.csv.qualifier, [['"', t("exp.qual.double")], ["'", t("exp.qual.single")], ["", t("exp.qual.none")]], (v) => patch("csv", { qualifier: v })))}
                  {row(t("exp.encoding"), select<ExportEncoding>(opts.csv.encoding, [["utf8", "UTF-8"], ["utf8Bom", t("exp.enc.utf8bom")], ["windows1252", "Windows-1252"]], (v) => patch("csv", { encoding: v })))}
                  {row(t("exp.lineEnd"), select(opts.csv.crlf ? "crlf" : "lf", [["crlf", "CRLF (Windows)"], ["lf", "LF (Linux/macOS)"]], (v) => patch("csv", { crlf: v === "crlf" })))}
                  {row(t("exp.nullText"), <input className="field" value={opts.csv.nullText} placeholder={t("exp.nullText.empty")} onChange={(e) => patch("csv", { nullText: e.target.value })} spellCheck={false} />)}
                  <label className="check-row"><input type="checkbox" checked={opts.csv.header} onChange={(e) => patch("csv", { header: e.target.checked })} /><span>{t("exp.header")}</span></label>
                  {opts.csv.encoding === "windows1252" && <div className="muted exp-note">{t("exp.enc.win1252Note")}</div>}
                </>
              )}
              {opts.kind === "json" && (
                <label className="check-row"><input type="checkbox" checked={opts.json.pretty} onChange={(e) => patch("json", { pretty: e.target.checked })} /><span>{t("exp.jsonPretty")}</span></label>
              )}
              {opts.kind === "sql" && (
                <>
                  {row(t("exp.rowsPerInsert"), (
                    <input
                      className="field exp-num"
                      inputMode="numeric"
                      value={opts.sql.rowsPerInsert}
                      onChange={(e) => patch("sql", { rowsPerInsert: Number(e.target.value.replace(/\D/g, "")) || 0 })}
                      onBlur={() => patch("sql", { rowsPerInsert: Math.max(1, Math.min(1000, opts.sql.rowsPerInsert || 1)) })}
                    />
                  ))}
                  <div className="muted exp-note">{t("exp.sqlNote", { table: `${target.schema}.${target.table}` })}</div>
                </>
              )}
              {opts.kind === "xlsx" && (
                <>
                  {row(t("exp.sheetName"), <input className="field" value={opts.xlsx.sheetName} placeholder={target.table} onChange={(e) => patch("xlsx", { sheetName: e.target.value })} spellCheck={false} />)}
                  <label className="check-row"><input type="checkbox" checked={opts.xlsx.header} onChange={(e) => patch("xlsx", { header: e.target.checked })} /><span>{t("exp.header")}</span></label>
                  <div className="muted exp-note">{t("exp.xlsxNote")}</div>
                </>
              )}
              {hasHidden && (
                <label className="check-row">
                  <input type="checkbox" checked={onlyVisible} onChange={(e) => setOnlyVisible(e.target.checked)} />
                  <span>{t("exp.onlyVisible", { n: target.visibleColumns.length, total: target.allColumns.length })}</span>
                </label>
              )}
            </div>

            <div className="exp-file">
              <label>{t("exp.file")}</label>
              <div className="exp-file-row">
                <input className="field" value={path ?? ""} placeholder={t("exp.file.placeholder")} title={path ?? undefined} spellCheck={false} onChange={(e) => setPath(e.target.value.trim() ? e.target.value : null)} />
                <button type="button" className="btn" onClick={() => void browse()}>{t("exp.browse")}</button>
              </div>
            </div>
          </fieldset>

          {phase.kind === "running" && (
            <div className="exp-status" role="status">
              <span className="spinner" />
              <span>{phase.canceling ? t("exp.canceling") : t("exp.running", { n: phase.rows.toLocaleString() })}</span>
            </div>
          )}
          {phase.kind === "done" && (
            <div className="exp-status is-ok" role="status">
              <div>{t("exp.done", { n: phase.result.rows.toLocaleString(), size: formatBytes(phase.result.bytes), s: (phase.result.elapsedMs / 1000).toFixed(1) })}</div>
              <div className="muted exp-path">{phase.result.path}</div>
            </div>
          )}
          {phase.kind === "canceled" && <div className="exp-status" role="status">{t("exp.canceled")}</div>}
          {phase.kind === "error" && <div className="msg-error" role="alert">{phase.message}</div>}
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          {running ? (
            <button type="button" className="btn" disabled={phase.kind === "running" && phase.canceling} onClick={cancel}>{t("common.cancel")}</button>
          ) : (
            <>
              <button type="button" className="btn btn-primary" disabled={!path || phase.kind === "done"} onClick={() => void start()}>{t("exp.start")}</button>
              <button type="button" className="btn" onClick={onClose}>{t(phase.kind === "done" ? "common.close" : "common.cancel")}</button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
