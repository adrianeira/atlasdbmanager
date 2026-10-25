import { useMemo, useState } from "react";
import type { ColumnProfile, DataProfile, OrderBy, TableFilter } from "../backend";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import type { ColumnDef } from "../model";
import { quoteIdent } from "../model";
import { FilterSortDialog } from "./FilterSortDialog";

/** Valores em destaque numa coluna (clicar nas barras da distribuição); `null` é o NULL. */
export interface Spotlight {
  column: string;
  values: Set<string | null>;
}

const pct = (part: number, total: number) => (total > 0 ? Number(((part / total) * 100).toFixed(2)) : 0);
const label = (v: string | null) => (v === null ? "(Null)" : v === "" ? "(empty)" : v);

/** Quantas linhas o destaque pega nesta coluna (entre as `top` valores guardados). */
export function spotlightCount(col: ColumnProfile, sp: Spotlight | null): number {
  if (!sp || sp.column !== col.name) return 0;
  return col.top.filter((v) => sp.values.has(v.value)).reduce((a, v) => a + v.count, 0);
}

/* ---------- O que se perfila: todos os registros, ou com filtro, ordem e limite ---------- */

export interface ProfileScope {
  mode: "all" | "filter";
  filterMode: "builder" | "text";
  filterText: string;
  sortText: string;
  limited: boolean;
  limit: string;
}

export const DEFAULT_SCOPE: ProfileScope = { mode: "all", filterMode: "builder", filterText: "", sortText: "", limited: false, limit: "1000" };

