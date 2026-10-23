import type { ReactNode } from "react";
import type { ExportEncoding } from "../../backend";
import { useT } from "../../i18n";
import type { DelimitedOptions, Kind, Options } from "./exportModel";
import { SmallIcon } from "../../icons";

interface Props {
  /** Exportando o resultado de uma consulta: o SQL pede o nome da tabela de destino. */
  isQuery?: boolean;
  kind: Kind;
  options: Options;
  onChange: (next: Options) => void;
  onClose: () => void;
}

/** Botão "Avançado" da etapa de destino: opções do formato escolhido. */
export function ExportOptionsDialog({ isQuery, kind, options, onChange, onClose }: Props) {
  const t = useT();

  const select = <T extends string>(value: T, items: [T, string][], set: (v: T) => void) => (
    <select className="field" value={value} onChange={(e) => set(e.target.value as T)}>
      {items.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
    </select>
  );
  const row = (label: string, control: ReactNode) => (
    <div className="exp-row">
      <label>{label}</label>
      {control}
    </div>
  );

  const delimited = (key: "txt" | "csv") => {
    const o = options[key];
    const patch = (p: Partial<DelimitedOptions>) => onChange({ ...options, [key]: { ...o, ...p } });
    return (
      <>
        {row(t("exp.delimiter"), select(o.delimiter, [[",", t("exp.delim.comma")], [";", t("exp.delim.semicolon")], ["\t", t("exp.delim.tab")], ["|", t("exp.delim.pipe")], [" ", t("exp.delim.space")]], (v) => patch({ delimiter: v })))}
        {row(t("exp.qualifier"), select(o.qualifier, [['"', t("exp.qual.double")], ["'", t("exp.qual.single")], ["", t("exp.qual.none")]], (v) => patch({ qualifier: v })))}
        {row(t("exp.encoding"), select<ExportEncoding>(o.encoding, [["utf8", "UTF-8"], ["utf8Bom", t("exp.enc.utf8bom")], ["windows1252", "Windows-1252"]], (v) => patch({ encoding: v })))}
        {row(t("exp.lineEnd"), select(o.crlf ? "crlf" : "lf", [["crlf", "CRLF (Windows)"], ["lf", "LF (Linux/macOS)"]], (v) => patch({ crlf: v === "crlf" })))}
        {row(t("exp.nullText"), <input className="field" value={o.nullText} placeholder={t("exp.nullText.empty")} onChange={(e) => patch({ nullText: e.target.value })} spellCheck={false} />)}
        <label className="check-row"><input type="checkbox" checked={o.header} onChange={(e) => patch({ header: e.target.checked })} /><span>{t("exp.header")}</span></label>
        {o.encoding === "windows1252" && <div className="muted exp-note">{t("exp.enc.win1252Note")}</div>}
      </>
    );
  };

  const body = () => {
    switch (kind) {
      case "txt":
      case "csv":
        return delimited(kind);
      case "json":
        return <label className="check-row"><input type="checkbox" checked={options.json.pretty} onChange={(e) => onChange({ ...options, json: { pretty: e.target.checked } })} /><span>{t("exp.jsonPretty")}</span></label>;
      case "html":
        return <label className="check-row"><input type="checkbox" checked={options.html.header} onChange={(e) => onChange({ ...options, html: { header: e.target.checked } })} /><span>{t("exp.header")}</span></label>;
      case "sql":
        return (
          <>
            {isQuery && row(t("exp.sqlTableName"), <input className="field" value={options.sql.tableName} placeholder={t("exp.sqlTableName.auto")} onChange={(e) => onChange({ ...options, sql: { ...options.sql, tableName: e.target.value } })} spellCheck={false} />)}
            {row(t("exp.rowsPerInsert"), (
              <input
                className="field exp-num"
                inputMode="numeric"
                value={options.sql.rowsPerInsert}
                onChange={(e) => onChange({ ...options, sql: { ...options.sql, rowsPerInsert: Number(e.target.value.replace(/\D/g, "")) || 0 } })}
                onBlur={() => onChange({ ...options, sql: { ...options.sql, rowsPerInsert: Math.max(1, Math.min(1000, options.sql.rowsPerInsert || 1)) } })}
              />
            ))}
            <div className="muted exp-note">{t("exp.sqlNote")}</div>
          </>
        );
      case "xlsx":
        return (
          <>
            {row(t("exp.sheetName"), <input className="field" value={options.xlsx.sheetName} placeholder={t("exp.sheetName.auto")} onChange={(e) => onChange({ ...options, xlsx: { ...options.xlsx, sheetName: e.target.value } })} spellCheck={false} />)}
            <label className="check-row"><input type="checkbox" checked={options.xlsx.header} onChange={(e) => onChange({ ...options, xlsx: { ...options.xlsx, header: e.target.checked } })} /><span>{t("exp.header")}</span></label>
            <div className="muted exp-note">{t("exp.xlsxNote")}</div>
          </>
        );
      case "xls":
        return (
          <>
            {row(t("exp.sheetName"), <input className="field" value={options.xls.sheetName} placeholder={t("exp.sheetName.auto")} onChange={(e) => onChange({ ...options, xls: { ...options.xls, sheetName: e.target.value } })} spellCheck={false} />)}
            <label className="check-row"><input type="checkbox" checked={options.xls.header} onChange={(e) => onChange({ ...options, xls: { ...options.xls, header: e.target.checked } })} /><span>{t("exp.header")}</span></label>
            <div className="muted exp-note">{t("exp.xlsNote")}</div>
          </>
        );
      case "dbf": {
        const num = (key: "decimals" | "textLength", min: number, max: number) => (
          <input
            className="field exp-num"
            inputMode="numeric"
            value={options.dbf[key]}
            onChange={(e) => onChange({ ...options, dbf: { ...options.dbf, [key]: Number(e.target.value.replace(/\D/g, "")) || 0 } })}
            onBlur={() => onChange({ ...options, dbf: { ...options.dbf, [key]: Math.max(min, Math.min(max, options.dbf[key] || max)) } })}
          />
        );
        return (
          <>
            {row(t("exp.dbfDecimals"), num("decimals", 0, 14))}
            {row(t("exp.dbfTextLength"), num("textLength", 1, 254))}
            <div className="muted exp-note">{t("exp.dbfNote")}</div>
          </>
        );
      }
      case "xml":
        return <div className="muted exp-note">{t("exp.noOptions")}</div>;
    }
  };

  return (
    <div className="modal-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal exp-options-dialog" role="dialog" aria-label={t("exp.advancedTitle")}>
        <div className="modal-header"><span className="modal-title">{t("exp.advancedTitle")} — {t(`exp.kind.${kind}`)}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="modal-body exp-options">{body()}</div>
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={onClose}>OK</button>
        </div>
      </div>
    </div>
  );
}
