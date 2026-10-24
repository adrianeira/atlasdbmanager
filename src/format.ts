/** Formatos de exibição do grid (Opções → Table Data → Display Format). Só mudam o que se vê; o valor e a cópia seguem os originais. */

export type TemporalKind = "date" | "time" | "datetime";

const TEMPORAL = /^(\d{4,})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?)?(?:[+-]\d{2}(?::?\d{2})?|Z)?$/;
const TIME_ONLY = /^(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?(?:[+-]\d{2}(?::?\d{2})?|Z)?$/;

interface Parts {
  y: string;
  mo: string;
  d: string;
  h: string;
  mi: string;
  s: string;
  frac: string;
}

function parse(v: string, kind: TemporalKind): Parts | null {
  if (kind === "time") {
    const m = TIME_ONLY.exec(v);
    return m ? { y: "", mo: "", d: "", h: m[1], mi: m[2], s: m[3] ?? "00", frac: m[4] ?? "" } : null;
  }
  const m = TEMPORAL.exec(v);
  if (!m) return null;
  if (kind === "date" && m[4] !== undefined) return null;
  return { y: m[1], mo: m[2], d: m[3], h: m[4] ?? "00", mi: m[5] ?? "00", s: m[6] ?? "00", frac: m[7] ?? "" };
}

/**
 * Aplica o padrão (`yyyy`, `yy`, `MM`, `dd`, `HH`, `hh` + `tt`, `mm`, `ss`, `f`... para a fração) ao valor. Padrão vazio ou valor que
 * não é uma data reconhecida (`infinity`, anos antes de Cristo...) volta como veio.
 */
export function formatTemporal(v: string, kind: TemporalKind, pattern: string): string {
  const p = pattern.trim();
  if (!p) return v;
  const parts = parse(v, kind);
  if (!parts) return v;
  const hour = Number(parts.h);
  const h12 = String(hour % 12 === 0 ? 12 : hour % 12).padStart(2, "0");
  return p.replace(/yyyy|yy|MM|dd|HH|hh|mm|ss|tt|f{1,6}/g, (tok) => {
    switch (tok) {
      case "yyyy": return parts.y;
      case "yy": return parts.y.slice(-2);
      case "MM": return parts.mo;
      case "dd": return parts.d;
      case "HH": return parts.h;
      case "hh": return h12;
      case "mm": return parts.mi;
      case "ss": return parts.s;
      case "tt": return hour < 12 ? "AM" : "PM";
      default: return parts.frac.padEnd(tok.length, "0").slice(0, tok.length);
    }
  });
}

/** Separadores do sistema (via `Intl`) ou os fixos `,` e `.`. */
export function separators(systemLocale: boolean): { group: string; decimal: string } {
  if (!systemLocale) return { group: ",", decimal: "." };
  const parts = new Intl.NumberFormat(undefined).formatToParts(1234567.5);
  return {
    group: parts.find((x) => x.type === "group")?.value ?? ",",
    decimal: parts.find((x) => x.type === "decimal")?.value ?? ".",
  };
}

/** Separador de milhar e de decimal em números simples (`-1234.50`); outro texto volta como veio. */
export function formatNumber(v: string, thousands: boolean, systemLocale: boolean): string {
  const m = /^(-?)(\d+)(?:\.(\d+))?$/.exec(v);
  if (!m || (!thousands && !systemLocale)) return v;
  const { group, decimal } = separators(systemLocale);
  const int = thousands ? m[2].replace(/\B(?=(\d{3})+(?!\d))/g, group) : m[2];
  return `${m[1]}${int}${m[3] !== undefined ? decimal + m[3] : ""}`;
}