/** O texto "<campo> ASC, <campo> DESC" vira a ordenação; devolve o nome da coluna que não existe, se houver. */
export function parseSort(text: string, columns: string[]): { sort: OrderBy[]; bad?: string } {
  const sort: OrderBy[] = [];
  for (const part of text.split(",").map((x) => x.trim()).filter(Boolean)) {
    const m = /^("?)(.+?)\1(?:\s+(asc|desc))?$/i.exec(part);
    const name = m?.[2] ?? part;
    if (!columns.includes(name)) return { sort, bad: name };
    sort.push({ column: name, descending: /desc/i.test(m?.[3] ?? "") });
  }
  return { sort };
}

const sortText = (sort: OrderBy[]) => sort.map((o) => `${quoteIdent(o.column)} ${o.descending ? "DESC" : "ASC"}`).join(", ");

interface SetupProps {
  columns: ColumnDef[];
  scope: ProfileScope;
  onScope: (s: ProfileScope) => void;
  busy: boolean;
  error: string | null;
  onStart: () => void;
  /** Aberto pelo botão Settings com o perfil na tela: só o filtro, a ordem e o limite, e o botão vira "Apply Data Settings". */
  settings?: boolean;
}

/** O painel que aparece ao clicar em Data Profiling: o que perfilar e o botão de começar. */
export function ProfileSetup({ columns, scope, onScope, busy, error, onStart, settings }: SetupProps) {
  const t = useT();
  const [editor, setEditor] = useState(false);
  const set = (p: Partial<ProfileScope>) => onScope({ ...scope, ...p });
  const asFilter = scope.filterText.trim() ? { expr: scope.filterText } : null;
  return (
    <div className="pf-setup">
      {!settings && (
        <div className="pf-radios">
          <label className="check-row"><input type="radio" name="pf-mode" checked={scope.mode === "all"} onChange={() => set({ mode: "all" })} /><span>{t("pf.allRecords")}</span></label>
          <label className="check-row"><input type="radio" name="pf-mode" checked={scope.mode === "filter"} onChange={() => set({ mode: "filter" })} /><span>{t("pf.addFilter")}</span></label>
        </div>
      )}
      {!settings && scope.mode === "all" ? (
        <div className="muted">({t("pf.startAll")})</div>
      ) : (
        <div className="pf-filter">
          <div className="pf-line">
            <strong>{t("pf.filter")}</strong>
            <span className="spacer" />
            <label className="check-row"><input type="radio" name="pf-fmode" checked={scope.filterMode === "builder"} onChange={() => set({ filterMode: "builder" })} /><span>{t("pf.builder")}</span></label>
            <label className="check-row"><input type="radio" name="pf-fmode" checked={scope.filterMode === "text"} onChange={() => set({ filterMode: "text" })} /><span>{t("pf.text")}</span></label>
          </div>
          {scope.filterMode === "builder" ? (
            <div className="pf-line">
              <button type="button" className="btn" onClick={() => setEditor(true)}>{t("pf.editFilter")}</button>
              <span className="muted pf-summary">{scope.filterText.trim() || t("pf.clickPlus")}{scope.sortText.trim() ? ` · ${t("pf.sortBy")}: ${scope.sortText}` : ""}</span>
            </div>
          ) : (
            <>
              <textarea className="field pf-text" rows={2} spellCheck={false} placeholder="<Field> = <Value> AND <Field> = <Value>" value={scope.filterText} onChange={(e) => set({ filterText: e.target.value })} />
              <div className="pf-line">
                <strong>{t("pf.sortBy")}</strong>
                <input className="field pf-sort" spellCheck={false} placeholder="<Field> ASC, <Field> DESC" value={scope.sortText} onChange={(e) => set({ sortText: e.target.value })} />
              </div>
            </>
          )}
          <div className="pf-line">
            <strong>{t("pf.records")}</strong>
            <select className="field" value={scope.limited ? "first" : "none"} onChange={(e) => set({ limited: e.target.value === "first" })}>
              <option value="none">{t("pf.noLimit")}</option>
              <option value="first">{t("pf.first")}</option>
            </select>
            {scope.limited && <input className="field pf-limit" inputMode="numeric" value={scope.limit} onChange={(e) => set({ limit: e.target.value.replace(/\D/g, "") })} />}
          </div>
        </div>
      )}
      {error && <div className="form-note form-note-warn">{error}</div>}
      <div><button type="button" className="btn btn-primary" disabled={busy} onClick={onStart}>{busy ? t("pf.profiling") : t(settings ? "pf.apply" : "pf.start")}</button></div>
      {editor && (
        <FilterSortDialog
          columns={columns}
          initialFilter={asFilter}
          initialSort={parseSort(scope.sortText, columns.map((c) => c.name)).sort}
          onApply={(f: TableFilter | null, s: OrderBy[]) => { onScope({ ...scope, filterText: f?.expr ?? "", sortText: sortText(s) }); setEditor(false); }}
          onClose={() => setEditor(false)}
        />
      )}
    </div>
  );
}

/* ---------- Resumo de cada coluna, em cima do cabeçalho do grid ---------- */

export const PROFILE_HEADER_HEIGHT = 124;
const SPOT = "#d98be4";
const BAR = "#00a9c5";

/** O resumo que o grid mostra sob o nome de uma coluna: nulos, distribuição dos valores e distintos. */
export function ProfileCell({ col, spotlight }: { col: ColumnProfile; spotlight: Spotlight | null }) {
  const t = useT();
  const notNull = col.rows - col.nulls;
  const spot = spotlightCount(col, spotlight);
  const spotted = spotlight?.column === col.name && spotlight.values.size > 0;
  const max = Math.max(1, ...col.top.map((v) => v.count));
  const bars = col.top.slice(0, 24);
  const tip = [
    col.distinct !== null ? t("pf.distinctUnique", { d: col.distinct, u: col.unique ?? 0 }) : "",
    "",
    `${t("pf.notNull")} ${pct(notNull, col.rows)}% (${t("pf.count")}: ${notNull})`,
    `${t("pf.null")} ${pct(col.nulls, col.rows)}% (${t("pf.count")}: ${col.nulls})`,
  ].join(String.fromCharCode(10));
  return (
    <div className="pf-cell" title={tip}>
      <div className="pf-meter">
        {spotted
          ? <><span style={{ width: `${pct(spot, col.rows)}%`, background: SPOT }} /><span style={{ width: `${100 - pct(spot, col.rows)}%`, background: "#b8e6f0" }} /></>
          : <><span style={{ width: `${pct(notNull, col.rows)}%`, background: "#66ccee" }} /><span style={{ width: `${pct(col.nulls, col.rows)}%`, background: "#b9b9b9" }} /></>}
      </div>
      <div className="pf-legend"><i style={{ background: spotted ? SPOT : "#33bbee" }} />{spotted ? t("pf.spotlighted", { n: spot }) : t("pf.notNull")}<b>{pct(spotted ? spot : notNull, col.rows)}%</b></div>
      <div className="pf-legend"><i style={{ background: spotted ? "#66ddee" : "#b0b0b0" }} />{spotted ? t("pf.other") : t("pf.null")}<b>{pct(spotted ? col.rows - spot : col.nulls, col.rows)}%</b></div>
      <div className="pf-mini">
        {bars.map((v, i) => <span key={i} style={{ height: `${Math.max(6, (v.count / max) * 100)}%`, background: spotlight?.column === col.name && spotlight.values.has(v.value) ? SPOT : BAR }} />)}
      </div>
      <div className="pf-foot" style={spotted ? { color: SPOT } : undefined}>
        {spotted
          ? [...spotlight!.values].map(label).join(", ")
          : col.distinct !== null ? t("pf.distinctUnique", { d: col.distinct, u: col.unique ?? 0 }) : "—"}
      </div>
    </div>
  );
}

/* ---------- Estatísticas da coluna selecionada e a distribuição dos valores ---------- */

interface StatsProps {
  col: ColumnProfile;
  spotlight: Spotlight | null;
  onToggle: (value: string | null, additive: boolean) => void;
}

/** "Column Statistics" e "Value Distribution" da coluna selecionada; clicar numa barra põe o valor em destaque (Ctrl soma). */
export function ColumnStatistics({ col, spotlight, onToggle }: StatsProps) {
  const t = useT();
  const notNull = col.rows - col.nulls;
  const rows: [string, string][] = useMemo(() => [
    [t("pf.count"), String(col.rows)],
    [t("pf.null"), `${col.nulls}  ${pct(col.nulls, col.rows)}%`],
    [t("pf.notNull"), `${notNull}  ${pct(notNull, col.rows)}%`],
    [t("pf.distinct"), col.distinct === null ? "—" : String(col.distinct)],
    [t("pf.unique"), col.unique === null ? "—" : String(col.unique)],
    [t("pf.min"), col.min === null ? "—" : col.min],
    [t("pf.max"), col.max === null ? "—" : col.max],
  ], [col, notNull, t]);
  const max = Math.max(1, ...col.top.map((v) => v.count));
  const here = spotlight?.column === col.name && spotlight.values.size > 0;
  return (
    <div className="pf-stats">
      <div className="pf-stats-left">
        <div className="pf-tab">{t("pf.columnStatistics")}</div>
        <div className="pf-colname"><SmallIcon name="table" /> {col.name}</div>
        {here && (
          <div className="pf-spot-box">
            <div>{t("pf.spotField")}: {col.name}</div>
            <div>{t("pf.spotValues")}: {[...spotlight!.values].map(label).join(", ")}</div>
          </div>
        )}
        <table className="pf-table">
          <tbody>{rows.map(([k, v]) => <tr key={k}><td>{k}</td><td>{v}</td></tr>)}</tbody>
        </table>
      </div>
      <div className="pf-stats-right">
        <div className="pf-dist-title">{t("pf.valueDistribution")}</div>
        <div className="pf-dist">
          <div className="pf-axis"><span>{max}</span><span>0</span></div>
          <div className="pf-bars">
            {col.top.length === 0 && <span className="muted">{t("pf.noDistribution")}</span>}
            {col.top.map((v, i) => {
              const on = here && spotlight!.values.has(v.value);
              return (
                <button key={i} type="button" className="pf-bar" title={`${label(v.value)}: ${v.count}`} onClick={(e) => onToggle(v.value, e.ctrlKey || e.metaKey)}>
                  <span className="pf-bar-fill" style={{ height: `${Math.max(2, (v.count / max) * 100)}%`, background: on ? SPOT : BAR }} />
                  {col.top.length <= 30 && <span className="pf-bar-label" style={on ? { color: SPOT } : undefined}>{label(v.value).slice(0, 4)}</span>}
                </button>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

export type { ColumnProfile, DataProfile };
