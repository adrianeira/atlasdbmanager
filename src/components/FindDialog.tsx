import { useEffect, useRef, useState } from "react";
import { errorText, toAtlasError, type FindHit, type FindOptions, type FindResult } from "../backend";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";

interface Props {
  connectionName: string;
  database: string;
  /** Aberto a partir de um schema: a busca começa restrita a ele. */
  schema?: string;
  search: (options: FindOptions) => Promise<FindResult>;
  onOpen: (hit: FindHit) => void;
  onClose: () => void;
}

const KIND_LABEL: Record<FindHit["kind"], MessageKey> = {
  table: "tb.table", view: "tb.view", matview: "cat.matview", sequence: "cat.sequence", type: "cat.type", function: "tb.function",
};
const KIND_ICON: Record<FindHit["kind"], SmallIconName> = {
  table: "table", view: "view", matview: "matview", sequence: "sequence", type: "design", function: "function",
};
const MATCH_LABEL: Record<FindHit["matchedIn"], MessageKey> = {
  name: "find.inName", column: "find.inColumn", definition: "find.inDefinition", comment: "find.inComment",
};

export function FindDialog({ connectionName, database, schema, search, onOpen, onClose }: Props) {
  const t = useT();
  const [text, setText] = useState("");
  const [onlySchema, setOnlySchema] = useState(!!schema);
  const [names, setNames] = useState(true);
  const [columns, setColumns] = useState(true);
  const [definitions, setDefinitions] = useState(true);
  const [comments, setComments] = useState(true);
  const [matchCase, setMatchCase] = useState(false);
  const [regex, setRegex] = useState(false);
  const [result, setResult] = useState<FindResult | null>(null);
  const [searchedFor, setSearchedFor] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState(0);
  const seq = useRef(0);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const canSearch = text.trim() !== "" && (names || columns || definitions || comments) && !busy;
  const run = () => {
    if (!canSearch) return;
    const mine = ++seq.current;
    setBusy(true);
    setError(null);
    search({ text, schema: onlySchema && schema ? schema : null, matchCase, regex, names, columns, definitions, comments })
      .then((r) => {
        if (mine !== seq.current) return;
        setResult(r);
        setSearchedFor(text);
        setSelected(0);
      })
      .catch((e) => mine === seq.current && setError(errorText(toAtlasError(e))))
      .finally(() => mine === seq.current && setBusy(false));
  };

  const hit = result?.hits[selected];
  const label = (h: FindHit) => [h.schema, h.name + (h.identity !== null ? `(${h.identity})` : ""), h.column].filter((x) => x !== null && x !== "").join(".");
  const check = (checked: boolean, set: (v: boolean) => void, key: MessageKey) => (
    <label className="check-row"><input type="checkbox" checked={checked} onChange={(e) => set(e.target.checked)} /><span>{t(key)}</span></label>
  );

  return (
    <div className="modal-overlay" role="presentation" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal find-dialog" role="dialog" aria-label={t("find.title")}>
        <div className="modal-header">{t(schema ? "find.titleSchema" : "find.title", { name: schema ?? "" })} — {connectionName} / {database}</div>
        <div className="find-form">
          <div className="find-row">
            <input
              className="field find-text"
              value={text}
              placeholder={t("find.placeholder")}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") run(); }}
              spellCheck={false}
              autoFocus
            />
            <button type="button" className="btn btn-primary" disabled={!canSearch} onClick={run}>{t("find.search")}</button>
          </div>
          <div className="find-options">
            {check(names, setNames, "find.optNames")}
            {check(columns, setColumns, "find.optColumns")}
            {check(definitions, setDefinitions, "find.optDefinitions")}
            {check(comments, setComments, "find.optComments")}
            <span className="find-sep" />
            {check(matchCase, setMatchCase, "find.matchCase")}
            {check(regex, setRegex, "find.regex")}
            {schema && check(onlySchema, setOnlySchema, "find.onlySchema")}
          </div>
        </div>
        <div className="find-results" role="listbox" aria-label={t("find.results")}>
          {result && result.hits.length > 0 && (
            <table className="grid-table" role="grid">
              <thead>
                <tr><th>{t("find.colObject")}</th><th>{t("find.colKind")}</th><th>{t("find.colFound")}</th><th>{t("find.colText")}</th></tr>
              </thead>
              <tbody>
                {result.hits.map((h, i) => (
                  <tr
                    key={i}
                    role="option"
                    aria-selected={i === selected}
                    className={i === selected ? "is-selected" : undefined}
                    onClick={() => setSelected(i)}
                    onDoubleClick={() => onOpen(h)}
                  >
                    <td><span className="cell-name"><SmallIcon name={KIND_ICON[h.kind]} />{label(h)}</span></td>
                    <td>{t(KIND_LABEL[h.kind])}</td>
                    <td>{t(MATCH_LABEL[h.matchedIn])}</td>
                    <td className="find-snippet">{h.snippet}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {result && result.hits.length === 0 && <div className="hist-empty">{t("find.none", { text: searchedFor })}</div>}
          {!result && !busy && !error && <div className="hist-empty">{t("find.hint")}</div>}
          {busy && <div className="hist-empty"><span className="spinner" /></div>}
        </div>
        {error && <div className="doc-banner is-warn">{error}</div>}
        <div className="modal-footer hist-footer">
          <span className="muted">
            {result && result.hits.length > 0 ? (result.truncated ? t("find.truncated", { n: result.hits.length }) : t("find.count", { n: result.hits.length })) : ""}
          </span>
          <span className="spacer" />
          <button type="button" className="btn" disabled={!hit} onClick={() => hit && onOpen(hit)}>{t("find.open")}</button>
          <button type="button" className="btn btn-primary" onClick={onClose}>{t("tab.close")}</button>
        </div>
      </div>
    </div>
  );
}
