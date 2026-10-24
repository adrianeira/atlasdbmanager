import { useEffect, useMemo, useRef, useState } from "react";
import { errorText, toAtlasError } from "../backend";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import type { CellValue, ForeignKeyDef } from "../model";
import { DataGrid, typeKind, type GridColumn } from "./DataGrid";

/** De onde o lookup lê a tabela referenciada (banco real ou dados de demonstração). */
export interface FkLookupSource {
  /** Colunas da tabela referenciada (nome + tipo). */
  columns: (schema: string, table: string) => Promise<{ name: string; type: string }[]>;
  /** Primeiras `limit` linhas com as colunas pedidas, ordenadas pelas colunas da chave. */
  rows: (schema: string, table: string, columns: string[], orderBy: string[], limit: number) => Promise<CellValue[][]>;
  /** Descrição de cada chave pedida (só com banco real); sem ela o grid não troca a chave pelo valor. */
  labels?: (schema: string, table: string, keyColumn: string, keys: string[]) => Promise<{ labelColumn: string | null; labels: [string, string][] }>;
}

const LIMIT = 1000;

/** Colunas extras escolhidas por FK (lembradas enquanto o app estiver aberto). */
const shownByFk = new Map<string, string[]>();

interface Props {
  fk: ForeignKeyDef;
  table: string;
  /** Valores atuais das colunas da FK na linha editada (na ordem de `fk.fields`). */
  current: CellValue[];
  anchor: DOMRect;
  source: FkLookupSource;
  onPick: (values: CellValue[]) => void;
  onClose: () => void;
}

const splitFields = (f: string) => f.split(",").map((x) => x.trim()).filter(Boolean);

export function FkLookup({ fk, table, current, anchor, source, onPick, onClose }: Props) {
  const t = useT();
  const keyCols = useMemo(() => splitFields(fk.refFields), [fk.refFields]);
  const fkKey = `${fk.refSchema}.${fk.refTable}:${fk.name}`;
  const [allCols, setAllCols] = useState<{ name: string; type: string }[]>([]);
  const [extra, setExtra] = useState<string[]>(() => shownByFk.get(fkKey) ?? []);
  const [rows, setRows] = useState<CellValue[][]>([]);
  const [filter, setFilter] = useState("");
  const [panel, setPanel] = useState(false);
  const [active, setActive] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);

  const shown = useMemo(() => [...keyCols, ...extra.filter((c) => !keyCols.includes(c))], [keyCols, extra]);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const cols = allCols.length ? allCols : await source.columns(fk.refSchema, fk.refTable);
      if (!allCols.length) setAllCols(cols);
      const data = await source.rows(fk.refSchema, fk.refTable, shown, keyCols, LIMIT);
      setRows(data);
      // Seleciona a linha do valor atual, como a referência.
      const idx = data.findIndex((r) => keyCols.every((_, i) => (r[i] ?? null) === (current[i] ?? null)));
      setActive(idx >= 0 ? idx : 0);
    } catch (e) {
      setError(errorText(toAtlasError(e)));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    // Recarrega quando as colunas exibidas mudam.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown.join("|")]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    const onDown = (e: MouseEvent) => !boxRef.current?.contains(e.target as Node) && onClose();
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [onClose]);

  // Filtro sobre as linhas carregadas (texto contido em qualquer coluna exibida).
  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return rows.map((r, i) => ({ r, i }));
    return rows.map((r, i) => ({ r, i })).filter(({ r }) => r.some((v) => v !== null && v.toLowerCase().includes(q)));
  }, [rows, filter]);

  const gridCols: GridColumn[] = useMemo(
    () => shown.map((name) => {
      const type = allCols.find((c) => c.name === name)?.type ?? "";
      return { name, type: "", kind: typeKind(type) };
    }),
    [shown, allCols],
  );

  const pick = (visibleIndex = active) => {
    const row = visible[visibleIndex]?.r;
    if (!row) return;
    onPick(keyCols.map((_, i) => row[i]));
  };

  const toggleExtra = (name: string) => {
    const next = extra.includes(name) ? extra.filter((c) => c !== name) : [...extra, name];
    shownByFk.set(fkKey, next);
    setExtra(next);
  };

  // Abre abaixo da célula; se não couber, acima (tamanho da janela da referência).
  const width = 400;
  const height = 300;
  const left = Math.max(8, Math.min(anchor.left, window.innerWidth - width - 8));
  const top = anchor.bottom + height + 8 < window.innerHeight ? anchor.bottom + 2 : Math.max(8, anchor.top - height - 2);

  return (
    <div className="fk-lookup" ref={boxRef} style={{ left, top, width, height }} role="dialog" aria-label={fk.name}>
      <div className="fk-title">
        <SmallIcon name="link" />
        <span title={`${fk.name} : ${table} (${fk.fields}) - ${fk.refTable} (${fk.refFields})`}>
          {fk.name} : {table} ({fk.fields}) - {fk.refTable} ({fk.refFields})
        </span>
      </div>
      <div className="fk-toolbar">
        <button type="button" className={`icon-button${panel ? " is-active" : ""}`} title={t("fk.columns")} onClick={() => setPanel((p) => !p)}>
          <SmallIcon name="columns" />
        </button>
        <span className="spacer" />
        <button type="button" className="icon-button" title={t("data.refresh")} onClick={() => void load()}>
          <SmallIcon name="refresh" />
        </button>
        <input className="field fk-filter" placeholder={t("fk.filter")} value={filter} onChange={(e) => { setFilter(e.target.value); setActive(0); }} />
      </div>
      <div className="fk-body">
        {panel && (
          <div className="fk-cols">
            {allCols.map((c) => {
              const isKey = keyCols.includes(c.name);
              return (
                <label key={c.name} className={`check-row${isKey ? " is-disabled" : ""}`}>
                  <input type="checkbox" checked={isKey || extra.includes(c.name)} disabled={isKey} onChange={() => toggleExtra(c.name)} />
                  <span>{c.name}</span>
                </label>
              );
            })}
          </div>
        )}
        <div className="fk-grid" onDoubleClick={() => pick()} onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), pick())}>
          {error ? (
            <div className="msg-error">{error}</div>
          ) : loading && !rows.length ? (
            <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>
          ) : (
            <DataGrid columns={gridCols} rows={visible.map((v) => v.r)} readOnly activeRow={active} onActiveRow={setActive} simpleHeader />
          )}
        </div>
      </div>
      <div className="fk-footer">
        <span className="muted">{t("fk.showing", { n: LIMIT })}</span>
        <span className="spacer" />
        <button type="button" className="btn btn-primary" disabled={!visible.length} onClick={() => pick()}>OK</button>
        <button type="button" className="btn" onClick={onClose}>{t("conn.cancel")}</button>
      </div>
    </div>
  );
}
