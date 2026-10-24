import { memo, useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { errorText, toAtlasError, type SchemaDiagram } from "../backend";
import {
  anchorPoint, autoLayout, buildModel, canvasSize, layoutLines, SIDES, snap,
  type EndSides, type ErModel, type ErTable, type Line, type Pos, type Rect, type Side,
} from "../er";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";

interface Props {
  /** Identifica o schema para lembrar a posição dos cartões entre sessões. */
  storageKey: string;
  schema: string;
  load: () => Promise<SchemaDiagram>;
  showGrid: boolean;
  snapToGrid: boolean;
  showFkNames: boolean;
  selected: string[];
  onSelect: (names: string[]) => void;
  onOpen: (name: string) => void;
}

type State = { status: "loading" } | { status: "error"; message: string } | { status: "ready"; model: ErModel };

const STORAGE_PREFIX = "atlas.er.";

const ZOOM_STEPS = [0.25, 0.4, 0.5, 0.67, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 2, 3];

/** Próximo degrau de zoom acima (`dir` 1) ou abaixo (-1) do valor atual. */
function stepZoom(z: number, dir: 1 | -1): number {
  if (dir > 0) return ZOOM_STEPS.find((s) => s > z + 0.001) ?? ZOOM_STEPS[ZOOM_STEPS.length - 1];
  return [...ZOOM_STEPS].reverse().find((s) => s < z - 0.001) ?? ZOOM_STEPS[0];
}

function readPositions(key: string): Record<string, Pos> {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_PREFIX + key) ?? "{}") as Record<string, Pos>;
    return Object.fromEntries(Object.entries(raw).filter(([, p]) => p && Number.isFinite(p.x) && Number.isFinite(p.y)));
  } catch {
    return {};
  }
}

function readAdjusts(key: string): Record<string, number> {
  try {
    const raw = JSON.parse(localStorage.getItem(`${STORAGE_PREFIX}lines.${key}`) ?? "{}") as Record<string, number>;
    return Object.fromEntries(Object.entries(raw).filter(([, v]) => Number.isFinite(v)));
  } catch {
    return {};
  }
}

function readEnds(key: string): Record<string, EndSides> {
  try {
    const raw = JSON.parse(localStorage.getItem(`${STORAGE_PREFIX}ends.${key}`) ?? "{}") as Record<string, EndSides>;
    const ok = (v: unknown): v is Side => v === "left" || v === "right" || v === "top" || v === "bottom";
    return Object.fromEntries(Object.entries(raw).map(([id, e]) => [id, { a: ok(e?.a) ? e.a : undefined, b: ok(e?.b) ? e.b : undefined }]));
  } catch {
    return {};
  }
}

function writeEnds(key: string, ends: Record<string, EndSides>) {
  try {
    localStorage.setItem(`${STORAGE_PREFIX}ends.${key}`, JSON.stringify(ends));
  } catch {
    // Sem armazenamento disponível: os lados das pontas só valem nesta sessão.
  }
}

function writeAdjusts(key: string, adjusts: Record<string, number>) {
  try {
    localStorage.setItem(`${STORAGE_PREFIX}lines.${key}`, JSON.stringify(adjusts));
  } catch {
    // Sem armazenamento disponível: o caminho das linhas só vale nesta sessão.
  }
}

function writePositions(key: string, pos: Record<string, Pos>) {
  try {
    localStorage.setItem(STORAGE_PREFIX + key, JSON.stringify(pos));
  } catch {
    // Sem armazenamento disponível: as posições só valem nesta sessão.
  }
}

/**
 * Arrasto com o mouse. Os movimentos são juntados e aplicados no máximo uma vez por quadro (o mouse manda bem mais
 * eventos que a tela desenha); ao soltar, aplica o último e chama `finish`.
 */
function trackDrag(apply: (ev: MouseEvent) => void, finish: () => void) {
  let frame = 0;
  let last: MouseEvent | null = null;
  const flush = () => {
    frame = 0;
    if (last) {
      apply(last);
      last = null;
    }
  };
  const onMove = (ev: MouseEvent) => {
    last = ev;
    if (!frame) frame = requestAnimationFrame(flush);
  };
  const onUp = () => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    if (frame) cancelAnimationFrame(frame);
    flush();
    finish();
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}

interface CardProps {
  tb: ErTable;
  x: number;
  y: number;
  selected: boolean;
  related: boolean;
  /** Colunas da chave estrangeira selecionada que pertencem a este cartão. */
  highlight?: Set<string>;
  onDown: (e: ReactMouseEvent, name: string) => void;
  onOpen: (name: string) => void;
}

