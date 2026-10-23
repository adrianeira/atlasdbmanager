/*
 * Diagrama ER da área de objetos (Exibir → ER Diagram): modelo, layout automático e geometria dos conectores.
 * Tudo aqui é puro (sem DOM), para o componente só desenhar.
 */
import type { SchemaDiagram } from "./backend";
import type { ColumnDef, DbObject } from "./model";
import { columnType } from "./sql/ddl";

export const ROW_H = 21;
export const HEAD_H = 28;
export const FOOT_PAD = 8;
export const MIN_W = 180;
export const MAX_W = 380;
/** Colunas mostradas por cartão; as demais viram a linha "… +N". */
export const MAX_ROWS = 40;
export const GRID = 20;

export interface ErColumn { name: string; type: string; isPk: boolean; isFk: boolean }
export interface ErTable { name: string; columns: ErColumn[]; hidden: number; w: number; h: number }
/** `from` é a tabela que tem a chave estrangeira (a que referencia); `to`, a referenciada. As colunas seguem a ordem da chave. */
export interface ErRelation { id: string; name: string; from: string; to: string; fromColumns: string[]; toColumns: string[]; required: boolean }
export interface ErModel { tables: ErTable[]; relations: ErRelation[] }
export interface Pos { x: number; y: number }
export interface Rect { x: number; y: number; w: number; h: number }

/** Modelo do diagrama: só as relações entre tabelas do próprio schema (as para outro schema não têm cartão). */
export function buildModel(d: SchemaDiagram, schema: string): ErModel {
  const tables: ErTable[] = d.tables.map((t) => {
    const columns = t.columns.slice(0, MAX_ROWS).map((c) => ({
      name: c.name,
      type: columnType({ name: c.name, type: c.dataType, length: c.length, decimal: c.decimal, notNull: false } as ColumnDef),
      isPk: c.isPk,
      isFk: c.isFk,
    }));
    const hidden = Math.max(0, t.columns.length - columns.length);
    const longest = columns.reduce((m, c) => Math.max(m, c.name.length + c.type.length + 3), hidden ? 12 : 0);
    const w = Math.min(MAX_W, Math.max(MIN_W, Math.ceil(Math.max(t.name.length * 8.5 + 28, longest * 7 + 44))));
    return { name: t.name, columns, hidden, w, h: HEAD_H + (columns.length + (hidden ? 1 : 0)) * ROW_H + FOOT_PAD };
  });
  const known = new Set(tables.map((t) => t.name));
  const relations = d.relations
    .filter((r) => r.refSchema === schema && known.has(r.table) && known.has(r.refTable))
    .map((r) => ({ id: `${r.table}.${r.name}`, name: r.name, from: r.table, to: r.refTable, fromColumns: r.columns, toColumns: r.refColumns, required: r.required }));
  return { tables, relations };
}

/** Diagrama montado dos objetos já carregados (modo de demonstração, sem servidor). */
export function diagramFromObjects(tables: DbObject[], schema: string): SchemaDiagram {
  const split = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);
  return {
    tables: tables.map((t) => {
      const fk = new Set((t.foreignKeys ?? []).flatMap((f) => split(f.fields)));
      return {
        name: t.name,
        columns: (t.columns ?? []).map((c) => ({
          name: c.name, dataType: c.type, length: c.length, decimal: c.decimal, isPk: !!c.key, isFk: fk.has(c.name),
        })),
      };
    }),
    relations: tables.flatMap((t) =>
      (t.foreignKeys ?? []).map((f) => ({
        name: f.name, table: t.name, columns: split(f.fields), refSchema: f.refSchema || schema, refTable: f.refTable, refColumns: split(f.refFields),
        required: split(f.fields).every((c) => !!(t.columns ?? []).find((x) => x.name === c)?.notNull),
      })),
    ),
  };
}

/**
 * Posição inicial dos cartões: as tabelas ligadas ficam vizinhas (percurso em largura a partir da mais ligada) e
 * entram em linhas de um grid quase quadrado, sem se sobrepor.
 */
