import { useEffect, useState } from "react";
import { errorText, local, toAtlasError } from "../backend";
import { useT, type Lang, type MessageKey } from "../i18n";
import { DEFAULT_OPTIONS, OPTION_LIMITS, sanitizeOptions, type AppOptions } from "../options";
import { MessageDialog } from "./MessageDialog";

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

type Tab = "general" | "editor" | "data" | "history" | "storage";
const TABS: { id: Tab; label: MessageKey }[] = [
  { id: "general", label: "opt.general" },
  { id: "editor", label: "opt.editor" },
  { id: "data", label: "opt.data" },
  { id: "history", label: "opt.history" },
  { id: "storage", label: "opt.storage" },
];

export function OptionsDialog({ options, appearance, onApply, onClose }: Props) {
  const t = useT();
  const [tab, setTab] = useState<Tab>("general");
  const [opts, setOpts] = useState<AppOptions>(options);
  const [look, setLook] = useState<AppearanceOptions>(appearance);
  // Os campos numéricos são texto enquanto se digita; viram número válido ao sair do campo.
  const [text, setText] = useState({ editorFontSize: String(options.editorFontSize), queryMaxRows: String(options.queryMaxRows), pageSize: String(options.pageSize) });
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

  const num = (key: keyof AppOptions) => (
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
  const limit = (key: keyof AppOptions) => `${OPTION_LIMITS[key][0].toLocaleString()} – ${OPTION_LIMITS[key][1].toLocaleString()}`;
  const row = (label: MessageKey, control: React.ReactNode, hint?: string) => (
    <div className="exp-row">
      <label>{t(label)}</label>
      {control}
      {hint && <span className="muted">{hint}</span>}
    </div>
  );

  const apply = async () => {
    setError(null);
    const finalOptions = sanitizeOptions(text);
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
    setText({ editorFontSize: String(DEFAULT_OPTIONS.editorFontSize), queryMaxRows: String(DEFAULT_OPTIONS.queryMaxRows), pageSize: String(DEFAULT_OPTIONS.pageSize) });
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
          </>
        );
      case "editor":
        return (
          <>
            {row("opt.fontSize", num("editorFontSize"), `px (${limit("editorFontSize")})`)}
            {row("opt.maxRows", num("queryMaxRows"), limit("queryMaxRows"))}
            <div className="muted exp-note">{t("opt.maxRowsNote")}</div>
          </>
        );
      case "data":
        return (
          <>
            {row("opt.pageSize", num("pageSize"), limit("pageSize"))}
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
        <div className="modal-header">{t("opt.title")}</div>
        <div className="dlg-tabs" role="tablist">
          {TABS.map((x) => (
            <button key={x.id} type="button" role="tab" aria-selected={tab === x.id} className={`dlg-tab${tab === x.id ? " is-active" : ""}`} onClick={() => setTab(x.id)}>{t(x.label)}</button>
          ))}
        </div>
        <div className="modal-body opt-body">{body()}</div>
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
