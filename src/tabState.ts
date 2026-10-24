/** O que cada tela guarda de si para ser reconstruída igual depois de destacar, anexar ou fechar o banco. */
import type { MutableRefObject } from "react";

export interface QuerySnapshot {
  kind: "query";
  /** Cursor (início e fim da seleção) e rolagem do editor. */
  view?: { anchor: number; head: number; scrollTop: number };
  /** Resultado da última execução (resultados, mensagens, Summary e Explain), a aba aberta e os resultados fixados. */
  outcome?: unknown;
  resultTab?: string;
  pins?: unknown[];
  split?: number;
  params?: Record<string, unknown>;
  activeRow?: number;
}

export interface DataSnapshot {
  kind: "data";
  page: number;
  activeRow: number;
  scrollTop: number;
  view: "grid" | "form";
  findOpen: boolean;
  findQuery: string;
}

export type TabSnapshot = QuerySnapshot | DataSnapshot;

/** A tela escreve aqui, a cada renderização, uma função que devolve o estado atual dela. */
export type SnapshotRef = MutableRefObject<(() => TabSnapshot) | null>;
