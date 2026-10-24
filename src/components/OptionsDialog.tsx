import { useEffect, useState } from "react";
import { errorText, local, toAtlasError } from "../backend";
import { useT, type Lang, type MessageKey } from "../i18n";
import { DEFAULT_OPTIONS, EDITOR_COLOR_KEYS, OPTION_LIMITS, sanitizeOptions, type AppOptions, type EditorColorKey, type RowShading } from "../options";
import { MessageDialog } from "./MessageDialog";
import { SmallIcon } from "../icons";

import { formatNumber, formatTemporal } from "../format";

/** Valor de exemplo para mostrar como cada formato fica. */
const SAMPLE = { date: "2026-10-24", time: "15:30:45.123", datetime: "2026-10-24 15:30:45.123" } as const;

type Theme = "system" | "light" | "dark";

/** O que a tela edita fora de `AppOptions`: tema, idioma e legendas da barra (ficam no layout). */
export interface AppearanceOptions {
  theme: Theme;
  lang: Lang;
  showCaptions: boolean;
}

interface Props {
  options: AppOptions;
  appearance: AppearanceOptions;
  onApply: (options: AppOptions, appearance: AppearanceOptions) => void;
  onClose: () => void;
}

type Tab = "general" | "tabs" | "completion" | "editor" | "data" | "history" | "recovery" | "storage";
type NumKey = "editorFontSize" | "queryMaxRows" | "pageSize" | "tabWidth" | "autoSaveSeconds";
type BoolKey = { [K in keyof AppOptions]: AppOptions[K] extends boolean ? K : never }[keyof AppOptions];
const TABS: { id: Tab; label: MessageKey }[] = [
  { id: "general", label: "opt.general" },
  { id: "tabs", label: "opt.tabs" },
  { id: "completion", label: "opt.completion" },
  { id: "editor", label: "opt.editor" },
  { id: "data", label: "opt.data" },
  { id: "recovery", label: "opt.recovery" },
  { id: "history", label: "opt.history" },
  { id: "storage", label: "opt.storage" },
];

/** A cor que o tema usa hoje para o elemento (mostrada no seletor enquanto não há cor própria). */
function defaultColor(k: EditorColorKey): string {
  const probe = document.createElement("span");
  probe.style.display = "none";
  document.body.appendChild(probe);
  const css = { common: "var(--c-text)", keyword: "var(--sql-keyword)", string: "var(--sql-string)", number: "var(--sql-number)", comment: "var(--sql-comment)", background: "var(--c-surface)" }[k];
  probe.style.color = css;
  const rgb = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(getComputedStyle(probe).color);
  probe.remove();
  return rgb ? `#${[rgb[1], rgb[2], rgb[3]].map((n) => Number(n).toString(16).padStart(2, "0")).join("")}` : "#000000";
}

