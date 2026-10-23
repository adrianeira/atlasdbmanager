import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useMemo } from "react";
import { local } from "../../backend";
import { makeTranslator, I18nContext, detectLang, type Lang } from "../../i18n";
import { DEFAULT_OPTIONS, OptionsContext, sanitizeOptions, type AppOptions } from "../../options";
import { useState } from "react";
import "../../editorZoom";
import { QueryBuilderDialog } from "./QueryBuilderDialog";

interface Params {
  id: string;
  connId: string;
  database: string;
  defaultSchema: string;
  initialSql: string;
  title: string;
}

/** Idioma e tema escolhidos na janela principal (guardados no mesmo armazenamento local). */
function readLayout(): { lang: Lang; theme: string } {
  try {
    const raw = JSON.parse(localStorage.getItem("atlas.layout.v1") ?? "null") as { lang?: Lang; theme?: string } | null;
    return { lang: raw?.lang ?? detectLang(), theme: raw?.theme ?? "system" };
  } catch {
    return { lang: detectLang(), theme: "system" };
  }
}

/** Conteúdo da janela separada do Query Builder: o mesmo componente, ocupando a janela inteira. */
export function QueryBuilderWindow() {
  const params = (window as unknown as { __ATLAS_QB__: Params }).__ATLAS_QB__;
  const { lang, theme } = useMemo(readLayout, []);
  const t = useMemo(() => makeTranslator(lang), [lang]);
  const [options, setOptions] = useState<AppOptions>(DEFAULT_OPTIONS);

  useEffect(() => {
    const root = document.documentElement;
    if (theme !== "system") root.dataset.theme = theme;
    root.lang = lang;
    document.title = params.title;
    const effective = theme === "system" ? (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : theme;
    void getCurrentWindow().setTheme(effective as "dark" | "light").catch(() => undefined);
    void local.getPref<Partial<AppOptions>>("ui.options").then((v) => setOptions(sanitizeOptions(v))).catch(() => undefined);
  }, [lang, theme, params.title]);

  useEffect(() => {
    document.documentElement.style.setProperty("--editor-font-size", `${options.editorFontSize}px`);
  }, [options.editorFontSize]);

  const close = () => void getCurrentWindow().close();

  return (
    <I18nContext.Provider value={t}>
      <OptionsContext.Provider value={options}>
        <QueryBuilderDialog
          connId={params.connId}
          database={params.database}
          defaultSchema={params.defaultSchema}
          initialSql={params.initialSql}
          onClose={close}
          onBuild={(sql, run, replaced) => {
            void emit("qb-build", { id: params.id, sql, run, replaced }).finally(close);
          }}
        />
      </OptionsContext.Provider>
    </I18nContext.Provider>
  );
}
