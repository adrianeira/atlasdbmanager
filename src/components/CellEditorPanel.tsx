import { useEffect, useMemo, useRef, useState } from "react";
import { chooseExportPath, chooseOpenFile, errorText, local, toAtlasError } from "../backend";
import { bytesToHex, bytesToText, cellBytes, hexDump, imageMime, textToBytes, toBase64 } from "../blob";
import { useT, type MessageKey } from "../i18n";
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

type Mode = "text" | "hex" | "image" | "web";
const MODES: { id: Mode; label: MessageKey }[] = [
  { id: "text", label: "cellEd.mode.text" },
  { id: "hex", label: "cellEd.mode.hex" },
  { id: "image", label: "cellEd.mode.image" },
  { id: "web", label: "cellEd.mode.web" },
];

/** Os bytes são UTF-8 válido? (só então dá para editar um `bytea` como texto sem estragá-lo). */
function isUtf8(b: Uint8Array): boolean {
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(b);
    return true;
  } catch {
    return false;
  }
}

/**
 * Painel "Cell Editor": edita o valor da célula ativa e o mostra como Texto, Hexa, Imagem ou Web (HTML). Em colunas binárias
 * (`bytea`) o conteúdo entra e sai por arquivo ("Carregar" e "Salvar") ou, quando é UTF-8, como texto.
 */
export function CellEditorPanel({ row, col, columnName, columnType, kind, value, readOnly, required, onChange, onClose }: Props) {
  const t = useT();
  const binary = kind === "binary";
  const [mode, setMode] = useState<Mode>("text");
  const bytes = useMemo(() => (value === null ? null : cellBytes(value, binary)), [value, binary]);
  const utf8 = useMemo(() => (bytes ? isUtf8(bytes) : true), [bytes]);
  const shownText = binary ? (bytes ? bytesToText(bytes) : "") : value ?? "";
  const [text, setText] = useState(shownText);
  const [wrap, setWrap] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

  // Trocou de célula (navegação no grid): descarta o texto não confirmado e mostra o novo valor.
  useEffect(() => {
    setText(shownText);
    setError(null);
  }, [row, col, value, shownText]);

  // Em binário não-UTF-8 o texto não é editável; as outras vistas continuam.
  const textLocked = readOnly || (binary && !utf8);

  const commit = () => {
    if (textLocked) return;
    // Fora de colunas de texto, vazio significa NULL (mesma regra do grid).
    let next: CellValue = text === "" && kind !== "text" ? null : text;
    if (binary && next !== null) next = bytesToHex(textToBytes(next));
    if (required && (next === null || next === "")) {
      setText(shownText); // reverte — igual ao grid, obrigatório não aceita vazio.
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

  const load = async () => {
    setError(null);
    try {
      const path = await chooseOpenFile(["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico", "pdf", "zip", "txt", "html", "bin"]);
      if (!path) return;
      const hex = await local.fileReadHex(path);
      if (binary) return void onChange(hex);
      // Coluna de texto: o arquivo entra como texto UTF-8.
      const b = cellBytes(hex, true);
      if (!isUtf8(b)) return setError(t("cellEd.notText"));
      onChange(bytesToText(b));
    } catch (e) {
      setError(errorText(toAtlasError(e)));
    }
  };

  const save = async () => {
    if (!bytes) return;
    setError(null);
    try {
      const mime = imageMime(bytes);
      const ext = mime ? mime.replace("image/", "").replace("svg+xml", "svg").replace("x-icon", "ico").replace("jpeg", "jpg") : binary ? "bin" : "txt";
      const path = await chooseExportPath(`${columnName}.${ext}`, [ext]);
      if (path) await local.fileWriteHex(path, bytesToHex(bytes));
    } catch (e) {
      setError(errorText(toAtlasError(e)));
    }
  };

  const mime = bytes ? imageMime(bytes) : null;
  // Texto que já é uma imagem em `data:` (ex.: um logotipo guardado em base64).
  const dataUri = !binary && value !== null && /^data:image\/[a-z0-9.+-]+;base64,[A-Za-z0-9+/=\s]+$/i.test(value) ? value.replace(/\s+/g, "") : null;
  const imageSrc = mime && bytes ? `data:${mime};base64,${toBase64(bytes)}` : dataUri;

  const body = () => {
    if (mode === "hex") {
      return bytes
        ? <pre className="cell-editor-hex">{hexDump(bytes)}{bytes.length > 64 * 1024 ? `\n\n${t("cellEd.hexLimit")}` : ""}</pre>
        : <div className="cell-editor-empty muted">{t("cellEd.noValue")}</div>;
    }
    if (mode === "image") {
      if (!bytes) return <div className="cell-editor-empty muted">{t("cellEd.noValue")}</div>;
      return imageSrc
        ? <div className="cell-editor-image"><img src={imageSrc} alt={columnName} /></div>
        : <div className="cell-editor-empty muted">{t("cellEd.notImage")}</div>;
    }
    if (mode === "web") {
      if (!bytes) return <div className="cell-editor-empty muted">{t("cellEd.noValue")}</div>;
      // Sem scripts nem acesso à página: só mostra o HTML do valor.
      return <iframe className="cell-editor-web" title={columnName} sandbox="" srcDoc={bytesToText(bytes)} />;
    }
    return (
      <textarea
        ref={taRef}
        className="cell-editor-text"
        style={{ whiteSpace: wrap ? "pre-wrap" : "pre" }}
        wrap={wrap ? "soft" : "off"}
        value={text}
        placeholder={value === null ? "(Null)" : binary && !utf8 ? t("cellEd.binaryHint") : ""}
        readOnly={textLocked}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.ctrlKey) { e.preventDefault(); commit(); }
        }}
      />
    );
  };

  return (
    <div className="cell-editor-panel">
      <div className="cell-editor-head">
        <span className="cell-editor-col">{columnName}</span>
        <span className="muted cell-editor-type">{columnType}{bytes && binary ? ` · ${bytes.length} bytes` : ""}</span>
        <div className="cell-editor-modes" role="tablist">
          {MODES.map((m) => (
            <button key={m.id} type="button" role="tab" aria-selected={mode === m.id} className={`cell-editor-mode${mode === m.id ? " is-active" : ""}`} onClick={() => setMode(m.id)}>{t(m.label)}</button>
          ))}
        </div>
        <span className="spacer" />
        {mode === "text" && (
          <label className="check-row">
            <input type="checkbox" checked={wrap} onChange={(e) => setWrap(e.target.checked)} />
            <span>{t("cellEd.wrap")}</span>
          </label>
        )}
        {mode === "text" && kind === "json" && (
          <button type="button" className="btn" disabled={readOnly} onClick={formatJson}>{t("cellEd.formatJson")}</button>
        )}
        <button type="button" className="btn" disabled={readOnly} onClick={() => void load()}>{t("cellEd.load")}</button>
        <button type="button" className="btn" disabled={!bytes} onClick={() => void save()}>{t("cellEd.save")}</button>
        <button
          type="button"
          className="btn"
          disabled={readOnly || required}
          onClick={() => { setText(""); onChange(null); }}
        >
          {t("grid.setNull")}
        </button>
        <button type="button" className="icon-button" title={t("common.apply")} disabled={textLocked || mode !== "text"} onClick={commit}><SmallIcon name="apply" /></button>
        <button type="button" className="find-close" title={t("data.find.close")} aria-label={t("data.find.close")} onClick={onClose}>✕</button>
      </div>
      {error && <div className="doc-banner is-warn">{error}</div>}
      {body()}
    </div>
  );
}
