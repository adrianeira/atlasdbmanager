/** Conteúdo binário de uma célula (`bytea` chega como texto `\x...` em hexadecimal): bytes, imagem e vista em hexa. */

export const isHexBlob = (v: string): boolean => /^\\x([0-9a-fA-F]{2})*$/.test(v);

export function hexToBytes(v: string): Uint8Array {
  const hex = v.startsWith("\\x") ? v.slice(2) : v;
  const out = new Uint8Array(Math.floor(hex.length / 2));
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export function bytesToHex(b: Uint8Array): string {
  let s = "\\x";
  for (const x of b) s += x.toString(16).padStart(2, "0");
  return s;
}

export function textToBytes(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

/** Texto UTF-8 dos bytes; bytes que não formam texto válido viram `�`. */
export function bytesToText(b: Uint8Array): string {
  return new TextDecoder("utf-8", { fatal: false }).decode(b);
}

export function toBase64(b: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < b.length; i += 0x8000) bin += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(bin);
}

const starts = (b: Uint8Array, sig: number[], at = 0) => sig.every((x, i) => b[at + i] === x);

/** Tipo de imagem pelos primeiros bytes (PNG, JPEG, GIF, BMP, WEBP, ICO) ou SVG em texto; `null` se não for imagem conhecida. */
export function imageMime(b: Uint8Array): string | null {
  if (starts(b, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return "image/png";
  if (starts(b, [0xff, 0xd8, 0xff])) return "image/jpeg";
  if (starts(b, [0x47, 0x49, 0x46, 0x38])) return "image/gif";
  if (starts(b, [0x42, 0x4d]) && b.length > 26) return "image/bmp";
  if (starts(b, [0x52, 0x49, 0x46, 0x46]) && starts(b, [0x57, 0x45, 0x42, 0x50], 8)) return "image/webp";
  if (starts(b, [0x00, 0x00, 0x01, 0x00]) && b.length > 22) return "image/x-icon";
  const head = bytesToText(b.subarray(0, 512)).trimStart().toLowerCase();
  if (head.startsWith("<svg") || (head.startsWith("<?xml") && head.includes("<svg"))) return "image/svg+xml";
  return null;
}

/** Vista em hexa no formato `00000000  48 65 6c 6c 6f ...  |Hello|` (16 bytes por linha); `limit` corta a vista longa. */
export function hexDump(b: Uint8Array, limit = 64 * 1024): string {
  const n = Math.min(b.length, limit);
  const lines: string[] = [];
  for (let off = 0; off < n; off += 16) {
    const row = b.subarray(off, Math.min(off + 16, n));
    const hex = [...row].map((x) => x.toString(16).padStart(2, "0")).join(" ");
    const ascii = [...row].map((x) => (x >= 0x20 && x < 0x7f ? String.fromCharCode(x) : ".")).join("");
    lines.push(`${off.toString(16).padStart(8, "0")}  ${hex.padEnd(47, " ")}  |${ascii}|`);
  }
  return lines.join("\n");
}

/** Bytes da célula: `bytea` em hexa, ou o texto em UTF-8. */
export function cellBytes(v: string, binary: boolean): Uint8Array {
  return binary && isHexBlob(v) ? hexToBytes(v) : textToBytes(v);
}
