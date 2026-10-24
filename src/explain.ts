/** Plano de execução (`EXPLAIN (FORMAT JSON)`) em árvore, com o custo próprio de cada nó, a posição no desenho e as estatísticas. */

export interface PlanNode {
  id: number;
  /** "Hash Join", "Seq Scan"... */
  type: string;
  /** O que o desenho mostra embaixo do nó: a tabela (ou CTE) lida, senão o tipo do nó. */
  label: string;
  startupCost: number;
  totalCost: number;
  rows: number;
  width: number;
  /** Custo do nó sem o dos filhos (o que ele mesmo gasta). */
  ownCost: number;
  /** Parte do custo próprio no total do plano, 0–100. */
  ownPct: number;
  children: PlanNode[];
  /** Todos os campos do nó como vieram do servidor (menos `Plans`). */
  props: Record<string, unknown>;
}

const num = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) ? v : 0);

/** Filhos cujo custo já está incluído no do pai (InitPlan e SubPlan têm custo à parte). */
const inParent = (child: Record<string, unknown>): boolean => child["Parent Relationship"] !== "InitPlan" && child["Parent Relationship"] !== "SubPlan";

/** Lê a saída do `EXPLAIN (FORMAT JSON)`; `null` se não for um plano. */
export function parsePlan(json: string): PlanNode | null {
  let data: unknown;
  try {
    data = JSON.parse(json);
  } catch {
    return null;
  }
  const first = Array.isArray(data) ? data[0] : data;
  const root = first && typeof first === "object" ? (first as Record<string, unknown>).Plan : undefined;
  if (!root || typeof root !== "object") return null;

  let nextId = 0;
  const build = (raw: Record<string, unknown>): PlanNode => {
    const { Plans, ...props } = raw;
    const kids = Array.isArray(Plans) ? (Plans as Record<string, unknown>[]).filter((c) => c && typeof c === "object") : [];
    const type = typeof raw["Node Type"] === "string" ? raw["Node Type"] : "?";
    const rel = raw["Relation Name"] ?? raw["CTE Name"];
    const id = nextId++;
    const children = kids.map(build);
    const total = num(raw["Total Cost"]);
    const kidsCost = kids.reduce((sum, k) => sum + (inParent(k) ? num(k["Total Cost"]) : 0), 0);
    return {
      id, type, label: typeof rel === "string" ? rel : type,
      startupCost: num(raw["Startup Cost"]), totalCost: total, rows: num(raw["Plan Rows"]), width: num(raw["Plan Width"]),
      ownCost: Math.max(0, total - kidsCost), ownPct: 0, children, props,
    };
  };
  const tree = build(root as Record<string, unknown>);
  const all = flatten(tree);
  const sum = all.reduce((s, n) => s + n.ownCost, 0);
  for (const n of all) n.ownPct = sum > 0 ? (n.ownCost / sum) * 100 : 0;
  return tree;
}

/** Todos os nós, do topo para baixo. */
export function flatten(root: PlanNode): PlanNode[] {
  return [root, ...root.children.flatMap(flatten)];
}

export interface Placed {
  node: PlanNode;
  /** Coluna (profundidade) e linha, em unidades de grade; `row` pode ser fracionário (o pai fica entre os filhos). */
  col: number;
  row: number;
}

/** Raiz à esquerda, filhos à direita; cada folha ocupa uma linha e o pai fica na média das linhas dos filhos. */
export function layout(root: PlanNode): { placed: Placed[]; cols: number; rows: number } {
  const placed: Placed[] = [];
  let leaf = 0;
  let cols = 1;
  const go = (n: PlanNode, col: number): number => {
    cols = Math.max(cols, col + 1);
    const slot: Placed = { node: n, col, row: 0 };
    placed.push(slot);
    if (!n.children.length) {
      slot.row = leaf++;
    } else {
      const rows = n.children.map((c) => go(c, col + 1));
      slot.row = rows.reduce((a, b) => a + b, 0) / rows.length;
    }
    return slot.row;
  };
  go(root, 0);
  return { placed, cols, rows: Math.max(1, leaf) };
}

export interface TypeStat {
  type: string;
  count: number;
  cost: number;
  pct: number;
}

/** Estatísticas por tipo de nó, em ordem alfabética (custo = soma dos custos próprios). */
export function statsByType(root: PlanNode): TypeStat[] {
  const map = new Map<string, TypeStat>();
  for (const n of flatten(root)) {
    const s = map.get(n.type) ?? { type: n.type, count: 0, cost: 0, pct: 0 };
    s.count++;
    s.cost += n.ownCost;
    s.pct += n.ownPct;
    map.set(n.type, s);
  }
  return [...map.values()].sort((a, b) => a.type.localeCompare(b.type));
}

/** Texto de busca de um nó (tipo, tabela, alias, índice, condições). */
export function nodeText(n: PlanNode): string {
  return [n.type, n.label, ...Object.values(n.props).filter((v) => typeof v === "string")].join(" ").toLowerCase();
}

/** Resumo do nó para a dica ao passar o mouse. */
export function nodeTip(n: PlanNode): string {
  const lines = [n.type];
  for (const key of ["Relation Name", "Alias", "Index Name", "Join Type", "Hash Cond", "Index Cond", "Filter", "Join Filter", "Sort Key"]) {
    const v = n.props[key];
    if (v !== undefined) lines.push(`${key}: ${Array.isArray(v) ? v.join(", ") : String(v)}`);
  }
  lines.push(`Cost: ${n.startupCost}..${n.totalCost}  Rows: ${n.rows}  Width: ${n.width}`);
  return lines.join("\n");
}