/**
 * Cartão de uma tabela. É memorizado: arrastar um cartão redesenha só ele (os outros recebem as mesmas props), e a
 * posição vai por `transform`, que o navegador move sem refazer o layout da página.
 */
const ErCard = memo(function ErCard({ tb, x, y, selected, related, highlight, onDown, onOpen }: CardProps) {
  const t = useT();
  return (
    <div
      data-name={tb.name}
      className={`er-card${selected ? " is-selected" : ""}${related ? " is-related" : ""}`}
      style={{ transform: `translate(${x}px, ${y}px)`, width: tb.w, height: tb.h }}
      onMouseDown={(e) => onDown(e, tb.name)}
      onDoubleClick={() => onOpen(tb.name)}
    >
      <div className="er-card-head" title={tb.name}>{tb.name}</div>
      <div className="er-card-body">
        {tb.columns.map((c) => (
          <div key={c.name} className={`er-col${highlight?.has(c.name) ? " is-hl" : ""}`}>
            <span className="er-col-icon">
              {c.isPk ? <SmallIcon name="columnPk" size={12} /> : c.isFk ? <SmallIcon name="columnFk" size={12} /> : null}
            </span>
            <span className="er-col-name">{c.name}</span>
            <span className="er-col-type">: {c.type}</span>
          </div>
        ))}
        {tb.hidden > 0 && <div className="er-col er-more"><span className="er-col-icon" />… {t("er.moreColumns", { n: tb.hidden })}</div>}
      </div>
    </div>
  );
});