export function OptionsDialog({ options, appearance, onApply, onClose }: Props) {
  const t = useT();
  const [tab, setTab] = useState<Tab>("general");
  const [opts, setOpts] = useState<AppOptions>(options);
  const [look, setLook] = useState<AppearanceOptions>(appearance);
  // Os campos numéricos são texto enquanto se digita; viram número válido ao sair do campo.
  const numbers = (o: AppOptions) => ({ editorFontSize: String(o.editorFontSize), queryMaxRows: String(o.queryMaxRows), pageSize: String(o.pageSize), tabWidth: String(o.tabWidth), autoSaveSeconds: String(o.autoSaveSeconds) });
  const [text, setText] = useState(numbers(options));
  const [recording, setRecording] = useState(true);
  const [keep, setKeep] = useState("2000");
  const [info, setInfo] = useState<{ path: string; schemaVersion: number | null; error: string | null } | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    void local.getPref<boolean>("history.enabled").then((v) => setRecording(v !== false)).catch(() => undefined);
    void local.getPref<number>("history.max").then((v) => { if (typeof v === "number" && v >= 10) setKeep(String(v)); }).catch(() => undefined);
    void local.status().then(setInfo).catch(() => undefined);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !confirmClear) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, confirmClear]);

  const num = (key: NumKey) => (
    <input
      className="field exp-num"
      inputMode="numeric"
      value={text[key]}
      onChange={(e) => setText({ ...text, [key]: e.target.value.replace(/\D/g, "") })}
      onBlur={() => {
        const v = sanitizeOptions({ ...opts, [key]: text[key] })[key];
        setOpts({ ...opts, [key]: v });
        setText({ ...text, [key]: String(v) });
      }}
    />
  );
  const limit = (key: NumKey) => `${OPTION_LIMITS[key][0].toLocaleString()} – ${OPTION_LIMITS[key][1].toLocaleString()}`;
  const check = (key: BoolKey, label: MessageKey, indent = false, disabled = false) => (
    <label className={`check-row${indent ? " opt-indent" : ""}${disabled ? " is-disabled" : ""}`}>
      <input type="checkbox" checked={opts[key]} disabled={disabled} onChange={(e) => setOpts({ ...opts, [key]: e.target.checked })} /><span>{t(label)}</span>
    </label>
  );
  const row = (label: MessageKey, control: React.ReactNode, hint?: string) => (
    <div className="exp-row">
      <label>{t(label)}</label>
      {control}
      {hint && <span className="muted">{hint}</span>}
    </div>
  );

  const apply = async () => {
    setError(null);
    const finalOptions = sanitizeOptions({ ...opts, ...text });
    try {
      await local.setPref("ui.options", finalOptions);
      await local.setPref("history.enabled", recording);
      const n = Math.max(10, Math.floor(Number(keep)) || 2000);
      await local.setPref("history.max", n);
    } catch (e) {
      setError(errorText(toAtlasError(e)));
      return;
    }
    onApply(finalOptions, look);
    onClose();
  };

  const restore = () => {
    setOpts(DEFAULT_OPTIONS);
    setText(numbers(DEFAULT_OPTIONS));
    setKeep("2000");
    setRecording(true);
  };

  const body = () => {
    switch (tab) {
      case "general":
        return (
          <>
            {row("opt.language", (
              <select className="field" value={look.lang} onChange={(e) => setLook({ ...look, lang: e.target.value as Lang })}>
                <option value="pt-BR">Português (Brasil)</option>
                <option value="en-US">English</option>
              </select>
            ))}
            {row("opt.theme", (
              <select className="field" value={look.theme} onChange={(e) => setLook({ ...look, theme: e.target.value as Theme })}>
                <option value="system">{t("opt.themeSystem")}</option>
                <option value="light">{t("opt.themeLight")}</option>
                <option value="dark">{t("opt.themeDark")}</option>
              </select>
            ))}
            <label className="check-row"><input type="checkbox" checked={look.showCaptions} onChange={(e) => setLook({ ...look, showCaptions: e.target.checked })} /><span>{t("opt.captions")}</span></label>
            {check("multipleForms", "opt.multipleForms")}
          </>
        );
      case "tabs":
        return (
          <>
            <div className="opt-section">{t("opt.onStartup")}</div>
            {(["objects", "continue"] as const).map((v) => (
              <label key={v} className="check-row">
                <input type="radio" name="opt-startup" checked={opts.startup === v} onChange={() => setOpts({ ...opts, startup: v })} />
                <span>{t(v === "objects" ? "opt.startObjects" : "opt.startContinue")}</span>
              </label>
            ))}
            <div className="muted exp-note">{t("opt.startNote")}</div>
          </>
        );
      case "recovery":
        return (
          <>
            {check("autoSaveQueries", "opt.recoveryQuery")}
            {row("opt.recoverySeconds", num("autoSaveSeconds"), limit("autoSaveSeconds"))}
            <div className="muted exp-note">{t("opt.recoveryNote")}</div>
          </>
        );
      case "completion":
        return (
          <>
            {check("codeCompletion", "opt.useCompletion")}
            {check("autoSelectFirst", "opt.autoSelect", true, !opts.codeCompletion)}
          </>
        );
      case "editor":
        return (
          <>
            {check("showLineNumbers", "opt.lineNumbers")}
            {check("codeFolding", "opt.folding")}
            {check("braceHighlight", "opt.braces")}
            {check("syntaxHighlight", "opt.syntax")}
            {check("wordWrap", "opt.wordWrap")}
            {row("opt.tabWidth", num("tabWidth"), limit("tabWidth"))}
            {check("insertSpaces", "opt.insertSpaces")}
            <div className="opt-section">{t("opt.fontColors")}</div>
            {row("opt.editorFont", (
              <>
                <input className="field" list="opt-fonts" value={opts.editorFont} placeholder={t("opt.defaultFont")} spellCheck={false} onChange={(e) => setOpts({ ...opts, editorFont: e.target.value })} />
                <datalist id="opt-fonts">{["Cascadia Mono", "Consolas", "Courier New", "JetBrains Mono", "Fira Code", "Source Code Pro", "Lucida Console", "Menlo", "Monaco", "DejaVu Sans Mono"].map((f) => <option key={f} value={f} />)}</datalist>
              </>
            ))}
            {row("opt.fontSize", num("editorFontSize"), `px (${limit("editorFontSize")})`)}
            <div className="opt-colors">
              {EDITOR_COLOR_KEYS.map((k: EditorColorKey) => (
                <div key={k} className="opt-color">
                  <label htmlFor={`opt-color-${k}`}>{t(`opt.color.${k}` as MessageKey)}</label>
                  <input
                    id={`opt-color-${k}`}
                    type="color"
                    value={opts.editorColors[k] || defaultColor(k)}
                    onChange={(e) => setOpts({ ...opts, editorColors: { ...opts.editorColors, [k]: e.target.value } })}
                  />
                  <button type="button" className="btn" disabled={!opts.editorColors[k]} title={t("opt.colorReset")} onClick={() => setOpts({ ...opts, editorColors: { ...opts.editorColors, [k]: "" } })}>↺</button>
                </div>
              ))}
            </div>
            {row("opt.maxRows", num("queryMaxRows"), limit("queryMaxRows"))}
            <div className="muted exp-note">{t("opt.maxRowsNote")}</div>
          </>
        );
      case "data":
        return (
          <>
            {check("limitRecords", "opt.limitRecords")}
            {row("opt.pageSize", num("pageSize"), limit("pageSize"))}
            <div className="opt-section">{t("opt.displayFormat")}</div>
            {([["displayDate", "opt.fmtDate", "dd/MM/yyyy", "date"], ["displayTime", "opt.fmtTime", "HH:mm:ss", "time"], ["displayDateTime", "opt.fmtDateTime", "dd/MM/yyyy HH:mm:ss", "datetime"]] as const).map(([key, label, example, kind]) => (
              <div key={key} className="exp-row">
                <label>{t(label)}</label>
                <input className="field" value={opts[key]} placeholder={example} spellCheck={false} onChange={(e) => setOpts({ ...opts, [key]: e.target.value })} />
                <span className="muted">{opts[key].trim() ? formatTemporal(SAMPLE[kind], kind, opts[key]) : t("opt.asServer")}</span>
              </div>
            ))}
            <div className="muted exp-note">{t("opt.fmtNote")}</div>
            {check("showThousands", "opt.thousands")}
            {check("systemLocale", "opt.systemLocale")}
            <div className="muted exp-note">{t("opt.sampleNumber")}: {formatNumber("1234567.89", opts.showThousands, opts.systemLocale)}</div>
            {row("opt.rowShading", (
              <select className="field" value={opts.rowShading} onChange={(e) => setOpts({ ...opts, rowShading: e.target.value as RowShading })}>
                <option value="none">{t("opt.shadeNone")}</option>
                <option value="every2">{t("opt.shadeSecond")}</option>
                <option value="every3">{t("opt.shadeThird")}</option>
              </select>
            ))}
            <div className="muted exp-note">{t("opt.pageSizeNote")}</div>
          </>
        );
      case "history":
        return (
          <>
            <label className="check-row"><input type="checkbox" checked={recording} onChange={(e) => setRecording(e.target.checked)} /><span>{t("hist.recording")}</span></label>
            {row("opt.keep", (
              <input className="field exp-num" inputMode="numeric" value={keep} onChange={(e) => setKeep(e.target.value.replace(/\D/g, ""))} />
            ), t("opt.keepHint"))}
            <div className="muted exp-note">{t("hist.recordingHint")}</div>
            <div><button type="button" className="btn" onClick={() => setConfirmClear(true)}>{t("hist.clear")}</button></div>
            {note && <div className="muted">{note}</div>}
          </>
        );
      case "storage":
        return (
          <>
            {row("opt.dataFile", <input className="field opt-path" readOnly value={info?.path ?? ""} />)}
            {info?.schemaVersion != null && <div className="muted">{t("opt.schemaVersion", { n: info.schemaVersion })}</div>}
            {info?.error && <div className="doc-banner is-warn">{info.error}</div>}
            <div className="muted exp-note">{t("opt.storageNote")}</div>
            <div><button type="button" className="btn" disabled={!info?.path} onClick={() => { void navigator.clipboard?.writeText(info?.path ?? "").catch(() => undefined); }}>{t("opt.copyPath")}</button></div>
          </>
        );
    }
  };

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal opt-dialog" role="dialog" aria-label={t("opt.title")}>
        <div className="modal-header"><span className="modal-title">{t("opt.title")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="opt-layout">
          <div className="opt-nav" role="tablist" aria-orientation="vertical">
            {TABS.map((x) => (
              <button key={x.id} type="button" role="tab" aria-selected={tab === x.id} className={`opt-nav-item${tab === x.id ? " is-active" : ""}`} onClick={() => setTab(x.id)}>{t(x.label)}</button>
            ))}
          </div>
          <div className="modal-body opt-body">
            <div className="opt-section">{t(TABS.find((x) => x.id === tab)!.label)}</div>
            {body()}
          </div>
        </div>
        {error && <div className="doc-banner is-warn">{error}</div>}
        <div className="modal-footer">
          <button type="button" className="btn" onClick={restore}>{t("opt.restore")}</button>
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={() => void apply()}>OK</button>
          <button type="button" className="btn" onClick={onClose}>{t("opt.cancel")}</button>
        </div>
      </div>
      {confirmClear && (
        <MessageDialog
          title={t("hist.clear")}
          message={t("hist.clearMessage")}
          danger
          okLabel={t("hist.clear")}
          onOk={() => {
            setConfirmClear(false);
            void local.historyClear(true).then((n) => setNote(t("opt.cleared", { n })), (e) => setError(errorText(toAtlasError(e))));
          }}
          onCancel={() => setConfirmClear(false)}
        />
      )}
    </div>
  );
}
