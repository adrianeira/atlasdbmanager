/** Diferença por linha entre dois textos (maior subsequência comum). Devolve os números (a partir de 1) das linhas que só existem de cada lado. */
export function diffLines(left: string, right: string): { left: number[]; right: number[] } {
  const a = left.split("\n");
  const b = right.split("\n");
  if (a.length * b.length > 4_000_000) return { left: [], right: [] };
  // Tabela de comprimentos: lcs[i][j] = tamanho da subsequência comum de a[i..] e b[j..].
  const lcs: Uint16Array[] = Array.from({ length: a.length + 1 }, () => new Uint16Array(b.length + 1));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const onlyLeft: number[] = [];
  const onlyRight: number[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) { i++; j++; }
    else if (lcs[i + 1][j] >= lcs[i][j + 1]) onlyLeft.push(++i);
    else onlyRight.push(++j);
  }
  while (i < a.length) onlyLeft.push(++i);
  while (j < b.length) onlyRight.push(++j);
  return { left: onlyLeft, right: onlyRight };
}
