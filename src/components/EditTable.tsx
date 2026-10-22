import type { ReactNode } from "react";
import { SmallIcon } from "../icons";

/** Coluna de uma grade de edição do designer (Fields, Indexes, Foreign Keys...). */
export interface EditColumn<T> {
  key: keyof T & string;
  label: string;
  width: number;
  kind: "text" | "check" | "select" | "number" | "key";
  options?: string[];
  /** Combobox editável: aceita texto livre além das opções. */
  free?: boolean;
}

interface Props<T> {
  columns: EditColumn<T>[];
  rows: T[];
  selected: number;
  onSelect: (i: number) => void;
  onChange: (i: number, key: keyof T & string, value: T[keyof T]) => void;
  readOnly?: boolean;
  footer?: ReactNode;
}

export function EditTable<T>({ columns, rows, selected, onSelect, onChange, readOnly }: Props<T>) {
  // Linha vazia "fantasma" como na referência quando não há itens.
  const display = rows.length ? rows : null;
  return (
    <div className="et-wrap">
      <table className="et">
        <colgroup>
          <col style={{ width: 10 }} />
          {columns.map((c) => <col key={c.key} style={{ width: c.width }} />)}
        </colgroup>
        <thead>
          <tr>
            <th />
            {columns.map((c, i) => <th key={c.key} className={i === 0 ? "is-first" : undefined}>{c.label}</th>)}
          </tr>
        </thead>
        <tbody>
          {display ? (
            display.map((r, ri) => (
              // Linhas zebradas, como o grid de dados e a referência.
              <tr key={ri} className={`${ri % 2 === 1 ? "is-alt" : ""}${ri === selected ? " is-selected" : ""}`.trim() || undefined} onMouseDown={() => onSelect(ri)}>
                <td className="et-rowhead">{ri === selected ? "▸" : ""}</td>
                {columns.map((c, ci) => {
                  const v = r[c.key];
                  const cellCls = ci === 0 && ri === selected ? "et-td is-primary" : "et-td";
                  if (c.kind === "check") {
                    return (
                      <td key={c.key} className="et-td et-center">
                        <input type="checkbox" checked={!!v} disabled={readOnly} onChange={(e) => onChange(ri, c.key, e.target.checked as T[keyof T])} />
                      </td>
                    );
                  }
                  if (c.kind === "key") {
                    return <td key={c.key} className="et-td">{v ? <span className="et-key"><SmallIcon name="key" />{String(v)}</span> : null}</td>;
                  }
                  if (c.kind === "select" && !c.free) {
                    // Lista fixa. Um valor atual fora da lista (ex.: tipo de array/enum/domínio lido
                    // do servidor) entra como opção para não ser trocado em silêncio pela primeira.
                    const cur = String(v ?? "");
                    const opts = c.options ?? [];
                    return (
                      <td key={c.key} className={cellCls}>
                        <select className="et-input" value={cur} disabled={readOnly} onChange={(e) => onChange(ri, c.key, e.target.value as T[keyof T])}>
                          {!opts.includes(cur) && <option value={cur}>{cur}</option>}
                          {opts.map((o) => <option key={o} value={o}>{o}</option>)}
                        </select>
                      </td>
                    );
                  }
                  const listId = c.kind === "select" ? `et-${c.key}` : undefined;
                  return (
                    <td key={c.key} className={cellCls}>
                      <input
                        className={`et-input${c.kind === "number" ? " is-num" : ""}`}
                        value={String(v ?? "")}
                        list={listId}
                        readOnly={readOnly}
                        spellCheck={false}
                        onChange={(e) => onChange(ri, c.key, (c.kind === "number" ? Number(e.target.value) || 0 : e.target.value) as T[keyof T])}
                      />
                      {listId && (
                        <datalist id={listId}>
                          {(c.options ?? []).map((o) => <option key={o} value={o} />)}
                        </datalist>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))
          ) : (
            <tr className="is-placeholder">
              <td className="et-rowhead" />
              {columns.map((c, i) => <td key={c.key} className={i === 0 ? "et-td is-ghost" : "et-td"}>{c.kind === "check" ? <input type="checkbox" disabled /> : null}</td>)}
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
