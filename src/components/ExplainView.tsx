import { useEffect, useMemo, useRef, useState } from "react";
import { chooseExportPath, errorText, local, toAtlasError } from "../backend";
import { flatten, layout, nodeText, nodeTip, parsePlan, statsByType, type PlanNode } from "../explain";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";

/** O que a aba Explain mostra: o plano nos dois formatos e de onde ele veio. */
export interface ExplainData {
  json: string;
  text: string;
  /** A consulta explicada. */
  sql: string;
  at: string;
  ms: number;
  connection: string;
  database: string;
  schema?: string;
}

type Sub = "visual" | "json" | "text" | "statistics" | "info";
const SUBS: Sub[] = ["visual", "json", "text", "statistics", "info"];

const COL_W = 130;
const ROW_H = 90;
const PAD_X = 50;
const PAD_Y = 50;
const ZOOMS = [0.4, 0.5, 0.67, 0.8, 1, 1.25, 1.5, 2, 3];
const RED = "#d32f2f";

const fmtCost = (n: number) => String(Number(n.toFixed(2)));
const fmtStat = (n: number) => n.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/** Cores do tema atual (o desenho vira arquivo, então não pode depender de variáveis CSS). */
function themeColors() {
  const css = getComputedStyle(document.documentElement);
  const get = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return { text: get("--c-text", "#1f1f1f"), line: "#9aa5b1", bg: get("--c-surface", "#ffffff"), mark: "#f2b01e" };
}

/** Ícone do nó: tabela (leitura), junção ou operação genérica. */
function NodeIcon({ type }: { type: string }) {
  if (/scan/i.test(type)) {
    return (
      <g>
        <rect x={-11} y={-12} width={22} height={22} rx={2} fill="#8a96a3" />
        <rect x={-11} y={-12} width={22} height={6} rx={2} fill="#5d6b7a" />
        <path d="M-11 -1h22M-11 5h22M0 -6v16" stroke="#d9dee4" strokeWidth={1} />
        <path d="M-11 4v10M-14.5 11l3.5 3.5 3.5-3.5" fill="none" stroke="#1e9be8" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" />
      </g>
    );
  }
  if (/join|nested loop/i.test(type)) {
    return (
      <g>
        <rect x={-14} y={-12} width={6} height={24} rx={3} fill="#9ccbf2" />
        <rect x={-6} y={-8} width={18} height={18} rx={2} fill="#7d8a97" />
        <path d="M-12 0h14M-1 -4l4 4-4 4" fill="none" stroke="#2fb34f" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" />
      </g>
    );
  }
  return (
    <g>
      <rect x={-12} y={-12} width={24} height={24} rx={3} fill="#8a96a3" />
      <path d="M-6 0h12M2 -4l4 4-4 4" fill="none" stroke="#fff" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" />
    </g>
  );
}

const jsonToken = /("(?:[^"\\]|\\.)*")(\s*:)?|\b(true|false|null)\b|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g;

/** JSON com as cores da referência: textos em vermelho, números em verde e verdadeiro/falso em azul. */
function highlightJson(src: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  let last = 0;
  let k = 0;
  for (const m of src.matchAll(jsonToken)) {
    const at = m.index ?? 0;
    if (at > last) out.push(src.slice(last, at));
    if (m[1] !== undefined) {
      out.push(m[2] !== undefined ? <span key={k++}>{m[1]}{m[2]}</span> : <span key={k++} className="ex-str">{m[1]}</span>);
    } else if (m[3] !== undefined) {
      out.push(<span key={k++} className="ex-kw">{m[3]}</span>);
    } else {
      out.push(<span key={k++} className="ex-num">{m[4]}</span>);
    }
    last = at + m[0].length;
  }
  if (last < src.length) out.push(src.slice(last));
  return out;
}

interface Props {
  data: ExplainData;
}

