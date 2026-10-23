import { useEffect, useMemo, useRef, useState } from "react";
import type { MenuEntry } from "../../commands";
import {
  chooseImportFiles, errorText, local, onImportLog, onImportProgress, pg, toAtlasError,
  type ImportCounts, type ImportJobRequest, type ImportPreview, type ImportMode,
} from "../../backend";
import { useT, type MessageKey } from "../../i18n";
import { SmallIcon, type SmallIconName } from "../../icons";
import { formatClock } from "../export/exportModel";
import { MessageDialog } from "../MessageDialog";
import { PopupMenu } from "../PopupMenu";
import { ImportAdvancedDialog } from "./ImportAdvancedDialog";
import {
  ALL_FORMATS, FORMATS, MODES, PROFILES_KEY, UNAVAILABLE_FORMATS, buildFormat, convertOptions, dateExamples, defaultSettings, formatOf,
  hasGrid, isText, loadLast, modeNeedsKey, saveLast, sourceOptions, sqlName, stemOf,
  type Advanced, type ImportProfile, type Job, type Kind, type MappingRow, type SourceSettings,
} from "./importModel";

export interface ImportTarget {
  connId: string;
  database: string;
  schema: string;
  /** Tabelas do schema (destinos possíveis). */
  tables: string[];
  /** Tabela em que o assistente foi aberto (destino sugerido). */
  initial?: string;
}

interface Props {
  target: ImportTarget;
  onClose: () => void;
  /** Chamado ao terminar uma importação (para reler o schema). */
  onDone: () => void;
}

const STEPS = 7;
const LAST = STEPS - 1;
const HEADINGS: MessageKey[] = ["imp.step0", "imp.step1", "imp.step2", "imp.step3", "imp.step4", "imp.step5", "imp.step6"];

const ICONS: Record<Kind, SmallIconName> = { txt: "fmtTxt", csv: "fmtCsv", json: "fmtJson", xml: "fmtXml", excel: "fmtXlsx", dbf: "fmtTxt" };

/** Informações lidas do arquivo e do destino de um job (não vão para o perfil). */
interface JobInfo {
  source?: ImportPreview;
  targetColumns?: string[];
  error?: string;
}

const COMMON_TYPES = ["text", "integer", "bigint", "numeric", "double precision", "boolean", "date", "timestamp", "timestamptz", "time", "jsonb"];

const newId = () => crypto.randomUUID();

/**
 * Assistente de importação em etapas: formato → arquivos → opções da origem → tabela de destino →
 * campos → modo → resumo com andamento. Cada tabela do arquivo vira um "job".
 */
