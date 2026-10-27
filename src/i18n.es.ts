import type { MessageKey } from "./i18n";
import { esPart1 } from "./i18n.es.part1";
import { esPart2 } from "./i18n.es.part2";
import { esPart3 } from "./i18n.es.part3";
import { esPart4 } from "./i18n.es.part4";
import { esPart5 } from "./i18n.es.part5";

/**
 * Interface em espanhol. As partes ficam em arquivos separados só para o dicionário não virar um arquivo gigante;
 * como cada parte é literal, o compilador acusa aqui qualquer texto novo do inglês que ficar sem tradução.
 */
export const es: Record<MessageKey, string> = { ...esPart1, ...esPart2, ...esPart3, ...esPart4, ...esPart5 };