export function autoLayout(model: ErModel): Record<string, Pos> {
  const { tables, relations } = model;
  const adj = new Map<string, Set<string>>(tables.map((t) => [t.name, new Set<string>()]));
  for (const r of relations) {
    if (r.from === r.to) continue;
    adj.get(r.from)?.add(r.to);
    adj.get(r.to)?.add(r.from);
  }
  const degree = (n: string) => adj.get(n)?.size ?? 0;
  const byDegree = (a: string, b: string) => degree(b) - degree(a) || a.localeCompare(b);

  const order: string[] = [];
  const seen = new Set<string>();
  for (const start of tables.map((t) => t.name).sort(byDegree)) {
    if (seen.has(start)) continue;
    seen.add(start);
    const queue = [start];
    while (queue.length) {
      const cur = queue.shift()!;
      order.push(cur);
      for (const next of [...(adj.get(cur) ?? [])].sort(byDegree)) {
        if (!seen.has(next)) {
          seen.add(next);
          queue.push(next);
        }
      }
    }
  }

  const size = new Map(tables.map((t) => [t.name, t]));
  const perRow = Math.max(1, Math.ceil(Math.sqrt(tables.length)));
  const GAP_X = 150;
  const GAP_Y = 70;
  const out: Record<string, Pos> = {};
  let x = 40;
  let y = 40;
  let rowH = 0;
  order.forEach((name, i) => {
    const t = size.get(name)!;
    if (i > 0 && i % perRow === 0) {
      x = 40;
      y += rowH + GAP_Y;
      rowH = 0;
    }
    out[name] = { x, y };
    x += t.w + GAP_X;
    rowH = Math.max(rowH, t.h);
  });
  return out;
}

/** Onde o nome da FK é escrito: `y` é a linha de base do texto e `anchor` diz para que lado ele cresce a partir de `x`. */
export interface Label { x: number; y: number; anchor: "start" | "end" }
/** `points` vai da tabela que tem a chave estrangeira (a que referencia) até a referenciada. */
/**
 * A alça para reposicionar a linha: `axis` é a coordenada que o usuário muda (`x` para o trecho vertical de um cotovelo
 * horizontal ou para uma linha reta vertical; `y` no caso contrário), `value` o valor atual, `min`/`max` o intervalo
 * permitido e `at` onde desenhar a alça.
 */
export interface LineHandle { axis: "x" | "y"; value: number; min: number; max: number; at: [number, number] }
export interface Connector { points: [number, number][]; label: Label; handle?: LineHandle }

export type Side = "left" | "right" | "top" | "bottom";

const GAP = 24;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Por quais bordas as duas caixas se ligam; `null` quando uma está sobre a outra nos dois eixos. */
export function sidesOf(a: Rect, b: Rect): { a: Side; b: Side } | null {
  if (a.x + a.w + GAP <= b.x) return { a: "right", b: "left" };
  if (b.x + b.w + GAP <= a.x) return { a: "left", b: "right" };
  if (a.y + a.h + GAP <= b.y) return { a: "bottom", b: "top" };
  if (b.y + b.h + GAP <= a.y) return { a: "top", b: "bottom" };
  return null;
}

/** Afastamento do ponto de saída na borda: `slot` de `count` ligações que usam o mesmo lado do cartão. */
export const slotOffset = (slot: number, count: number): number => (slot - (count - 1) / 2) * 14;

/**
 * O nome fica ao lado da linha, perto da tabela que tem a chave (como na referência): em cima do primeiro trecho quando
 * ele é horizontal, à esquerda quando é vertical. Assim não cobre a linha.
 */
function childLabel(points: [number, number][]): Label {
  const [p0, p1] = points;
  if (p1[1] === p0[1] && p1[0] !== p0[0]) {
    const dir = Math.sign(p1[0] - p0[0]);
    return { x: p0[0] + dir * 8, y: p0[1] - 6, anchor: dir > 0 ? "start" : "end" };
  }
  const dir = Math.sign(p1[1] - p0[1]) || 1;
  return { x: p0[0] - 8, y: p0[1] + dir * 15 + 4, anchor: "end" };
}

/**
 * Percurso entre as duas caixas, do lado esquerdo/superior para o direito/inferior; `aFirst` diz se `a` é o primeiro.
 * `adjust` é a coordenada escolhida pelo usuário para o trecho do meio (ver `LineHandle`).
 */