export function ImportWizard({ target, onClose, onDone }: Props) {
  const t = useT();
  const last = useMemo(loadLast, []);

  const [step, setStep] = useState(0);
  const [kind, setKind] = useState<Kind>(last.kind);
  const [settings, setSettings] = useState<SourceSettings>(last.settings);
  const [advanced, setAdvanced] = useState<Advanced>(last.advanced);
  const [mode, setMode] = useState<ImportMode>("append");
  const [jobs, setJobs] = useState<Job[]>([]);
  const [info, setInfo] = useState<Record<string, JobInfo>>({});
  const [selectedJob, setSelectedJob] = useState("");
  const [fieldJob, setFieldJob] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [menu, setMenu] = useState<"profiles" | null>(null);
  const [profiles, setProfiles] = useState<ImportProfile[]>([]);
  const [namePrompt, setNamePrompt] = useState(false);
  const [preview, setPreview] = useState<ImportPreview | "loading" | { error: string } | null>(null);

  const [running, setRunning] = useState(false);
  const [log, setLog] = useState<{ text: string; error?: boolean }[]>([]);
  const [counts, setCounts] = useState<Record<number, ImportCounts>>({});
  const [currentJob, setCurrentJob] = useState(0);
  const [doneJobs, setDoneJobs] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const importId = useRef("");
  const startedAt = useRef(0);
  const logRef = useRef<HTMLDivElement>(null);
  const jobsRef = useRef(jobs);
  jobsRef.current = jobs;
  const infoRef = useRef(info);
  infoRef.current = info;

  const format = formatOf(kind);
  const specOf = (job: Job) => ({ path: job.file, format: buildFormat(kind, settings), table: job.sheet, options: sourceOptions(settings) });
  const patchJob = (id: string, patch: Partial<Job>) => setJobs((js) => js.map((j) => (j.id === id ? { ...j, ...patch } : j)));

  /* ---------- Inicialização ---------- */
  useEffect(() => {
    let alive = true;
    void local.getPref<ImportProfile[]>(PROFILES_KEY).then((p) => alive && setProfiles(Array.isArray(p) ? p : [])).catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => setElapsed(performance.now() - startedAt.current), 100);
    return () => window.clearInterval(id);
  }, [running]);

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [log]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || running || advancedOpen || namePrompt) return;
      if (menu) setMenu(null);
      else onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [running, advancedOpen, namePrompt, menu, onClose]);

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as Element | null)?.closest?.(".exp-menu-wrap")) setMenu(null);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  /* ---------- Formato e arquivos ---------- */
  const changeKind = (k: Kind) => {
    if (k === kind) return;
    setKind(k);
    setSettings(defaultSettings(k));
    setJobs([]);
    setInfo({});
  };

  /** Destino padrão de uma origem: a tabela de mesmo nome, se existir; senão uma tabela nova. */
  const defaultTarget = (name: string, first: boolean): Pick<Job, "target" | "newTable"> => {
    const wanted = sqlName(name);
    const match = target.tables.find((x) => x.toLowerCase() === wanted) ?? (first ? target.initial : undefined);
    return match ? { target: match, newTable: "" } : { target: "", newTable: wanted };
  };

  const addFiles = async () => {
    try {
      const paths = await chooseImportFiles(format.ext);
      if (!paths.length) return;
      setBusy(true);
      const added: Job[] = [];
      for (const file of paths) {
        const sheets = await pg.importListTables(file, buildFormat(kind, settings));
        for (const sheet of sheets) {
          if (jobsRef.current.some((j) => j.file === file && j.sheet === sheet) || added.some((j) => j.file === file && j.sheet === sheet)) continue;
          const name = sheets.length > 1 ? sheet : stemOf(file);
          added.push({ id: newId(), file, sheet, ...defaultTarget(name, jobsRef.current.length + added.length === 0), mappings: [], keys: [] });
        }
      }
      setJobs((js) => [...js, ...added]);
      setMessage(null);
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    } finally {
      setBusy(false);
    }
  };

  const removeJob = (id: string) => {
    setJobs((js) => js.filter((j) => j.id !== id));
    setInfo((i) => Object.fromEntries(Object.entries(i).filter(([k]) => k !== id)));
  };

  /** Mudou algo que altera os campos lidos: relê tudo e refaz o mapeamento. */
  const changeSettings = (next: SourceSettings) => {
    setSettings(next);
    setInfo({});
    setJobs((js) => js.map((j) => ({ ...j, mappings: [], keys: [] })));
  };

  /* ---------- Pré-visualização (etapa 2) ---------- */
  const firstJob = jobs[0];
  useEffect(() => {
    if (step !== 2 || !firstJob) return;
    let alive = true;
    setPreview("loading");
    const id = window.setTimeout(() => {
      pg.importPreview({ path: firstJob.file, format: buildFormat(kind, settings), table: firstJob.sheet, options: sourceOptions(settings) }, convertOptions(settings), 8)
        .then((p) => alive && setPreview(p))
        .catch((e) => alive && setPreview({ error: errorText(toAtlasError(e)) }));
    }, 250);
    return () => {
      alive = false;
      window.clearTimeout(id);
    };
  }, [step, firstJob, kind, settings]);

  /* ---------- Campos (etapa 4) ---------- */
  /** Lê os campos da origem e do destino e monta o mapeamento automático (se ainda não existe). */
  const prepareJob = async (job: Job): Promise<Job> => {
    let cached = infoRef.current[job.id];
    if (!cached?.source || (job.target && !cached.targetColumns)) {
      const next: JobInfo = { ...cached };
      try {
        if (!next.source) next.source = await pg.importPreview(specOf(job), convertOptions(settings), 1);
        if (job.target && !next.targetColumns) {
          next.targetColumns = (await pg.tableDetail(target.connId, target.database, target.schema, job.target)).columns.map((c) => c.name);
        }
        next.error = undefined;
      } catch (e) {
        next.error = errorText(toAtlasError(e));
      }
      cached = next;
      setInfo((i) => ({ ...i, [job.id]: next }));
    }
    if (job.mappings.length > 0 || !cached.source) return job;
    const src = cached.source;
    const targets = cached.targetColumns;
    const mappings: MappingRow[] = src.columns.map((source, i) => {
      if (!job.target) return { source, target: sqlName(source), type: src.types[i] ?? "text" };
      const wanted = sqlName(source);
      const hit = targets?.find((c) => c.toLowerCase() === source.toLowerCase() || c.toLowerCase() === wanted) ?? "";
      return { source, target: hit, type: "" };
    });
    const keys = job.target ? mappings.filter((m) => m.target.toLowerCase() === "id").map((m) => m.target) : [];
    const ready = { ...job, mappings, keys };
    patchJob(job.id, { mappings, keys });
    return ready;
  };

  useEffect(() => {
    if (step !== 4) return;
    const job = jobs.find((j) => j.id === fieldJob) ?? jobs[0];
    if (!job) return;
    if (job.id !== fieldJob) setFieldJob(job.id);
    void prepareJob(job);
    // `prepareJob` lê o estado por refs; só reage à troca de etapa/job/destino.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, fieldJob, jobs.length, jobs.find((j) => j.id === fieldJob)?.target]);

  const setTargetOf = (id: string, name: string) => {
    const job = jobs.find((j) => j.id === id);
    if (!job) return;
    const newTable = name === "" && !job.newTable ? sqlName(stemOf(job.file)) : job.newTable;
    patchJob(id, { target: name, newTable, mappings: [], keys: [] });
  };

  const patchMapping = (job: Job, index: number, patch: Partial<MappingRow>) => {
    const mappings = job.mappings.map((m, i) => (i === index ? { ...m, ...patch } : m));
    // Um campo de destino só recebe uma origem: a escolha anterior é liberada.
    const cleaned = patch.target && job.target ? mappings.map((m, i) => (i !== index && m.target === patch.target ? { ...m, target: "" } : m)) : mappings;
    const mapped = new Set(cleaned.map((m) => m.target));
    patchJob(job.id, { mappings: cleaned, keys: job.keys.filter((k) => mapped.has(k)) });
  };

  const toggleKey = (job: Job, name: string, on: boolean) => patchJob(job.id, { keys: on ? [...job.keys.filter((k) => k !== name), name] : job.keys.filter((k) => k !== name) });

  const autoMap = (job: Job) => {
    const cached = info[job.id];
    patchJob(job.id, { mappings: [], keys: [] });
    if (cached) void prepareJob({ ...job, mappings: [], keys: [] });
  };

  const clearMap = (job: Job) => patchJob(job.id, { mappings: job.mappings.map((m) => (job.target ? { ...m, target: "" } : m)), keys: [] });

  /* ---------- Navegação ---------- */
  const firstProblem = (upTo: number, list: Job[]): { step: number; message: string } | null => {
    if (upTo > 1 && list.length === 0) return { step: 1, message: t("imp.needFile") };
    if (upTo > 3 && list.some((j) => !j.target && !j.newTable.trim())) return { step: 3, message: t("imp.needTarget") };
    if (upTo > 5) {
      for (const j of list) {
        const name = j.target || j.newTable;
        const mapped = j.mappings.filter((m) => m.target.trim());
        if (mapped.length === 0) return { step: 4, message: t("imp.needMapping", { table: name }) };
        if (j.target && modeNeedsKey(mode) && !j.keys.some((k) => mapped.some((m) => m.target === k))) return { step: 4, message: t("imp.needKey", { table: name }) };
      }
    }
    return null;
  };

  const goTo = async (to: number) => {
    if (to > step) {
      let list = jobsRef.current;
      if (to > 4) {
        setBusy(true);
        const prepared: Job[] = [];
        for (const j of list) prepared.push(await prepareJob(j));
        list = prepared;
        setBusy(false);
      }
      const problem = firstProblem(to, list);
      if (problem) {
        setStep(problem.step);
        setMessage(problem.message);
        if (problem.step === 4) setFieldJob(list.find((j) => j.mappings.every((m) => !m.target) || (j.target && modeNeedsKey(mode) && j.keys.length === 0))?.id ?? list[0].id);
        return;
      }
    }
    setMessage(null);
    setMenu(null);
    setStep(to);
  };

  /* ---------- Perfis ---------- */
  const persistProfiles = (next: ImportProfile[]) => {
    setProfiles(next);
    void local.setPref(PROFILES_KEY, next).catch(() => undefined);
  };

  const saveProfile = (name: string) => {
    const profile: ImportProfile = { name, kind, settings, mode, advanced, jobs };
    persistProfiles([...profiles.filter((p) => p.name !== name), profile].sort((a, b) => a.name.localeCompare(b.name)));
    setNamePrompt(false);
    setLog((l) => [...l, { text: t("exp.profileSaved", { name }) }]);
  };

  const applyProfile = (p: ImportProfile) => {
    setKind(ALL_FORMATS.some((f) => f.kind === p.kind) ? p.kind : "csv");
    setSettings({ ...defaultSettings(p.kind), ...p.settings });
    setMode(p.mode);
    setAdvanced(p.advanced);
    setInfo({});
    // Tabelas de destino que não existem mais viram tabelas novas.
    setJobs(p.jobs.map((j) => (j.target && !target.tables.includes(j.target) ? { ...j, id: newId(), target: "", newTable: j.newTable || sqlName(j.target) } : { ...j, id: newId() })));
    setLog((l) => [...l, { text: t("exp.profileLoaded", { name: p.name }) }]);
  };

  /* ---------- Execução ---------- */
  const start = async () => {
    if (running || busy) return;
    const list = jobsRef.current;
    const problem = firstProblem(STEPS, list);
    if (problem) {
      setStep(problem.step);
      setMessage(problem.message);
      return;
    }
    saveLast(kind, settings, advanced);
    const add = (text: string, error = false) => setLog((l) => [...l, { text, error }]);
    importId.current = crypto.randomUUID();
    const id = importId.current;
    startedAt.current = performance.now();
    setElapsed(0);
    setCounts({});
    setDoneJobs(0);
    setCurrentJob(0);
    setRunning(true);
    setLog([{ text: t("imp.log.start") }, { text: t("imp.log.format", { format: t(format.label) }) }]);
    const requests: ImportJobRequest[] = list.map((j) => {
      const isNew = !j.target;
      const mapped = j.mappings.filter((m) => m.target.trim());
      return {
        source: specOf(j),
        target: isNew ? j.newTable.trim() : j.target,
        create: isNew ? mapped.map((m) => ({ name: m.target.trim(), type: m.type.trim() || "text" })) : undefined,
        mappings: mapped.map((m) => ({ source: m.source, target: m.target.trim() })),
        keys: isNew ? [] : j.keys,
        mode: isNew ? "append" : mode,
      };
    });
    const unlistenProgress = await onImportProgress(id, (job, c) => {
      setCurrentJob(job);
      setCounts((m) => ({ ...m, [job]: c }));
    });
    const unlistenLog = await onImportLog(id, (line) => add(line, line.startsWith("[ERR]")));
    let failed = false;
    try {
      const summary = await pg.import({
        importId: id, sessionId: target.connId, database: target.database, schema: target.schema, jobs: requests,
        advanced: {
          extendedInsert: advanced.extendedInsert,
          maxStatementKb: advanced.limitStatementSize ? Math.max(1, advanced.maxStatementKb) : 1_000_000,
          emptyAsNull: advanced.emptyAsNull,
          continueOnError: advanced.continueOnError,
        },
        convert: convertOptions(settings),
      });
      setDoneJobs(summary.jobs.length);
      summary.jobs.forEach((r, i) => setCounts((m) => ({ ...m, [i]: r })));
      for (const r of summary.jobs) {
        add(t("imp.log.result", {
          table: r.table, processed: r.processed.toLocaleString(), added: r.added.toLocaleString(), updated: r.updated.toLocaleString(),
          deleted: r.deleted.toLocaleString(), errors: r.errors.toLocaleString(), rolled: r.rolledBack ? t("imp.log.rolled") : "",
        }), r.errors > 0);
        if (r.errors > 0 || r.rolledBack) failed = true;
      }
      add(failed ? t("imp.log.partial") : t("imp.log.done"));
    } catch (e) {
      const err = toAtlasError(e);
      if (err.kind === "canceled") add(t("imp.log.canceled"));
      else add(t("imp.log.error", { message: errorText(err) }), true);
    } finally {
      unlistenProgress();
      unlistenLog();
    }
    setElapsed(performance.now() - startedAt.current);
    setRunning(false);
    onDone();
  };

  const stop = () => void pg.exportCancel(importId.current).catch(() => undefined);

  const totals = Object.values(counts).reduce(
    (a, c) => ({ processed: a.processed + c.processed, added: a.added + c.added, updated: a.updated + c.updated, deleted: a.deleted + c.deleted, errors: a.errors + c.errors }),
    { processed: 0, added: 0, updated: 0, deleted: 0, errors: 0 },
  );
  const fraction = jobs.length === 0 ? 0 : Math.min(1, doneJobs / jobs.length);

  const profileMenu: MenuEntry[] = profiles.length
    ? [
        ...profiles.map((p): MenuEntry => ({ kind: "item", text: p.name, run: () => applyProfile(p) })),
        { kind: "sep" },
        ...profiles.map((p): MenuEntry => ({ kind: "item", text: t("exp.profileDelete", { name: p.name }), run: () => persistProfiles(profiles.filter((x) => x.name !== p.name)) })),
      ]
    : [{ kind: "item", text: t("exp.profileNone") }];

  /* ---------- Etapas ---------- */
  const formatStep = (
    <div className="exp-formats" role="radiogroup" aria-label={t("imp.formatLabel")}>
      <div className="exp-formats-title">{t("imp.formatLabel")}</div>
      {FORMATS.concat(ALL_FORMATS.filter((f) => f.kind === "dbf")).map((f) => (
        <label key={f.kind} className={`exp-format${kind === f.kind ? " is-active" : ""}`}>
          <input type="radio" name="imp-format" checked={kind === f.kind} onChange={() => changeKind(f.kind)} />
          <SmallIcon name={ICONS[f.kind]} size={18} />
          <span>{t(f.label)}</span>
        </label>
      ))}
      {UNAVAILABLE_FORMATS.map((label) => (
        <label key={label} className="exp-format is-disabled" title={t("imp.unavailable")}>
          <input type="radio" name="imp-format" disabled />
          <SmallIcon name="fmtTxt" size={18} />
          <span>{t(label)}</span>
        </label>
      ))}
    </div>
  );

  const filesStep = (
    <>
      <div className="exp-files">
        <table className="exp-files-table">
          <colgroup><col style={{ width: 24 }} /><col /><col style={{ width: "28%" }} /></colgroup>
          <thead><tr><th /><th>{t("imp.colFile")}</th><th>{t("imp.colSheet")}</th></tr></thead>
          <tbody>
            {jobs.map((j) => (
              <tr key={j.id} className={j.id === selectedJob ? "is-checked" : undefined} onClick={() => setSelectedJob(j.id)}>
                <td><SmallIcon name="table" /></td>
                <td className="exp-name" title={j.file}>{j.file}</td>
                <td className="exp-name" title={j.sheet}>{j.sheet}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="exp-files-actions">
        <button type="button" className="btn" disabled={busy} onClick={() => void addFiles()}>{t("imp.addFiles")}</button>
        <button type="button" className="btn" disabled={!jobs.some((j) => j.id === selectedJob)} onClick={() => { removeJob(selectedJob); setSelectedJob(""); }}>{t("imp.remove")}</button>
        {busy && <span className="spinner" />}
      </div>
    </>
  );

  const num = (value: number, set: (n: number) => void, min = 0) => (
    <input className="field imp-num" type="number" min={min} value={value} onChange={(e) => set(Math.max(min, Math.floor(Number(e.target.value) || 0)))} />
  );
  const choice = <T extends string>(value: T, items: [T, string][], set: (v: T) => void) => (
    <select className="field" value={value} onChange={(e) => set(e.target.value as T)}>
      {items.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
    </select>
  );
  const row = (label: string, control: React.ReactNode) => (
    <div className="imp-row"><label>{label}</label>{control}</div>
  );
  const patchSettings = (p: Partial<SourceSettings>) => changeSettings({ ...settings, ...p });

  const optionsStep = (
    <div className="imp-options">
      <div className="imp-cols">
        <div className="imp-col">
          {hasGrid(kind) && (
            <>
              <label className="check-row">
                <input type="checkbox" checked={settings.fieldNameRowOn} onChange={(e) => patchSettings({ fieldNameRowOn: e.target.checked, firstDataRow: e.target.checked ? Math.max(settings.firstDataRow, settings.fieldNameRow + 1) : 1 })} />
                <span>{t("imp.fieldNameRow")}</span>
                {settings.fieldNameRowOn && num(settings.fieldNameRow, (n) => patchSettings({ fieldNameRow: n, firstDataRow: Math.max(settings.firstDataRow, n + 1) }), 1)}
              </label>
              {row(t("imp.firstDataRow"), num(settings.firstDataRow, (n) => patchSettings({ firstDataRow: n }), 1))}
              {row(t("imp.lastDataRow"), (
                <span className="imp-inline">{num(settings.lastDataRow, (n) => patchSettings({ lastDataRow: n }))}{settings.lastDataRow === 0 && <span className="muted">{t("imp.lastDataRow.all")}</span>}</span>
              ))}
            </>
          )}
          {isText(kind) && (
            <>
              {row(t("imp.delimiter"), choice(settings.delimiter, [[",", t("exp.delim.comma")], [";", t("exp.delim.semicolon")], ["\t", t("exp.delim.tab")], ["|", t("exp.delim.pipe")], [" ", t("exp.delim.space")]], (v) => patchSettings({ delimiter: v })))}
              {row(t("imp.qualifier"), choice(settings.qualifier, [['"', t("exp.qual.double")], ["'", t("exp.qual.single")], ["", t("exp.qual.none")]], (v) => patchSettings({ qualifier: v })))}
              {row(t("imp.encoding"), choice(settings.encoding, [["utf8", "UTF-8"], ["windows1252", "Windows-1252"]], (v) => patchSettings({ encoding: v })))}
            </>
          )}
        </div>
        <div className="imp-col">
          {row(t("imp.dateOrder"), choice(settings.dateOrder, [["DMY", "DD MM YY"], ["MDY", "MM DD YY"], ["YMD", "YY MM DD"]], (v) => patchSettings({ dateOrder: v })))}
          {row(t("imp.dateTimeOrder"), choice(settings.dateTimeOrder, [["DateTime", t("imp.order.dt")], ["TimeDate", t("imp.order.td")]], (v) => patchSettings({ dateTimeOrder: v })))}
          {row(t("imp.dateDelimiter"), <input className="field imp-short" maxLength={1} value={settings.dateDelimiter} onChange={(e) => patchSettings({ dateDelimiter: e.target.value })} />)}
          {row(t("imp.timeDelimiter"), <input className="field imp-short" maxLength={1} value={settings.timeDelimiter} onChange={(e) => patchSettings({ timeDelimiter: e.target.value })} />)}
          {row(t("imp.decimalSymbol"), <input className="field imp-short" maxLength={1} value={settings.decimalSymbol} onChange={(e) => patchSettings({ decimalSymbol: e.target.value })} />)}
          <div className="imp-examples">
            <div className="muted">{t("imp.examples")}</div>
            {dateExamples(settings).map((x) => <div key={x}>{x}</div>)}
          </div>
        </div>
      </div>
      <div className="imp-preview-title">{t("imp.preview")}</div>
      <div className="exp-files imp-preview">
        {preview === "loading" || preview === null ? (
          <div className="muted exp-loading"><span className="spinner" />{t("imp.reading")}</div>
        ) : "error" in preview ? (
          <div className="msg-error">{preview.error}</div>
        ) : preview.columns.length === 0 ? (
          <div className="muted">{t("imp.noPreview")}</div>
        ) : (
          <table className="exp-files-table imp-grid">
            <thead><tr>{preview.columns.map((c, i) => <th key={i} title={c}>{c}</th>)}</tr></thead>
            <tbody>
              {preview.rows.map((r, ri) => (
                <tr key={ri}>{r.map((v, ci) => <td key={ci} className={`exp-name${v === null ? " is-null" : ""}`} title={v ?? ""}>{v ?? "(null)"}</td>)}</tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );

  const targetStep = (
    <div className="exp-files">
      <table className="exp-files-table">
        <colgroup><col style={{ width: 24 }} /><col style={{ width: "36%" }} /><col /></colgroup>
        <thead><tr><th /><th>{t("imp.source")}</th><th>{t("imp.colTarget")}</th></tr></thead>
        <tbody>
          {jobs.map((j) => (
            <tr key={j.id}>
              <td><SmallIcon name="table" /></td>
              <td className="exp-name" title={`${j.file} — ${j.sheet}`}>{j.sheet === stemOf(j.file) ? stemOf(j.file) : `${stemOf(j.file)} / ${j.sheet}`}</td>
              <td className="imp-target-cell">
                <select className="field" value={j.target} onChange={(e) => setTargetOf(j.id, e.target.value)}>
                  <option value="">{t("imp.newTable")}</option>
                  {target.tables.map((n) => <option key={n} value={n}>{n}</option>)}
                </select>
                {!j.target && (
                  <input className="field" value={j.newTable} placeholder={t("imp.newTableName")} spellCheck={false} onChange={(e) => patchJob(j.id, { newTable: e.target.value })} />
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );

  const fieldsJob = jobs.find((j) => j.id === fieldJob) ?? jobs[0];
  const fieldsInfo = fieldsJob ? info[fieldJob] ?? info[fieldsJob.id] : undefined;
  const fieldsStep = !fieldsJob ? null : (
    <>
      <div className="exp-field-row">
        <label>{t("imp.source")}</label>
        <select className="field" value={fieldsJob.id} onChange={(e) => setFieldJob(e.target.value)}>
          {jobs.map((j) => <option key={j.id} value={j.id}>{`${stemOf(j.file)}${j.sheet !== stemOf(j.file) ? ` / ${j.sheet}` : ""} → ${j.target || j.newTable}`}</option>)}
        </select>
      </div>
      <div className="exp-files">
        {fieldsInfo?.error ? (
          <div className="msg-error">{fieldsInfo.error}</div>
        ) : !fieldsInfo?.source || fieldsJob.mappings.length === 0 ? (
          <div className="muted exp-loading"><span className="spinner" />{t("imp.reading")}</div>
        ) : (
          <table className="exp-files-table">
            <colgroup><col style={{ width: "36%" }} /><col />{fieldsJob.target ? <col style={{ width: 54 }} /> : <col style={{ width: "30%" }} />}</colgroup>
            <thead>
              <tr>
                <th>{t("imp.sourceField")}</th>
                <th>{t("imp.targetField")}</th>
                <th>{fieldsJob.target ? t("imp.key") : t("imp.type")}</th>
              </tr>
            </thead>
            <tbody>
              {fieldsJob.mappings.map((m, i) => (
                <tr key={m.source + i}>
                  <td className="exp-name" title={m.source}>{m.source}</td>
                  {fieldsJob.target ? (
                    <>
                      <td>
                        <select className="field imp-cell" value={m.target} onChange={(e) => patchMapping(fieldsJob, i, { target: e.target.value })}>
                          <option value="">{t("imp.skip")}</option>
                          {(fieldsInfo.targetColumns ?? []).map((c) => <option key={c} value={c}>{c}</option>)}
                        </select>
                      </td>
                      <td style={{ textAlign: "center" }}>
                        <input type="checkbox" disabled={!m.target} checked={!!m.target && fieldsJob.keys.includes(m.target)} onChange={(e) => toggleKey(fieldsJob, m.target, e.target.checked)} aria-label={t("imp.key")} />
                      </td>
                    </>
                  ) : (
                    <>
                      <td><input className="field imp-cell" value={m.target} placeholder={t("imp.skip")} spellCheck={false} onChange={(e) => patchMapping(fieldsJob, i, { target: e.target.value })} /></td>
                      <td><input className="field imp-cell" list="imp-types" value={m.type} spellCheck={false} onChange={(e) => patchMapping(fieldsJob, i, { type: e.target.value })} /></td>
                    </>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <datalist id="imp-types">{COMMON_TYPES.map((x) => <option key={x} value={x} />)}</datalist>
      </div>
      <div className="exp-fields-actions">
        <button type="button" className="btn" onClick={() => autoMap(fieldsJob)}>{t("imp.autoMap")}</button>
        <button type="button" className="btn" onClick={() => clearMap(fieldsJob)}>{t("imp.clearMap")}</button>
      </div>
      {!fieldsJob.target && <div className="muted exp-note">{t("imp.newTableNote")}</div>}
    </>
  );

  const modeStep = (
    <>
      <div className="exp-formats" role="radiogroup">
        {MODES.map((m) => (
          <label key={m.mode} className={`exp-format${mode === m.mode ? " is-active" : ""}`}>
            <input type="radio" name="imp-mode" checked={mode === m.mode} onChange={() => setMode(m.mode)} />
            <span>{t(m.label)}</span>
          </label>
        ))}
      </div>
      <div className="exp-files-actions">
        <span className="spacer" />
        <button type="button" className="btn" onClick={() => setAdvancedOpen(true)}>{t("imp.advanced")}</button>
      </div>
    </>
  );

  const current = jobs[currentJob];
  const summaryStep = (
    <>
      <dl className="exp-summary">
        <dt>{t("exp.sourceTable")}</dt><dd>{current ? (current.target || current.newTable) : "—"}{jobs.length > 1 ? ` (${Math.min(jobs.length, doneJobs + (running ? 1 : 0))}/${jobs.length})` : ""}</dd>
        <dt>{t("imp.processed")}</dt><dd>{totals.processed.toLocaleString()}</dd>
        <dt>{t("imp.added")}</dt><dd>{totals.added.toLocaleString()}</dd>
        <dt>{t("imp.updated")}</dt><dd>{totals.updated.toLocaleString()}</dd>
        <dt>{t("imp.deleted")}</dt><dd>{totals.deleted.toLocaleString()}</dd>
        <dt>{t("imp.errors")}</dt><dd>{totals.errors.toLocaleString()}</dd>
        <dt>{t("exp.time")}</dt><dd>{formatClock(elapsed)}</dd>
      </dl>
      <div className="exp-log" ref={logRef} role="log" aria-live="polite">
        {log.map((line, i) => <div key={i} className={line.error ? "is-error" : undefined}>{line.text}</div>)}
      </div>
      <div className="exp-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(fraction * 100)}>
        <div className="exp-progress-bar" style={{ width: `${Math.round(fraction * 100)}%` }} />
      </div>
    </>
  );

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget && !running && step !== LAST) onClose(); }}>
      <div className="modal exp-wizard" role="dialog" aria-label={t("imp.title")}>
        <div className="modal-header"><span className="modal-title">{running ? `${Math.round(fraction * 100)}% - ` : ""}{t("imp.title")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose} disabled={running}><SmallIcon name="close" /></button></div>
        <div className="exp-heading">{t(HEADINGS[step])}</div>
        <div className="exp-body">
          {step === 0 && formatStep}
          {step === 1 && filesStep}
          {step === 2 && optionsStep}
          {step === 3 && targetStep}
          {step === 4 && fieldsStep}
          {step === 5 && modeStep}
          {step === 6 && summaryStep}
          {message && <div className="msg-error exp-message" role="alert">{message}</div>}
        </div>
        <div className="exp-footer">
          <div className="exp-menu-wrap split-button">
            <button type="button" className="btn" disabled={step !== LAST || running || jobs.length === 0} onClick={() => setNamePrompt(true)}>{t("exp.saveProfile")}</button>
            <button type="button" className="btn" disabled={step !== LAST || running} onClick={() => setMenu((m) => (m === "profiles" ? null : "profiles"))}>{t("exp.openProfile")}</button>
            {menu === "profiles" && <PopupMenu items={profileMenu} commands={{}} onClose={() => setMenu(null)} style={{ top: "auto", bottom: "100%" }} />}
          </div>
          <span className="spacer" />
          <button type="button" className="btn" disabled={step === 0 || running || busy} onClick={() => void goTo(0)}>{t("exp.first")}</button>
          <button type="button" className="btn" disabled={step === 0 || running || busy} onClick={() => void goTo(step - 1)}>{t("exp.back")}</button>
          {step === LAST ? (
            running
              ? <button type="button" className="btn btn-primary" onClick={stop}>{t("exp.stop")}</button>
              : <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void start()}>{t("exp.start")}</button>
          ) : (
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void goTo(step + 1)}>{t("exp.next")}</button>
          )}
          <button type="button" className="btn" disabled={step === LAST || running || busy} onClick={() => void goTo(LAST)}>{t("exp.last")}</button>
          <button type="button" className="btn" disabled={running} onClick={onClose}>{t(step === LAST ? "common.close" : "common.cancel")}</button>
        </div>
      </div>

      {advancedOpen && <ImportAdvancedDialog value={advanced} onChange={setAdvanced} onClose={() => setAdvancedOpen(false)} />}
      {namePrompt && (
        <MessageDialog
          title={t("exp.saveProfile")}
          prompt={{ label: t("exp.profileName"), initial: "" }}
          okLabel={t("exp.saveProfile")}
          onOk={(name) => { if (name) saveProfile(name); }}
          onCancel={() => setNamePrompt(false)}
        />
      )}
    </div>
  );
}