export function ExplainView({ data }: Props) {
  const t = useT();
  const [sub, setSub] = useState<Sub>("visual");
  const [threshold, setThreshold] = useState("50");
  const [zoom, setZoom] = useState(1);
  const [find, setFind] = useState("");
  const [hit, setHit] = useState(0);
  const [note, setNote] = useState<string | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);

  const plan = useMemo(() => parsePlan(data.json), [data.json]);
  const pretty = useMemo(() => {
    try { return JSON.stringify(JSON.parse(data.json), null, 2); } catch { return data.json; }
  }, [data.json]);
  const placed = useMemo(() => (plan ? layout(plan) : null), [plan]);
  const limit = Math.min(100, Math.max(0, Number(threshold) || 0));

  const needle = find.trim().toLowerCase();
  const matches = useMemo(() => (plan && needle ? flatten(plan).filter((n) => nodeText(n).includes(needle)).map((n) => n.id) : []), [plan, needle]);
  const current = matches.length ? matches[hit % matches.length] : null;
  useEffect(() => setHit(0), [needle]);
  useEffect(() => {
    if (current !== null) svgRef.current?.querySelector(`[data-node="${current}"]`)?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [current, hit]);

  const step = (dir: 1 | -1) => setZoom((z) => (dir > 0 ? ZOOMS.find((s) => s > z + 0.001) ?? z : [...ZOOMS].reverse().find((s) => s < z - 0.001) ?? z));

  const width = placed ? PAD_X * 2 + (placed.cols - 1) * COL_W : 0;
  const height = placed ? PAD_Y * 2 + (placed.rows - 1) * ROW_H : 0;
  const pos = (col: number, row: number) => ({ x: PAD_X + col * COL_W, y: PAD_Y + row * ROW_H });
  const colors = themeColors();

  const saveImage = async () => {
    const svg = svgRef.current;
    if (!svg) return;
    setNote(null);
    try {
      const path = await chooseExportPath("explain.svg", ["svg"]);
      if (!path) return;
      const copy = svg.cloneNode(true) as SVGSVGElement;
      copy.setAttribute("xmlns", "http://www.w3.org/2000/svg");
      copy.removeAttribute("style");
      copy.setAttribute("width", String(width));
      copy.setAttribute("height", String(height));
      const bytes = new TextEncoder().encode(`<?xml version="1.0" encoding="UTF-8"?>\n${copy.outerHTML}`);
      let hex = "";
      for (const b of bytes) hex += b.toString(16).padStart(2, "0");
      await local.fileWriteHex(path, hex);
      setNote(t("ex.saved", { path }));
    } catch (e) {
      setNote(errorText(toAtlasError(e)));
    }
  };

  const renderNode = (n: PlanNode, col: number, row: number) => {
    const { x, y } = pos(col, row);
    const hot = n.ownPct >= limit && limit > 0 && n.ownCost > 0;
    const marked = matches.includes(n.id);
    return (
      <g key={n.id} data-node={n.id} transform={`translate(${x} ${y})`} fontFamily="Segoe UI, sans-serif" fontSize={11} textAnchor="middle">
        <title>{nodeTip(n)}</title>
        {marked && <rect x={-34} y={-30} width={68} height={68} rx={6} fill="none" stroke={colors.mark} strokeWidth={n.id === current ? 3 : 1.5} />}
        <text y={-19} fill={colors.text}>{fmtCost(n.totalCost)}</text>
        <NodeIcon type={n.type} />
        <text y={32} fill={hot ? RED : colors.text}>{n.label.length > 18 ? `${n.label.slice(0, 17)}…` : n.label}</text>
      </g>
    );
  };

  const body = () => {
    switch (sub) {
      case "visual":
        return plan && placed ? (
          <div className="ex-canvas">
            <svg ref={svgRef} width={width * zoom} height={height * zoom} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={t("ex.visual")}>
              <rect width={width} height={height} fill={colors.bg} />
              {placed.placed.flatMap((p) => p.node.children.map((c) => {
                const child = placed.placed.find((q) => q.node === c)!;
                const a = pos(p.col, p.row);
                const b = pos(child.col, child.row);
                const x1 = a.x + 18;
                const x2 = b.x - 18;
                const mid = (x1 + x2) / 2;
                return <path key={`${p.node.id}-${c.id}`} d={`M${x1} ${a.y}C${mid} ${a.y} ${mid} ${b.y} ${x2} ${b.y}`} fill="none" stroke={colors.line} strokeWidth={1.5} />;
              }))}
              {placed.placed.map((p) => renderNode(p.node, p.col, p.row))}
            </svg>
          </div>
        ) : <div className="muted ex-pad">{t("ex.noPlan")}</div>;
      case "json":
        return <pre className="ex-pre">{highlightJson(pretty)}</pre>;
      case "text":
        return <pre className="ex-pre">{data.text}</pre>;
      case "statistics": {
        const rows = plan ? statsByType(plan) : [];
        return (
          <table className="ex-stats">
            <thead><tr><th>{t("ex.nodeType")}</th><th className="num">{t("ex.count")}</th><th className="num">{t("ex.cost")}</th><th className="num">{t("ex.costPct")}</th></tr></thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.type}><td>{r.type}</td><td className="num">{r.count}</td><td className="num">{fmtStat(r.cost)}</td><td className="num">{fmtStat(r.pct)}%</td></tr>
              ))}
            </tbody>
          </table>
        );
      }
      case "info":
        return (
          <div className="ex-info">
            <div className="ex-info-head">
              <div><span className="muted">{t("ex.timestamp")}</span> {data.at}</div>
              <div><span className="muted">{t("ex.queryTime")}</span> {(data.ms / 1000).toFixed(3)}s</div>
              <div><span className="muted">{t("ex.connection")}</span> {data.connection}</div>
              <div><span className="muted">{t("ex.database")}</span> {data.database}</div>
              {data.schema && <div><span className="muted">{t("ex.schema")}</span> {data.schema}</div>}
            </div>
            <pre className="ex-sql">{data.sql}</pre>
          </div>
        );
    }
  };

  return (
    <div className="ex-view">
      <div className="ex-bar">
        <div className="ex-subs" role="tablist">
          {SUBS.map((s) => (
            <button key={s} type="button" role="tab" aria-selected={sub === s} className={`ex-sub${sub === s ? " is-active" : ""}`} onClick={() => setSub(s)}>{t(`ex.${s}` as "ex.visual")}</button>
          ))}
        </div>
        {sub === "visual" && (
          <div className="ex-tools">
            <label className="ex-thr">{t("ex.threshold")}
              <input className="field" inputMode="numeric" value={threshold} onChange={(e) => setThreshold(e.target.value.replace(/\D/g, "").slice(0, 3))} />
            </label>
            <button type="button" className="icon-button" title={t("er.zoomIn")} aria-label={t("er.zoomIn")} disabled={zoom >= ZOOMS[ZOOMS.length - 1]} onClick={() => step(1)}>+</button>
            <button type="button" className="icon-button" title={t("er.zoomOut")} aria-label={t("er.zoomOut")} disabled={zoom <= ZOOMS[0]} onClick={() => step(-1)}>−</button>
            <button type="button" className="icon-button" title={t("ex.saveImage")} aria-label={t("ex.saveImage")} disabled={!plan} onClick={() => void saveImage()}><SmallIcon name="save" /></button>
            <span className="ex-sep" />
            <input
              className="field ex-find"
              placeholder={t("ex.find")}
              value={find}
              onChange={(e) => setFind(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") setHit((h) => h + (e.shiftKey ? matches.length - 1 : 1)); }}
            />
            <button type="button" className="icon-button" title={t("ex.prev")} aria-label={t("ex.prev")} disabled={matches.length < 2} onClick={() => setHit((h) => h + matches.length - 1)}>‹</button>
            <button type="button" className="icon-button" title={t("ex.next")} aria-label={t("ex.next")} disabled={matches.length < 2} onClick={() => setHit((h) => h + 1)}>›</button>
          </div>
        )}
        {sub === "statistics" && (
          <div className="ex-tools">
            <select className="field" aria-label={t("ex.group")} value="type" onChange={() => undefined}>
              <option value="type">{t("ex.perType")}</option>
            </select>
          </div>
        )}
      </div>
      {note && <div className="muted ex-note">{note}</div>}
      <div className="ex-body">{body()}</div>
    </div>
  );
}
