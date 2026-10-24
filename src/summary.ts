/** Resumo de uma coluna no rodapé do grid (sobre as linhas carregadas). */

export type Aggregate = "none" | "count" | "distinct" | "sum" | "avg" | "min" | "max";

export const NUMERIC_AGGREGATES: Aggregate[] = ["sum", "avg", "min", "max"];

const DECIMAL = /^-?\d+(\.\d+)?$/;

/** Número decimal exato: inteiro escalado por 10^`scale`. */
interface Scaled {
  n: bigint;
  scale: number;
}

function parseDecimal(v: string, scale: number): bigint {
  const neg = v.startsWith("-");
  const [int, frac = ""] = (neg ? v.slice(1) : v).split(".");
  const n = BigInt(int + frac.padEnd(scale, "0"));
  return neg ? -n : n;
}

function toText({ n, scale }: Scaled, maxFraction = scale): string {
  const neg = n < 0n;
  let digits = (neg ? -n : n).toString().padStart(scale + 1, "0");
  const int = digits.slice(0, digits.length - scale);
  let frac = digits.slice(digits.length - scale);
  frac = frac.slice(0, maxFraction).replace(/0+$/, "");
  digits = frac ? `${int}.${frac}` : int;
  return neg && /[1-9]/.test(digits) ? `-${digits}` : digits;
}

/**
 * Valor do resumo (como texto, pronto para mostrar) ou `null` se não houver o que calcular. Soma, média, mínimo e máximo valem
 * só para números (decimal exato, sem erro de ponto flutuante); contagem e distintos valem para qualquer coluna e ignoram NULL.
 * Mínimo e máximo de texto e datas seguem a ordem do texto (datas ISO ordenam certo).
 */
export function aggregate(values: (string | null)[], agg: Aggregate, numeric: boolean): string | null {
  const present = values.filter((v): v is string => v !== null && v !== undefined);
  if (agg === "none") return null;
  if (agg === "count") return String(present.length);
  if (agg === "distinct") return String(new Set(present).size);
  if (!present.length) return null;
  if (agg === "min" || agg === "max") {
    if (numeric && present.every((v) => DECIMAL.test(v))) {
      const scale = Math.max(...present.map((v) => v.split(".")[1]?.length ?? 0));
      const nums = present.map((v) => parseDecimal(v, scale));
      const pick = nums.reduce((a, b) => (agg === "min" ? (b < a ? b : a) : b > a ? b : a));
      return toText({ n: pick, scale });
    }
    return present.reduce((a, b) => (agg === "min" ? (b < a ? b : a) : b > a ? b : a));
  }
  if (!numeric) return null;
  const nums = present.filter((v) => DECIMAL.test(v));
  if (!nums.length) return null;
  const scale = Math.max(...nums.map((v) => v.split(".")[1]?.length ?? 0));
  const sum = nums.reduce((a, v) => a + parseDecimal(v, scale), 0n);
  if (agg === "sum") return toText({ n: sum, scale });
  // Média: 4 casas a mais que a maior escala, truncada e sem zeros à direita.
  const extra = 4;
  const scaled = (sum * 10n ** BigInt(extra)) / BigInt(nums.length);
  return toText({ n: scaled, scale: scale + extra });
}
