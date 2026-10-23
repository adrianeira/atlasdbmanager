import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useT } from "../i18n";

export type DateTimeKind = "date" | "time" | "datetime";

interface Props {
  kind: DateTimeKind;
  /** Valor atual (formato PostgreSQL ou vazio). */
  value: string;
  /** Posição-âncora da célula; o popup abre logo abaixo (ou acima se não couber). */
  anchor: DOMRect;
  onPick: (v: string) => void;
  onClose: () => void;
}

const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);

/** Lê o valor como { date: "YYYY-MM-DD" | "", time: "HH:MM:SS" | "" }. */
function parse(v: string): { date: string; time: string } {
  const s = v.trim().replace(/([+-]\d{2}(:?\d{2})?|Z)$/, "").replace("T", " ");
  const m = s.match(/^(\d{4}-\d{2}-\d{2})?[ ]?(\d{2}:\d{2}(?::\d{2})?)?/);
  return { date: m?.[1] ?? "", time: (m?.[2] ?? "").padEnd(8, ":00").slice(0, 8).replace(/::/g, ":00:").replace(/:$/, "") };
}

/** Compõe o valor de saída conforme o tipo. */
function format(kind: DateTimeKind, date: string, time: string): string {
  if (kind === "date") return date;
  if (kind === "time") return time;
  return date && time ? `${date} ${time}` : date || time;
}

