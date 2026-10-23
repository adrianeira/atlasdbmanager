import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { local } from "../backend";
import { I18nContext, detectLang, makeTranslator, type Lang } from "../i18n";
import { DEFAULT_OPTIONS, OptionsContext, sanitizeOptions, type AppOptions } from "../options";
import "../editorZoom";

/** Idioma e tema escolhidos na janela principal (guardados no mesmo armazenamento local). */
function readLayout(): { lang: Lang; theme: string } {
  try {
    const raw = JSON.parse(localStorage.getItem("atlas.layout.v1") ?? "null") as { lang?: Lang; theme?: string } | null;
    return { lang: raw?.lang ?? detectLang(), theme: raw?.theme ?? "system" };
  } catch {
    return { lang: detectLang(), theme: "system" };
  }
}

/** Casco das janelas do sistema à parte (Query Builder, Data Transfer): idioma, tema, fonte do editor e título. */
export function ToolWindow({ title, children }: { title: string; children: ReactNode }) {
  const { lang, theme } = useMemo(readLayout, []);
  const t = useMemo(() => makeTranslator(lang), [lang]);
  const [options, setOptions] = useState<AppOptions>(DEFAULT_OPTIONS);

  useEffect(() => {
    const root = document.documentElement;
    if (theme !== "system") root.dataset.theme = theme;
    root.lang = lang;
    document.title = title;
    const effective = theme === "system" ? (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : theme;
    void getCurrentWindow().setTheme(effective as "dark" | "light").catch(() => undefined);
    void local.getPref<Partial<AppOptions>>("ui.options").then((v) => setOptions(sanitizeOptions(v))).catch(() => undefined);
  }, [lang, theme, title]);

  useEffect(() => {
    document.documentElement.style.setProperty("--editor-font-size", `${options.editorFontSize}px`);
  }, [options.editorFontSize]);

  return (
    <I18nContext.Provider value={t}>
      <OptionsContext.Provider value={options}>{children}</OptionsContext.Provider>
    </I18nContext.Provider>
  );
}