function route(a: Rect, b: Rect, offA: number, offB: number, adjust?: number): { points: [number, number][]; aFirst: boolean; handle?: LineHandle } {
  const sides = sidesOf(a, b);
  if (!sides) {
    // Cartões sobrepostos nos dois eixos (um arrastado sobre o outro): linha reta entre os centros.
    return { points: [[a.x + a.w / 2, a.y + a.h / 2], [b.x + b.w / 2, b.y + b.h / 2]], aFirst: true };
  }
  if (sides.a === "right" || sides.a === "left") {
    const aFirst = sides.a === "right";
    const [l, r, ol, or] = aFirst ? [a, b, offA, offB] : [b, a, offB, offA];
    const x1 = l.x + l.w;
    const x2 = r.x;
    const lo = Math.max(l.y, r.y);
    const hi = Math.min(l.y + l.h, r.y + r.h);
    if (lo + 12 < hi) {
      const y = clamp(adjust ?? (lo + hi) / 2 + offA, lo + 6, hi - 6);
      return { points: [[x1, y], [x2, y]], aFirst, handle: { axis: "y", value: y, min: lo + 6, max: hi - 6, at: [(x1 + x2) / 2, y] } };
    }
    const mx = clamp(adjust ?? (x1 + x2) / 2, x1 + 8, x2 - 8);
    const y1 = l.y + l.h / 2 + ol;
    const y2 = r.y + r.h / 2 + or;
    return { points: [[x1, y1], [mx, y1], [mx, y2], [x2, y2]], aFirst, handle: { axis: "x", value: mx, min: x1 + 8, max: x2 - 8, at: [mx, (y1 + y2) / 2] } };
  }
  const aFirst = sides.a === "bottom";
  const [u, d, ou, od] = aFirst ? [a, b, offA, offB] : [b, a, offB, offA];
  const y1 = u.y + u.h;
  const y2 = d.y;
  const lo = Math.max(u.x, d.x);
  const hi = Math.min(u.x + u.w, d.x + d.w);
  if (lo + 12 < hi) {
    const x = clamp(adjust ?? (lo + hi) / 2 + offA, lo + 6, hi - 6);
    return { points: [[x, y1], [x, y2]], aFirst, handle: { axis: "x", value: x, min: lo + 6, max: hi - 6, at: [x, (y1 + y2) / 2] } };
  }
  const my = clamp(adjust ?? (y1 + y2) / 2, y1 + 8, y2 - 8);
  const x1 = u.x + u.w / 2 + ou;
  const x2 = d.x + d.w / 2 + od;
  return { points: [[x1, y1], [x1, my], [x2, my], [x2, y2]], aFirst, handle: { axis: "y", value: my, min: y1 + 8, max: y2 - 8, at: [(x1 + x2) / 2, my] } };
}

/** Lados escolhidos pelo usuário para as pontas da linha (`a` = tabela que tem a chave, `b` = a referenciada). */
export interface EndSides { a?: Side; b?: Side }

const DIR: Record<Side, [number, number]> = { left: [-1, 0], right: [1, 0], top: [0, -1], bottom: [0, 1] };
export const SIDES: Side[] = ["top", "right", "bottom", "left"];

/** Lados que se olham, pelo eixo em que os centros mais se afastam (para a ponta que o usuário não escolheu). */
function facingSides(a: Rect, b: Rect): { a: Side; b: Side } {
  const dx = b.x + b.w / 2 - (a.x + a.w / 2);
  const dy = b.y + b.h / 2 - (a.y + a.h / 2);
  if (Math.abs(dx) >= Math.abs(dy)) return dx >= 0 ? { a: "right", b: "left" } : { a: "left", b: "right" };
  return dy >= 0 ? { a: "bottom", b: "top" } : { a: "top", b: "bottom" };
}

/** Lados usados por uma ligação: os escolhidos pelo usuário e, onde não houver escolha, os automáticos. */
export function resolveSides(a: Rect, b: Rect, forced?: EndSides): { a: Side; b: Side } | null {
  const auto = sidesOf(a, b);
  if (!forced || (!forced.a && !forced.b)) return auto;
  const base = auto ?? facingSides(a, b);
  return { a: forced.a ?? base.a, b: forced.b ?? base.b };
}

/** Ponto de ligação em um lado do cartão: o meio da borda, afastado `off` ao longo dela (sem sair do cartão). */
export function anchorPoint(r: Rect, side: Side, off = 0): [number, number] {
  const lim = (len: number) => Math.max(0, len / 2 - 8);
  if (side === "left" || side === "right") {
    return [side === "left" ? r.x : r.x + r.w, r.y + r.h / 2 + clamp(off, -lim(r.h), lim(r.h))];
  }
  return [r.x + r.w / 2 + clamp(off, -lim(r.w), lim(r.w)), side === "top" ? r.y : r.y + r.h];
}

