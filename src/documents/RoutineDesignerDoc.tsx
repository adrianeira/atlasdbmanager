import { useEffect, useMemo, useRef, useState } from "react";
import { errorText, toAtlasError } from "../backend";
import { FullscreenButton } from "../components/FullscreenButton";
import { SqlEditor, type SqlEditorHandle } from "../components/SqlEditor";
import type { StatusCell } from "../components/StatusBar";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import { beautifySql } from "../sql/format";
import { routineName, routineTemplate, routineSaveScript, type RoutineDialect } from "../sql/routine";

/** Acesso ao banco real. */
export interface RoutineBackend {
  /** O texto do procedimento ou função (`CREATE ...`). */
  loadDefinition: (name: string) => Promise<string>;
  /** Executa o script de gravação. */
  run: (sql: string) => Promise<void>;
}

interface Props {
  dialect: RoutineDialect;
  schema: string;
  /** Ausente = rotina nova. */
  routineName?: string;
  active: boolean;
  completionSchema?: Record<string, string[]>;
  onStatus: (cells: StatusCell[]) => void;
  onDirty: (dirty: boolean) => void;
  /** Depois de gravar: o nome que a rotina tem no servidor. */
  onSaved: (name: string) => void;
  backend?: RoutineBackend;
}

/** Designer de procedimentos e funções do SQL Server e do MySQL: o texto da rotina e o botão de gravar. */
export function RoutineDesignerDoc({ dialect, schema, routineName: initialName, active, completionSchema, onStatus, onDirty, onSaved, backend }: Props) {
  const t = useT();
  const editor = useRef<SqlEditorHandle | null>(null);
  const template = useMemo(() => routineTemplate(dialect, schema), [dialect, schema]);
  const [name, setName] = useState(initialName);
  const [original, setOriginal] = useState(initialName ? "" : template);
  const [text, setText] = useState(initialName ? "" : template);
  const [loading, setLoading] = useState(!!initialName && !!backend);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

  useEffect(() => {
    if (!initialName || !backend) return;
    let alive = true;
    backend
      .loadDefinition(initialName)
      .then((def) => {
        if (!alive) return;
        setOriginal(def);
        setText(def);
      })
      .catch((e) => alive && setLoadError(errorText(toAtlasError(e))))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // Uma vez ao abrir a aba.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const dirty = text.trim() !== original.trim();
  useEffect(() => onDirty(dirty), [dirty, onDirty]);
  useEffect(() => {
    if (active) onStatus([{ text: dirty ? t("des.unsaved") : "", grow: true }]);
  }, [active, dirty, t, onStatus]);

  const save = async () => {
    const newName = routineName(text);
    if (!newName) {
      setMessage({ text: t("rt.noName"), error: true });
      return;
    }
    if (!backend || saving) return;
    setSaving(true);
    setMessage(null);
    try {
      try {
        await backend.run(routineSaveScript(dialect, text));
      } catch (e) {
        // No MySQL o `DROP` já pode ter rodado: recria a versão que estava gravada para não perder a rotina.
        if (dialect === "mysql" && name && name === newName && original.trim()) await backend.run(routineSaveScript(dialect, original)).catch(() => undefined);
        throw e;
      }
      setOriginal(text);
      setName(newName);
      setMessage({ text: t("rt.saved", { name: newName }), error: false });
      onSaved(newName);
    } catch (e) {
      setMessage({ text: errorText(toAtlasError(e)), error: true });
    } finally {
      setSaving(false);
    }
  };

  if (loading) return <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>;

  return (
    <div className="doc">
      {loadError && <div className="doc-banner is-warn">{loadError}</div>}
      <div className="object-toolbar" role="toolbar">
        <button type="button" className="ot-button" disabled={!dirty || !!loadError || saving} onClick={() => void save()} title={t("des.save")}>
          <SmallIcon name="save" />
          <span>{t("des.save")}</span>
        </button>
        <span className="ot-sep" />
        <button
          type="button"
          className="ot-button"
          disabled={!text.trim()}
          onClick={() => { const cur = editor.current?.getText() ?? ""; if (cur.trim()) editor.current?.replaceAll(beautifySql(cur)); }}
          title={t("query.beautify")}
        >
          <SmallIcon name="beautify" />
          <span>{t("query.beautify")}</span>
        </button>
        {saving && <span className="spinner" />}
        <div className="ot-spacer" />
        <FullscreenButton />
      </div>
      <div className="des-body">
        <div className="des-main">
          <div className="query-body">
            <div className="query-editor" style={{ flexBasis: message ? "75%" : "100%" }}>
              <SqlEditor
                value={text}
                schema={completionSchema}
                defaultSchema={schema}
                dialect={dialect}
                onChange={setText}
                onRun={() => void save()}
                handleRef={(h) => { editor.current = h; }}
              />
            </div>
            {message && (
              <div className="query-results">
                <div className="result-body">
                  <pre className={`result-message${message.error ? " is-error" : ""}`}>{message.text}</pre>
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
