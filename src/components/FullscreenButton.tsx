import { createContext, useContext } from "react";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";

/** Tela cheia do documento da aba: ele cobre a janela do app (o App decide e guarda quem está em tela cheia). */
export interface DocFullscreen {
  on: boolean;
  toggle: () => void;
}

export const DocFullscreenContext = createContext<DocFullscreen>({ on: false, toggle: () => {} });

/** Numa janela de aba destacada: o que o botão do canto faz (devolver a aba à janela principal), no lugar da tela cheia, que a própria janela já tem. */
export const DocAttachContext = createContext<(() => void) | null>(null);

/** O botão do canto da barra de ferramentas dos documentos. */
export function FullscreenButton() {
  const t = useT();
  const { on, toggle } = useContext(DocFullscreenContext);
  const attach = useContext(DocAttachContext);
  if (attach) {
    return (
      <button type="button" className="icon-button" title={t("tab.attachHint")} aria-label={t("tab.attach")} onClick={attach}>
        <SmallIcon name="attach" />
      </button>
    );
  }
  const label = t(on ? "er.exitFullscreen" : "er.fullscreen");
  return (
    <button type="button" className="icon-button" title={label} aria-label={label} onClick={toggle}>
      <SmallIcon name="fullscreen" />
    </button>
  );
}
