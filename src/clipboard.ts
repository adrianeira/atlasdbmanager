import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { isDesktop } from "./backend";

/*
 * Área de transferência. No app, pelo plugin oficial do Tauri (área de transferência do sistema,
 * sem o pedido de permissão do WebView — o `navigator.clipboard.readText` do WebView2 fica pendente).
 * No navegador (modo de demonstração), pela API web.
 */

export async function copyToClipboard(text: string): Promise<void> {
  if (isDesktop) return writeText(text);
  await navigator.clipboard.writeText(text);
}

export async function readClipboard(): Promise<string> {
  if (isDesktop) return readText();
  return navigator.clipboard.readText();
}
