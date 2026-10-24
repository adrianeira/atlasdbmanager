import { useState } from "react";
import type { OptionPair } from "../backend";
import { useT } from "../i18n";
import { isSecretOption } from "../sql/foreignServer";

/** Linhas chave/valor com adicionar e remover; o valor de uma senha sai mascarado. */
export function OptionsGrid({ rows, onChange, label }: { rows: OptionPair[]; onChange: (rows: OptionPair[]) => void; label: string }) {
  const t = useT();
  const [sel, setSel] = useState(0);
  const set = (i: number, p: Partial<OptionPair>) => onChange(rows.map((r, k) => (k === i ? { ...r, ...p } : r)));
  return (
    <div className="fs-grid" role="group" aria-label={label}>
      <div className="fs-head"><span>{t("fsv.optionName")}</span><span>{t("fsv.optionValue")}</span></div>
      <div className="fs-rows">
        {rows.map((r, i) => (
          <div key={i} className={`fs-row${sel === i ? " is-sel" : ""}`} onClick={() => setSel(i)}>
            <input className="field" value={r.key} spellCheck={false} aria-label={t("fsv.optionName")} onChange={(e) => set(i, { key: e.target.value })} />
            <input className="field" type={isSecretOption(r.key) ? "password" : "text"} value={r.value} spellCheck={false} aria-label={t("fsv.optionValue")} onChange={(e) => set(i, { value: e.target.value })} />
          </div>
        ))}
      </div>
      <div className="fs-tools">
        <button type="button" className="icon-button" aria-label="+" onClick={() => { onChange([...rows, { key: "", value: "" }]); setSel(rows.length); }}>+</button>
        <button type="button" className="icon-button" aria-label="−" disabled={!rows.length} onClick={() => { onChange(rows.filter((_, k) => k !== sel)); setSel(Math.max(0, sel - 1)); }}>−</button>
      </div>
    </div>
  );
}