/** Tira pontos repetidos e os do meio de um trecho reto. */
function simplify(points: [number, number][]): [number, number][] {
  const out: [number, number][] = [];
  for (const p of points) {
    const last = out[out.length - 1];
    if (last && last[0] === p[0] && last[1] === p[1]) continue;
    out.push(p);
  }
  return out.filter((p, i) => {
    if (i === 0 || i === out.length - 1) return true;
    const [q, r] = [out[i - 1], out[i + 1]];
    return !((q[0] === p[0] && p[0] === r[0]) || (q[1] === p[1] && p[1] === r[1]));
  });
}

/**
 * Percurso quando o usuário escolheu o lado de uma ou das duas pontas: sai do ponto de ligação por um trecho curto na
 * direção do lado e liga os dois trechos com no máximo dois cantos (`adjust` posiciona o trecho do meio).
 */
function routeForced(a: Rect, b: Rect, sa: Side, sb: Side, offA: number, offB: number, adjust?: number): { points: [number, number][]; handle?: LineHandle } {
  const STUB = 20;
  const pa = anchorPoint(a, sa, offA);
  const pb = anchorPoint(b, sb, offB);
  const da = DIR[sa];
  const db = DIR[sb];
  const ea: [number, number] = [pa[0] + da[0] * STUB, pa[1] + da[1] * STUB];
  const eb: [number, number] = [pb[0] + db[0] * STUB, pb[1] + db[1] * STUB];
  const horA = da[1] === 0;
  const horB = db[1] === 0;
  let mid: [number, number][] = [];
  let handle: LineHandle | undefined;
  if (horA && horB) {
    if (ea[1] !== eb[1]) {
      const lo = Math.min(ea[0], eb[0]) - 80;
      const hi = Math.max(ea[0], eb[0]) + 80;
      // Duas pontas do mesmo lado: a linha contorna os cartões pelo lado de fora.
      const fallback = sa === sb ? (sa === "right" ? Math.max(ea[0], eb[0]) + 24 : Math.min(ea[0], eb[0]) - 24) : (ea[0] + eb[0]) / 2;
      const mx = clamp(adjust ?? fallback, lo, hi);
      mid = [[mx, ea[1]], [mx, eb[1]]];
      handle = { axis: "x", value: mx, min: lo, max: hi, at: [mx, (ea[1] + eb[1]) / 2] };
    }
  } else if (!horA && !horB) {
    if (ea[0] !== eb[0]) {
      const lo = Math.min(ea[1], eb[1]) - 80;
      const hi = Math.max(ea[1], eb[1]) + 80;
      const fallback = sa === sb ? (sa === "bottom" ? Math.max(ea[1], eb[1]) + 24 : Math.min(ea[1], eb[1]) - 24) : (ea[1] + eb[1]) / 2;
      const my = clamp(adjust ?? fallback, lo, hi);
      mid = [[ea[0], my], [eb[0], my]];
      handle = { axis: "y", value: my, min: lo, max: hi, at: [(ea[0] + eb[0]) / 2, my] };
    }
  } else if (horA) {
    // Sai na horizontal e entra na vertical. O canto "horizontal primeiro" só serve se o primeiro trecho continua no
    // sentido da saída e o último anda no sentido da entrada (ex.: para baixo, no topo); senão a linha voltaria sobre
    // o próprio cartão, e o outro canto (vertical primeiro) é usado.
    const sx = Math.sign(eb[0] - ea[0]);
    const sy = Math.sign(eb[1] - ea[1]);
    const first = (sx === 0 || sx === da[0]) && (sy === 0 || sy === -db[1]);
    mid = first ? [[eb[0], ea[1]]] : [[ea[0], eb[1]]];
  } else {
    const sx = Math.sign(eb[0] - ea[0]);
    const sy = Math.sign(eb[1] - ea[1]);
    const first = (sy === 0 || sy === da[1]) && (sx === 0 || sx === -db[0]);
    mid = first ? [[ea[0], eb[1]]] : [[eb[0], ea[1]]];
  }
  return { points: simplify([pa, ea, ...mid, eb, pb]), handle };
}

/**
 * Liga duas caixas pelas bordas, de `a` (a tabela que tem a chave estrangeira) até `b`. Se elas se sobrepõem no outro
 * eixo a linha sai reta, no meio da faixa comum; senão faz um cotovelo entre os centros das bordas. `offA`/`offB`
 * afastam o ponto de saída de cada lado, para ligações que saem do mesmo lado do cartão não se confundirem
 * (na linha reta vale o de `a`). `adjust` posiciona o trecho do meio (a alça). `forced`: lados escolhidos pelo usuário.
 */
