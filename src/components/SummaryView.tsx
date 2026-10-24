import { useState } from "react";
import { useT } from "../i18n";

/** Uma linha do resumo: um comando do texto executado. */
export interface SummaryRow {
  sql: string;
  /** "OK" ou a mensagem de erro do servidor. */
  message: string;
  error: boolean;
  /** Segundos até o servidor responder e segundos lendo as linhas (`null` quando vários comandos vão juntos: o servidor entrega as respostas de uma vez, então não dá para separar). */
  queryTime: number | null;
  fetchTime: number | null;
}

export interface SummaryData {
  /** Início e fim da execução (milissegundos desde 1970). */
  start: number;
  end: number;
  rows: SummaryRow[];
}

const pad = (n: number) => String(n).padStart(2, "0");
const stamp = (ms: number) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};
const secs = (n: number | null) => n === null ? "—" : `${n.toLocaleString(undefined, { minimumFractionDigits: 3, maximumFractionDigits: 3 })}s`;

export function SummaryView({ data }: { data: SummaryData }) {
  const t = useT();
  const [errorsOnly, setErrorsOnly] = useState(false);
  const failed = data.rows.filter((r) => r.error).length;
  const rows = errorsOnly ? data.rows.filter((r) => r.error) : data.rows;
  return (
    <div className="sm-view">
      <div className="sm-head">
        <div className="sm-counts">
          <span className="muted">{t("sm.processed")}</span><span>{data.rows.length}</span>
          <span className="muted">{t("sm.success")}</span><span>{data.rows.length - failed}</span>
          <span className="muted">{t("sm.error")}</span><span>{failed}</span>
        </div>
        <div className="sm-counts">
          <span className="muted">{t("sm.start")}</span><span>{stamp(data.start)}</span>
          <span className="muted">{t("sm.end")}</span><span>{stamp(data.end)}</span>
          <span className="muted">{t("sm.elapsed")}</span><span>{secs((data.end - data.start) / 1000)}</span>
        </div>
        <label className="check-row sm-only"><input type="checkbox" checked={errorsOnly} onChange={(e) => setErrorsOnly(e.target.checked)} /><span>{t("sm.errorsOnly")}</span></label>
      </div>
      <div className="sm-scroll">
        <table className="ex-stats sm-table">
          <thead><tr><th>{t("sm.query")}</th><th>{t("sm.message")}</th><th className="num">{t("sm.queryTime")}</th><th className="num">{t("sm.fetchTime")}</th></tr></thead>
          <tbody>
            {rows.map((r, i) => (
              <tr key={i} className={r.error ? "is-error" : undefined}>
                <td className="sm-sql" title={r.sql}>{r.sql.replace(/\s+/g, " ")}</td>
                <td className="sm-msg" title={r.message}>{r.message}</td>
                <td className="num">{secs(r.queryTime)}</td>
                <td className="num">{secs(r.fetchTime)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