/** Diagrama ER do schema: cartões de tabela arrastáveis, com as chaves estrangeiras como linhas. */
export function ErDiagram({ storageKey, schema, load, showGrid, snapToGrid, showFkNames, selected, onSelect, onOpen }: Props) {
  const t = useT();
  const [state, setState] = useState<State>({ status: "loading" });
  const [positions, setPositions] = useState<Record<string, Pos>>({});
  const [reloadKey, setReloadKey] = useState(0);
  /** Chave estrangeira selecionada (clicar na linha) e a dica que acompanha o mouse sobre ela. */
  const [selRel, setSelRel] = useState<string | null>(null);
  const [tip, setTip] = useState<{ x: number; y: number; id: string } | null>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const posRef = useRef(positions);
  posRef.current = positions;
  /** Caminho escolhido pelo usuário para cada linha (ver `LineHandle`), por id da chave estrangeira. */
  const [adjusts, setAdjusts] = useState<Record<string, number>>({});
  const adjRef = useRef(adjusts);
  adjRef.current = adjusts;
  /** Lado do cartão escolhido para cada ponta de cada linha, e a ponta que está sendo arrastada agora. */
  const [ends, setEnds] = useState<Record<string, EndSides>>({});
  const endsRef = useRef(ends);
  endsRef.current = ends;
  const [endDrag, setEndDrag] = useState<{ id: string; end: "a" | "b"; side: Side } | null>(null);
  /** Enquanto arrasta, a dica não acompanha o mouse (cada atualização redesenharia o diagrama à toa). */
  const dragging = useRef(false);
  /** Zoom do diagrama (1 = 100%) e tela cheia. As coordenadas do mouse são divididas pelo zoom para valerem no desenho. */
  const [zoom, setZoom] = useState(1);
  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;
  const wrapRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({ w: 0, h: 0 });
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewport({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Ctrl + roda do mouse: zoom (o ouvinte não é passivo para poder cancelar o zoom da página).
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      setZoom((z) => stepZoom(z, e.deltaY < 0 ? 1 : -1));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // Tela cheia: o diagrama cobre a janela do app (o navegador pode recusar a tela cheia nativa); Esc sai.
  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setFullscreen(false); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen]);
  const toggleFullscreen = () => setFullscreen((f) => !f);

  useEffect(() => {
    let alive = true;
    setState({ status: "loading" });
    setSelRel(null);
    load()
      .then((diagram) => {
        if (!alive) return;
        const model = buildModel(diagram, schema);
        const saved = readPositions(storageKey);
        const merged = autoLayout(model);
        for (const tb of model.tables) if (saved[tb.name]) merged[tb.name] = saved[tb.name];
        setPositions(merged);
        setAdjusts(readAdjusts(storageKey));
        setEnds(readEnds(storageKey));
        setState({ status: "ready", model });
      })
      .catch((e) => alive && setState({ status: "error", message: errorText(toAtlasError(e)) }));
    return () => {
      alive = false;
    };
    // `load` é recriada a cada render do pai; só o schema e o botão Atualizar recarregam.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storageKey, schema, reloadKey]);

  const model = state.status === "ready" ? state.model : null;
  const content = useMemo(() => (model ? canvasSize(model, positions) : { w: 0, h: 0 }), [model, positions]);
  /** O desenho ocupa pelo menos a área visível (em unidades do desenho, já descontado o zoom). */
  const size = { w: Math.max(content.w, Math.ceil(viewport.w / zoom)), h: Math.max(content.h, Math.ceil(viewport.h / zoom)) };

  // Durante o arrasto de uma ponta, a linha já se refaz com o lado mais próximo do mouse.
  const liveEnds = useMemo(
    () => (endDrag ? { ...ends, [endDrag.id]: { ...ends[endDrag.id], [endDrag.end]: endDrag.side } } : ends),
    [ends, endDrag],
  );
  const lines = useMemo<Line[]>(() => (model ? layoutLines(model, positions, adjusts, liveEnds) : []), [model, positions, adjusts, liveEnds]);

  const relById = useMemo(() => new Map((model?.relations ?? []).map((r) => [r.id, r])), [model]);
  const rel = selRel ? relById.get(selRel) ?? null : null;
  /** Colunas da chave selecionada, por tabela: ficam destacadas nos dois cartões. */
  const hl = useMemo(() => {
    const m = new Map<string, Set<string>>();
    if (rel) {
      m.set(rel.from, new Set(rel.fromColumns));
      m.set(rel.to, new Set([...(m.get(rel.to) ?? []), ...rel.toColumns]));
    }
    return m;
  }, [rel]);
  const tipRel = tip ? relById.get(tip.id) ?? null : null;

  const hoverLine = (e: ReactMouseEvent, id: string) => {
    if (dragging.current) return;
    const box = canvasRef.current?.getBoundingClientRect();
    if (box) setTip({ x: (e.clientX - box.left) / zoomRef.current, y: (e.clientY - box.top) / zoomRef.current, id });
  };

  /** Marca o início e o fim de um arrasto no canvas (cursor de "agarrado" e linhas sem reagir ao mouse). */
  const setDragging = (on: boolean) => {
    dragging.current = on;
    const el = canvasRef.current;
    if (!el) return;
    if (on) el.dataset.dragging = "1";
    else delete el.dataset.dragging;
  };

  /** Seleciona a chave estrangeira e, arrastando a linha ou a alça, move o trecho do meio dela. */
  const startLineDrag = (e: ReactMouseEvent, line: Line) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    setSelRel(line.id);
    onSelect([]);
    const handle = line.path.handle;
    if (!handle) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let moved = false;
    trackDrag(
      (ev) => {
        const delta = (handle.axis === "x" ? ev.clientX - startX : ev.clientY - startY) / zoomRef.current;
        if (!moved && Math.abs(delta) < 4) return;
        if (!moved) {
          moved = true;
          setTip(null);
          setDragging(true);
        }
        const value = Math.min(handle.max, Math.max(handle.min, snap(handle.value + delta, snapToGrid)));
        setAdjusts((a) => (a[line.id] === value ? a : { ...a, [line.id]: value }));
      },
      () => {
        if (!moved) return;
        setDragging(false);
        writeAdjusts(storageKey, adjRef.current);
      },
    );
  };

  /** Duplo clique na linha: volta ao caminho automático (trecho do meio e lados das pontas). */
  const resetLine = (id: string) => {
    if (id in adjRef.current) {
      const { [id]: _removed, ...rest } = adjRef.current;
      setAdjusts(rest);
      writeAdjusts(storageKey, rest);
    }
    if (id in endsRef.current) {
      const { [id]: _removed, ...rest } = endsRef.current;
      setEnds(rest);
      writeEnds(storageKey, rest);
    }
  };

  /** Retângulo de uma tabela na posição atual. */
  const rectOf = (name: string): Rect | null => {
    const tb = model?.tables.find((x) => x.name === name);
    const p = posRef.current[name];
    return tb && p ? { x: p.x, y: p.y, w: tb.w, h: tb.h } : null;
  };

  /** Arrastar a bolinha de uma ponta: mostra os 4 pontos de ligação do cartão e encaixa no mais próximo ao soltar. */
  const startEndDrag = (e: ReactMouseEvent, id: string, end: "a" | "b") => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const r = relById.get(id);
    const rect = r ? rectOf(end === "a" ? r.from : r.to) : null;
    const box = canvasRef.current?.getBoundingClientRect();
    if (!r || !rect || !box) return;
    let chosen: Side | null = null;
    let started = false;
    trackDrag(
      (ev) => {
        if (!started) {
          started = true;
          setTip(null);
          setDragging(true);
        }
        const px = (ev.clientX - box.left) / zoomRef.current;
        const py = (ev.clientY - box.top) / zoomRef.current;
        let best: Side = SIDES[0];
        let bestD = Infinity;
        for (const side of SIDES) {
          const [ax, ay] = anchorPoint(rect, side);
          const d = (ax - px) ** 2 + (ay - py) ** 2;
          if (d < bestD) {
            bestD = d;
            best = side;
          }
        }
        chosen = best;
        setEndDrag((cur) => (cur && cur.id === id && cur.end === end && cur.side === best ? cur : { id, end, side: best }));
      },
      () => {
        setDragging(false);
        setEndDrag(null);
        if (!chosen) return;
        const next = { ...endsRef.current, [id]: { ...endsRef.current[id], [end]: chosen } };
        setEnds(next);
        writeEnds(storageKey, next);
      },
    );
  };

  const startDrag = (e: ReactMouseEvent, name: string) => {
    if (e.button !== 0) return;
    setSelRel(null);
    if (e.ctrlKey || e.metaKey) onSelect(selected.includes(name) ? selected.filter((n) => n !== name) : [...selected, name]);
    else if (!selected.includes(name)) onSelect([name]);
    const origin = positions[name];
    if (!origin) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let moved = false;
    trackDrag(
      (ev) => {
        const dx = (ev.clientX - startX) / zoomRef.current;
        const dy = (ev.clientY - startY) / zoomRef.current;
        if (!moved && Math.abs(dx) + Math.abs(dy) < 4) return;
        if (!moved) {
          moved = true;
          setTip(null);
          setDragging(true);
        }
        const x = Math.max(0, snap(origin.x + dx, snapToGrid));
        const y = Math.max(0, snap(origin.y + dy, snapToGrid));
        setPositions((p) => (p[name]?.x === x && p[name]?.y === y ? p : { ...p, [name]: { x, y } }));
      },
      () => {
        if (!moved) return;
        setDragging(false);
        writePositions(storageKey, posRef.current);
      },
    );
  };

  // Callbacks estáveis para os cartões memorizados (sempre chamam a versão mais recente).
  const startDragRef = useRef(startDrag);
  startDragRef.current = startDrag;
  const onCardDown = useCallback((e: ReactMouseEvent, name: string) => startDragRef.current(e, name), []);
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;
  const onCardOpen = useCallback((name: string) => onOpenRef.current(name), []);

  const selectedSet = new Set(selected);

  return (
    <div className={`er-wrap${fullscreen ? " is-fullscreen" : ""}`} ref={wrapRef}>
      <div className="er-scroll" ref={scrollRef}>
        {state.status === "loading" && <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>}
        {state.status === "error" && <div className="doc-banner is-warn">{state.message}</div>}
        {model && model.tables.length === 0 && <div className="er-empty muted">{t("er.empty")}</div>}
        {model && model.tables.length > 0 && (
          <div className="er-zoomer" style={{ width: size.w * zoom, height: size.h * zoom }}>
          <div
            ref={canvasRef}
            className={`er-canvas${showGrid ? " has-grid" : ""}`}
            style={{ width: size.w, height: size.h, transform: `scale(${zoom})` }}
            onMouseDown={(e) => {
              if (e.target !== e.currentTarget) return;
              setSelRel(null);
              onSelect([]);
            }}
          >
            <svg className="er-lines" width={size.w} height={size.h} aria-hidden="true">
              {lines.map((l) => {
                const pts = l.path.points.map((q) => q.join(",")).join(" ");
                return (
                  <g key={l.id} className={`er-line${selRel === l.id ? " is-selected" : ""}`}>
                    {selRel === l.id && <polyline className="er-line-glow" points={pts} />}
                    <polyline className="er-line-vis" points={pts} />
                    <polyline
                      className={`er-line-hit${selRel === l.id && l.path.handle ? ` is-${l.path.handle.axis}` : ""}`}
                      points={pts}
                      onMouseDown={(e) => startLineDrag(e, l)}
                      onDoubleClick={() => resetLine(l.id)}
                      onMouseMove={(e) => hoverLine(e, l.id)}
                      onMouseLeave={() => setTip(null)}
                    />
                  </g>
                );
              })}
            </svg>
            {model.tables.map((tb) => {
              const p = positions[tb.name];
              if (!p) return null;
              return (
                <ErCard
                  key={tb.name}
                  tb={tb}
                  x={p.x}
                  y={p.y}
                  selected={selectedSet.has(tb.name)}
                  related={!!rel && (rel.from === tb.name || rel.to === tb.name)}
                  highlight={hl.get(tb.name)}
                  onDown={onCardDown}
                  onOpen={onCardOpen}
                />
              );
            })}
            <svg className="er-lines er-lines-top" width={size.w} height={size.h} aria-hidden="true">
              {lines.map((l) => {
                const first = l.path.points[0];
                const last = l.path.points[l.path.points.length - 1];
                // Nome da FK e se a referência é obrigatória (todas as colunas NOT NULL) ou opcional.
                const text = `${l.name} · ${t(l.required ? "er.required" : "er.optional")}`;
                const textW = text.length * 6.8;
                const x0 = l.path.label.anchor === "start" ? l.path.label.x : l.path.label.x - (textW + 18);
                return (
                  <g key={l.id}>
                    {selRel === l.id && (
                      <>
                        <circle className="er-end" cx={first[0]} cy={first[1]} r={4} onMouseDown={(e) => startEndDrag(e, l.id, "a")} />
                        <circle className="er-end" cx={last[0]} cy={last[1]} r={4} onMouseDown={(e) => startEndDrag(e, l.id, "b")} />
                        {l.path.handle && (
                          <rect
                            className={`er-handle is-${l.path.handle.axis}`}
                            x={l.path.handle.at[0] - 4}
                            y={l.path.handle.at[1] - 4}
                            width={8}
                            height={8}
                            rx={1.5}
                            onMouseDown={(e) => startLineDrag(e, l)}
                            onDoubleClick={() => resetLine(l.id)}
                          />
                        )}
                      </>
                    )}
                    {showFkNames && (
                      <g className="er-label">
                        <g className="er-link-icon" transform={`translate(${x0},${l.path.label.y - 11})`}>
                          <rect x="0.8" y="6" width="8" height="4.4" rx="2.2" transform="rotate(-40 4.8 8.2)" />
                          <rect x="6" y="2.4" width="8" height="4.4" rx="2.2" transform="rotate(-40 10 4.6)" />
                        </g>
                        <text x={x0 + 18} y={l.path.label.y}>{text}</text>
                      </g>
                    )}
                  </g>
                );
              })}
            </svg>
            {endDrag && (() => {
              const r = relById.get(endDrag.id);
              const rect = r ? rectOf(endDrag.end === "a" ? r.from : r.to) : null;
              if (!rect) return null;
              return (
                <svg className="er-lines er-lines-top" width={size.w} height={size.h} aria-hidden="true">
                  {SIDES.map((side) => {
                    const [x, y] = anchorPoint(rect, side);
                    return <rect key={side} className={`er-anchor${side === endDrag.side ? " is-near" : ""}`} x={x - 5} y={y - 5} width={10} height={10} rx={1.5} />;
                  })}
                </svg>
              );
            })()}
            {tip && tipRel && (
              <div className="er-tip" style={{ left: tip.x + 14, top: tip.y + 18 }}>
                <b>{tipRel.name}</b>
                <span>{t("er.referencing")}</span> {tipRel.from}
                <br />
                <span>{t("er.reference")}</span> {t(tipRel.required ? "er.required" : "er.optional")}
              </div>
            )}
          </div>
          </div>
        )}
      </div>
      <div className="er-bar">
        <button type="button" className="ot-button" disabled={state.status === "loading"} onClick={() => setReloadKey((k) => k + 1)}>
          <SmallIcon name="refresh" />
          <span>{t("er.refresh")}</span>
        </button>
        <span className="muted">{model ? t("er.summary", { tables: model.tables.length, relations: lines.length }) : ""}</span>
        <span className="er-spacer" />
        <div className="er-zoom" role="group" aria-label={t("er.zoom")}>
          <button type="button" className="icon-button" title={t("er.zoomOut")} aria-label={t("er.zoomOut")} disabled={zoom <= ZOOM_STEPS[0]} onClick={() => setZoom((z) => stepZoom(z, -1))}>−</button>
          <button type="button" className="er-zoom-value" title={t("er.zoomReset")} onClick={() => setZoom(1)}>{Math.round(zoom * 100)}%</button>
          <button type="button" className="icon-button" title={t("er.zoomIn")} aria-label={t("er.zoomIn")} disabled={zoom >= ZOOM_STEPS[ZOOM_STEPS.length - 1]} onClick={() => setZoom((z) => stepZoom(z, 1))}>+</button>
        </div>
        <button type="button" className="icon-button" title={t(fullscreen ? "er.exitFullscreen" : "er.fullscreen")} aria-label={t(fullscreen ? "er.exitFullscreen" : "er.fullscreen")} onClick={toggleFullscreen}>
          <SmallIcon name="fullscreen" />
        </button>
      </div>
    </div>
  );
}
