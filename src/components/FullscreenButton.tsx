import { createContext, useContext } from "react";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";

/** Tela cheia do documento da aba: ele cobre a janela do app (o App decide e guarda quem está em tela cheia). */
export interface DocFullscreen {
  on: boolean;
  toggle: () => void;
}

export const DocFullscreenContext = createContext<DocFullscreen>({ on: false, toggle: () => {} });

/** O botão do canto da barra de ferramentas dos documentos. */
export function FullscreenButton() {
  const t = useT();
  const { on, toggle } = useContext(DocFullscreenContext);
  const label = t(on ? "er.exitFullscreen" : "er.fullscreen");
  return (
    <button type="button" className="icon-button" title={label} aria-label={label} onClick={toggle}>
      <SmallIcon name="fullscreen" />
    </button>
  );
}
