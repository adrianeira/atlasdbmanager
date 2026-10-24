import type { ConnectionMock } from "./model";

/** Aba de tabela, view, view materializada ou consulta ligada a um banco, como fica gravada na sessão, guardada ao fechar o banco ou levada para uma janela. */
export interface SavedTable {
  connectionId: string;
  database: string;
  schema: string;
  object: string;
  kind: "table" | "view" | "matview" | "query";
  /** Consulta: o texto do editor. */
  sql?: string;
  title: string;
  /** Era a aba aberta ao fechar. */
  active?: boolean;
  /** Posição na barra de abas (0 é Objects). */
  pos?: number;
  /** Estado da tela guardado por pouco tempo (resultados, cursor, página...); lido (e apagado) quando a aba é reconstruída. */
  stateId?: string;
}

/** O que a janela de uma aba destacada recebe ao abrir (em `window.__ATLAS_TAB__`). */
export interface TabWindowParams {
  /** Identifica a aba: uma aba abre no máximo uma janela. */
  id: string;
  title: string;
  /** A conexão (sem a lista de bancos: a janela lê o que precisa do servidor, pela mesma sessão da janela principal). */
  conn: ConnectionMock;
  entry: SavedTable;
}

/** Eventos da janela da aba para a janela principal. */
export type TabEvent =
  | { type: "attach"; id: string; entry: SavedTable }
  | { type: "closed"; id: string }
  | { type: "update"; id: string; entry: SavedTable }
  /** A janela da aba está sendo arrastada: onde está o cursor (pixels da tela). */
  | { type: "probe"; id: string; x: number; y: number }
  /** Resposta da janela principal: o cursor está sobre a barra de abas. */
  | { type: "probe-hit"; id: string; hit: boolean };

export const TAB_EVENT = "tab-window";
