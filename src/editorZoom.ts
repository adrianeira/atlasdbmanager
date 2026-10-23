/** Zoom dos editores SQL (Ctrl + roda do mouse, Ctrl + / Ctrl - e Ctrl 0), compartilhado por todos e lembrado entre sessões. */

const KEY = "atlas.editor.zoom";
const MIN = 0.5;
const MAX = 4;
const STEP = 0.1;

const clamp = (z: number) => Math.min(MAX, Math.max(MIN, Math.round(z * 10) / 10));

function load(): number {
  try {
    const raw = Number(localStorage.getItem(KEY));
    return Number.isFinite(raw) && raw > 0 ? clamp(raw) : 1;
  } catch {
    return 1;
  }
}

let zoom = load();
const listeners = new Set<() => void>();

function apply() {
  document.documentElement.style.setProperty("--editor-zoom", String(zoom));
  listeners.forEach((fn) => fn());
}
apply();

export function setZoom(next: number) {
  const z = clamp(next);
  if (z === zoom) return;
  zoom = z;
  try {
    localStorage.setItem(KEY, String(zoom));
  } catch {
    // Sem armazenamento: o zoom só vale nesta sessão.
  }
  apply();
}

export const zoomIn = () => setZoom(zoom + STEP);
export const zoomOut = () => setZoom(zoom - STEP);
export const zoomReset = () => setZoom(1);

/** Chamado a cada mudança de zoom (o editor remede as linhas). Devolve a função que cancela. */
export function onZoom(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