export function connector(a: Rect, b: Rect, offA = 0, offB = 0, adjust?: number, forced?: EndSides): Connector {
  if (forced && (forced.a || forced.b)) {
    const s = resolveSides(a, b, forced)!;
    const r = routeForced(a, b, s.a, s.b, offA, offB, adjust);
    return { points: r.points, label: childLabel(r.points), handle: r.handle };
  }
  const { points, aFirst, handle } = route(a, b, offA, offB, adjust);
  const ordered = aFirst ? points : [...points].reverse();
  return { points: ordered, label: childLabel(ordered), handle };
}

/** Laço pequeno na borda direita, para a chave estrangeira de uma tabela para ela mesma. */
export function selfLoop(a: Rect, index = 0): Connector {
  const x = a.x + a.w;
  const y = a.y + HEAD_H + 6 + index * 22;
  return { points: [[x, y], [x + 28, y], [x + 28, y + 16], [x, y + 16]], label: { x: x + 34, y: y + 12, anchor: "start" } };
}

/** Largura e altura do desenho: o cartão mais afastado mais uma margem. */
export function canvasSize(model: ErModel, pos: Record<string, Pos>): { w: number; h: number } {
  let w = 0;
  let h = 0;
  for (const t of model.tables) {
    const p = pos[t.name];
    if (!p) continue;
    w = Math.max(w, p.x + t.w + 90);
    h = Math.max(h, p.y + t.h + 90);
  }
  return { w, h };
}

/** Alinha à grade quando "Snap to Grid" está ligado. */
export const snap = (v: number, on: boolean): number => (on ? Math.round(v / GRID) * GRID : Math.round(v));

export interface Line { id: string; name: string; required: boolean; path: Connector }

/**
 * Todas as linhas do diagrama para as posições atuais dos cartões. Ligações que saem do mesmo lado de um cartão
 * ocupam pontos diferentes da borda, na ordem em que os outros cartões estão (para as linhas não se cruzarem).
 */
export function layoutLines(model: ErModel, positions: Record<string, Pos>, adjusts: Record<string, number> = {}, ends: Record<string, EndSides> = {}): Line[] {
  const byName = new Map(model.tables.map((tb) => [tb.name, tb]));
  const rect = (name: string): Rect | null => {
    const tb = byName.get(name);
    const p = positions[name];
    return tb && p ? { x: p.x, y: p.y, w: tb.w, h: tb.h } : null;
  };
  interface Link { rel: ErRelation; a: Rect; b: Rect; sides: ReturnType<typeof sidesOf> }
  const links: Link[] = [];
  const loops: { rel: ErRelation; a: Rect }[] = [];
  for (const rel of model.relations) {
    const a = rect(rel.from);
    const b = rect(rel.to);
    if (!a || !b) continue;
    if (rel.from === rel.to) loops.push({ rel, a });
    else links.push({ rel, a, b, sides: resolveSides(a, b, ends[rel.id]) });
  }
  const groups = new Map<string, { id: string; key: number }[]>();
  const add = (name: string, side: Side, id: string, other: Rect) => {
    const key = side === "left" || side === "right" ? other.y + other.h / 2 : other.x + other.w / 2;
    const k = `${name}|${side}`;
    groups.set(k, [...(groups.get(k) ?? []), { id, key }]);
  };
  for (const l of links) {
    if (!l.sides) continue;
    add(l.rel.from, l.sides.a, l.rel.id, l.b);
    add(l.rel.to, l.sides.b, l.rel.id, l.a);
  }
  for (const list of groups.values()) list.sort((x, y) => x.key - y.key);
  const offset = (name: string, side: Side, id: string) => {
    const list = groups.get(`${name}|${side}`) ?? [];
    return slotOffset(list.findIndex((e) => e.id === id), list.length);
  };

  const out: Line[] = [];
  for (const l of links) {
    const offA = l.sides ? offset(l.rel.from, l.sides.a, l.rel.id) : 0;
    const offB = l.sides ? offset(l.rel.to, l.sides.b, l.rel.id) : 0;
    out.push({ id: l.rel.id, name: l.rel.name, required: l.rel.required, path: connector(l.a, l.b, offA, offB, adjusts[l.rel.id], ends[l.rel.id]) });
  }
  const loopIndex = new Map<string, number>();
  for (const l of loops) {
    const i = loopIndex.get(l.rel.from) ?? 0;
    loopIndex.set(l.rel.from, i + 1);
    out.push({ id: l.rel.id, name: l.rel.name, required: l.rel.required, path: selfLoop(l.a, i) });
  }
  return out;
}
