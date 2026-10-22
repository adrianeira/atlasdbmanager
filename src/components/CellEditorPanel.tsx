import { useEffect, useRef, useState } from "react";
import { useT } from "../i18n";
import type { CellValue } from "../model";
import { SmallIcon } from "../icons";
import type { TypeKind } from "./DataGrid";

interface Props {
  /** Linha/coluna ativas (só para saber quando trocar de célula e resetar o texto local). */
  row: number;
  col: number;
  columnName: string;
  columnType: string;
  kind: TypeKind;
  value: CellValue;
  readOnly: boolean;
  required: boolean;
  onChange: (v: CellValue) => void;
  onClose: () => void;
}

/** Painel "Cell Editor": edita o valor da célula ativa num texto grande, com quebra de linha. */
export function CellEditorPanel({ row, col, columnName, columnType, kind, value, readOnly, required, onChange, onClose }: Props) {
  const t = useT();
  const [text, setText] = useState(value ?? "");
  const [wrap, setWrap] = useState(true);
  const taRef = useRef<HTMLTextAreaElement>(null);

  // Trocou de célula (navegação no grid): descarta o texto não confirmado e mostra o novo valor.
  useEffect(() => {
    setText(value ?? "");
  }, [row, col, value]);

  const commit = () => {
    if (readOnly) return;
    // Fora de colunas de texto, vazio significa NULL (mesma regra do grid).
    const next: CellValue = text === "" && kind !== "text" ? null : text;
    if (required && (next === null || next === "")) {
      setText(value ?? ""); // reverte — igual ao grid, obrigatório não aceita vazio.
      return;
    }
    if (next !== value) onChange(next);
  };

  const formatJson = () => {
    try {
      setText(JSON.stringify(JSON.parse(text), null, 2));
    } catch {
      // Texto não é JSON válido — não mexe, o usuário continua editando.
    }
  };

  return (
    <div className="cell-editor-panel">
      <div className="cell-editor-head">
        <span className="cell-editor-col">{columnName}</span>
        <span className="muted cell-editor-type">{columnType}</span>
        <span className="spacer" />
        <label className="check-row">
          <input type="checkbox" checked={wrap} onChange={(e) => setWrap(e.target.checked)} />
          <span>{t("cellEd.wrap")}</span>
        </label>
        {kind === "json" && (
          <button type="button" className="find-button" disabled={readOnly} onClick={formatJson}>{t("cellEd.formatJson")}</button>
        )}
        <button
          type="button"
          className="find-button"
          disabled={readOnly || required}
          onClick={() => { setText(""); onChange(null); }}
        >
          {t("grid.setNull")}
        </button>
        <button type="button" className="icon-button" title={t("common.apply")} disabled={readOnly} onClick={commit}><SmallIcon name="apply" /></button>
        <button type="button" className="find-close" title={t("data.find.close")} aria-label={t("data.find.close")} onClick={onClose}>✕</button>
      </div>
      <textarea
        ref={taRef}
        className="cell-editor-text"
        style={{ whiteSpace: wrap ? "pre-wrap" : "pre" }}
        wrap={wrap ? "soft" : "off"}
        value={text}
        placeholder={value === null ? "(Null)" : ""}
        readOnly={readOnly}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.ctrlKey) { e.preventDefault(); commit(); }
        }}
      />
    </div>
  );
}
