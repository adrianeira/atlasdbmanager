import { useEffect, useState } from "react";
import { useT, type MessageKey } from "../i18n";
import { SmallIcon } from "../icons";
import { paramProblem, type ParamType, type ParamValue } from "../sql/params";

interface Props {
  names: string[];
  /** Valores já usados nesta aba (por nome), para não digitar de novo. */
  initial: Record<string, ParamValue>;
  onOk: (values: Record<string, ParamValue>) => void;
  onCancel: () => void;
}

const TYPES: { id: ParamType; label: MessageKey }[] = [
  { id: "text", label: "par.text" },
  { id: "number", label: "par.number" },
  { id: "null", label: "par.null" },
  { id: "raw", label: "par.raw" },
];

/** Janela "Parameters": um valor (com o tipo do literal) para cada `:nome` da consulta que vai rodar. */
export function ParametersDialog({ names, initial, onOk, onCancel }: Props) {
  const t = useT();
  const [values, setValues] = useState<Record<string, ParamValue>>(() => Object.fromEntries(names.map((n) => [n, initial[n] ?? { type: "text" as ParamType, value: "" }])));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onCancel(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const set = (name: string, p: Partial<ParamValue>) => {
    setError(null);
    setValues((v) => ({ ...v, [name]: { ...v[name], ...p } }));
  };

  const ok = () => {
    for (const name of names) {
      const problem = paramProblem(values[name]);
      if (problem) return setError(t(problem === "number" ? "par.badNumber" : "par.badRaw", { name }));
    }
    onOk(values);
  };

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal par-dialog" role="dialog" aria-label={t("par.title")}>
        <div className="modal-header">
          <span className="modal-title">{t("par.title")}</span>
          <button type="button" className="modal-close" aria-label={t("tab.close")} onClick={onCancel}><SmallIcon name="close" /></button>
        </div>
        <div className="modal-body">
          <div className="muted">{t("par.intro")}</div>
          <div className="par-table">
            <span className="par-head">{t("par.name")}</span>
            <span className="par-head">{t("par.type")}</span>
            <span className="par-head">{t("par.value")}</span>
            {names.map((name, i) => (
              <ParamRow
                key={name}
                name={name}
                value={values[name]}
                first={i === 0}
                onChange={(p) => set(name, p)}
                onEnter={ok}
              />
            ))}
          </div>
          {error && <div className="form-note form-note-warn">{error}</div>}
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={ok}>OK</button>
          <button type="button" className="btn" onClick={onCancel}>{t("opt.cancel")}</button>
        </div>
      </div>
    </div>
  );
}

function ParamRow({ name, value, first, onChange, onEnter }: { name: string; value: ParamValue; first: boolean; onChange: (p: Partial<ParamValue>) => void; onEnter: () => void }) {
  const t = useT();
  return (
    <>
      <label htmlFor={`par-${name}`}>:{name}</label>
      <select className="field" value={value.type} onChange={(e) => onChange({ type: e.target.value as ParamType })}>
        {TYPES.map((x) => <option key={x.id} value={x.id}>{t(x.label)}</option>)}
      </select>
      <input
        id={`par-${name}`}
        className="field"
        value={value.type === "null" ? "" : value.value}
        disabled={value.type === "null"}
        placeholder={value.type === "null" ? "NULL" : ""}
        spellCheck={false}
        autoFocus={first}
        onChange={(e) => onChange({ value: e.target.value })}
        onKeyDown={(e) => { if (e.key === "Enter") onEnter(); }}
      />
    </>
  );
}