export function DateTimePopup({ kind, value, anchor, onPick, onClose }: Props) {
  const t = useT();
  const { date: initDate, time: initTime } = useMemo(() => parse(value), [value]);
  const today = new Date();
  const todayISO = `${today.getFullYear()}-${pad(today.getMonth() + 1)}-${pad(today.getDate())}`;
  const seed = initDate || todayISO;
  const seedParts = seed.split("-").map(Number);

  const [year, setYear] = useState(seedParts[0] || today.getFullYear());
  const [month, setMonth] = useState((seedParts[1] || today.getMonth() + 1) - 1);
  const [picked, setPicked] = useState(initDate);
  const [timeStr, setTimeStr] = useState(initTime || "00:00:00");
  const boxRef = useRef<HTMLDivElement>(null);

  // Fecha ao clicar fora ou pressionar Esc.
  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Element | null;
      // Clicar no próprio editor da célula não fecha o popup (ele é a âncora).
      if (boxRef.current?.contains(t as Node)) return;
      if (t && (t as Element).closest?.(".dg-editor")) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("mousedown", onDoc, true);
    document.addEventListener("keydown", onKey);
    return () => { document.removeEventListener("mousedown", onDoc, true); document.removeEventListener("keydown", onKey); };
  }, [onClose]);

  const monthName = new Date(year, month, 1).toLocaleDateString(undefined, { month: "long", year: "numeric" });
  const first = new Date(year, month, 1);
  const start = (first.getDay() + 6) % 7; // segunda = 0 na tabela
  const daysInMonth = new Date(year, month + 1, 0).getDate();
  const daysPrev = new Date(year, month, 0).getDate();

  type Cell = { d: number; m: number; y: number; other: boolean };
  const cells: Cell[] = [];
  for (let i = 0; i < start; i++) {
    cells.push({ d: daysPrev - start + 1 + i, m: month - 1, y: year, other: true });
  }
  for (let d = 1; d <= daysInMonth; d++) cells.push({ d, m: month, y: year, other: false });
  while (cells.length % 7 !== 0 || cells.length < 42) cells.push({ d: cells.length - start - daysInMonth + 1, m: month + 1, y: year, other: true });

  const normMonth = (y: number, m: number) => { while (m < 0) { m += 12; y--; } while (m > 11) { m -= 12; y++; } return { y, m }; };
  const navMonth = (delta: number) => { const { y, m } = normMonth(year, month + delta); setYear(y); setMonth(m); };

  const pickDay = (c: Cell) => {
    const n = normMonth(c.y, c.m);
    const iso = `${n.y}-${pad(n.m + 1)}-${pad(c.d)}`;
    setYear(n.y); setMonth(n.m); setPicked(iso);
    if (kind === "date") onPick(iso);
  };

  const commit = () => onPick(format(kind, picked, timeStr));

  const needDate = kind !== "time";
  const needTime = kind !== "date";

  // Posicionamento: logo abaixo da âncora, caindo acima se não couber na tela.
  const width = 232 + (needTime ? 108 : 0);
  const height = needDate ? 268 : 160;
  const left = Math.max(8, Math.min(anchor.left, window.innerWidth - width - 8));
  const top = anchor.bottom + height + 8 < window.innerHeight ? anchor.bottom + 2 : Math.max(8, anchor.top - height - 2);

  const dow = ["D", "S", "T", "Q", "Q", "S", "S"]; // sáb/dom primeiro/último
  const [hh, mm, ss] = (timeStr.split(":").map(Number).concat([0, 0, 0])).slice(0, 3);
  const setTime = (h: number, m: number, s: number) => setTimeStr(`${pad((h + 24) % 24)}:${pad((m + 60) % 60)}:${pad((s + 60) % 60)}`);

  return createPortal(
    <div ref={boxRef} className="dtp" style={{ position: "fixed", left, top, width }} role="dialog">
      {needDate && (
        <div className="dtp-cal">
          <div className="dtp-head">
            <button type="button" className="icon-button" onClick={() => navMonth(-1)} aria-label="◀">◀</button>
            <span className="dtp-month">{monthName}</span>
            <button type="button" className="icon-button" onClick={() => navMonth(1)} aria-label="▶">▶</button>
          </div>
          <div className="dtp-dow">{dow.map((d, i) => <span key={i}>{d}</span>)}</div>
          <div className="dtp-grid">
            {cells.map((c, i) => {
              const iso = `${normMonth(c.y, c.m).y}-${pad(normMonth(c.y, c.m).m + 1)}-${pad(c.d)}`;
              const isSel = iso === picked;
              const isToday = iso === todayISO;
              return (
                <button
                  key={i}
                  type="button"
                  className={`dtp-day${c.other ? " is-other" : ""}${isSel ? " is-sel" : ""}${isToday ? " is-today" : ""}`}
                  onClick={() => pickDay(c)}
                >
                  {c.d}
                </button>
              );
            })}
          </div>
        </div>
      )}
      {needTime && (
        <div className="dtp-time">
          <div className="dtp-time-head">{t("dtp.time")}</div>
          <div className="dtp-time-row">
            <TimeSpin value={hh} max={23} onChange={(v) => setTime(v, mm, ss)} />
            <span>:</span>
            <TimeSpin value={mm} max={59} onChange={(v) => setTime(hh, v, ss)} />
            <span>:</span>
            <TimeSpin value={ss} max={59} onChange={(v) => setTime(hh, mm, v)} />
          </div>
        </div>
      )}
      <div className="dtp-foot">
        <button type="button" className="link" onClick={() => onPick("")}>{t("dtp.clear")}</button>
        <button type="button" className="link" onClick={() => {
          const n = new Date();
          const iso = `${n.getFullYear()}-${pad(n.getMonth() + 1)}-${pad(n.getDate())}`;
          const hms = `${pad(n.getHours())}:${pad(n.getMinutes())}:${pad(n.getSeconds())}`;
          setPicked(iso); setTimeStr(hms); setYear(n.getFullYear()); setMonth(n.getMonth());
          if (kind === "date") onPick(iso);
          else if (kind === "time") onPick(hms);
          else onPick(`${iso} ${hms}`);
        }}>{t("dtp.today")}</button>
        <span className="spacer" />
        {kind !== "date" && <button type="button" className="btn" onClick={commit}>OK</button>}
      </div>
    </div>,
    document.body,
  );
}

function TimeSpin({ value, max, onChange }: { value: number; max: number; onChange: (v: number) => void }) {
  return (
    <input
      className="field dtp-time-input"
      type="text"
      inputMode="numeric"
      value={pad(value)}
      onChange={(e) => {
        const n = Number(e.target.value.replace(/\D/g, ""));
        if (!Number.isNaN(n)) onChange(Math.max(0, Math.min(max, n)));
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowUp") { e.preventDefault(); onChange((value + 1) % (max + 1)); }
        else if (e.key === "ArrowDown") { e.preventDefault(); onChange((value - 1 + max + 1) % (max + 1)); }
      }}
      onFocus={(e) => e.target.select()}
    />
  );
}
