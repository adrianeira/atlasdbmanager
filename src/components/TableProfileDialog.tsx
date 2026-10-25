import { useEffect, useState } from "react";
import { errorText, toAtlasError, type TableProfile } from "../backend";
import { useT, type MessageKey } from "../i18n";
import { formatBytes } from "../model";
import { SmallIcon } from "../icons";

interface Props {
  table: string;
  load: () => Promise<TableProfile>;
  onClose: () => void;
  /** Banco da tabela; ausente = PostgreSQL. */
  dialect?: "sqlite";
}

const KIND: Record<string, MessageKey> = { r: "tp.kindTable", p: "tp.kindPartitioned", f: "tp.kindForeign", m: "tp.kindMatview" };
const REPLICA: Record<string, MessageKey> = { d: "tp.replicaDefault", n: "tp.replicaNothing", f: "tp.replicaFull", i: "tp.replicaIndex" };

/** Janela "Table Profile": o resumo de uma tabela (dono, tamanho, estrutura, uso e manutenção). Só leitura. */
export function TableProfileDialog({ table, load, onClose, dialect }: Props) {
  const sqlite = dialect === "sqlite";
  const t = useT();
  const [p, setP] = useState<TableProfile | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    load().then((x) => alive && setP(x)).catch((e) => alive && setError(errorText(toAtlasError(e))));
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => { alive = false; window.removeEventListener("keydown", onKey); };
    // Uma vez ao abrir.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const n = (v: number | null | undefined) => (v === null || v === undefined ? "—" : v.toLocaleString());
  const when = (v: string | null) => (v ? v.replace(/\.\d+/, "") : t("tp.never"));
  const section = (title: MessageKey, rows: [MessageKey, string][]) => (
    <section className="tp-section" key={title}>
      <h4>{t(title)}</h4>
      <dl>
        {rows.map(([label, value]) => (
          <div key={label} className="tp-row"><dt>{t(label)}</dt><dd title={value}>{value}</dd></div>
        ))}
      </dl>
    </section>
  );

  return (
    <div className="modal-overlay" role="presentation" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal tp-dialog" role="dialog" aria-label={t("tp.title")}>
        <div className="modal-header">
          <span className="modal-title">{t("tp.title")} — {table}</span>
          <button type="button" className="modal-close" aria-label={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button>
        </div>
        <div className="modal-body tp-body">
          {!p && !error && <div className="doc-loading"><span className="spinner" />{t("common.loading")}</div>}
          {error && <div className="doc-banner is-warn">{error}</div>}
          {p && (
            <>
              {section("tp.general", sqlite ? [
                ["tp.type", t(KIND[p.kind] ?? "tp.kindTable")],
              ] : [
                ["tp.owner", p.owner],
                ["tp.type", t(KIND[p.kind] ?? "tp.kindTable")],
                ["tp.persistence", t(`tp.persist.${p.persistence}` as MessageKey)],
                ["tp.tablespace", p.tablespace],
                ["tp.rowSecurity", t(p.rowSecurity ? "tp.yes" : "tp.no")],
                ["tp.replica", t(REPLICA[p.replicaIdentity] ?? "tp.replicaDefault")],
                ["tp.comment", p.comment ?? ""],
              ])}
              {section("tp.size", [
                ["tp.estimatedRows", p.estimatedRows < 0 ? t("tp.notAnalyzed") : n(p.estimatedRows)],
                ["tp.tableSize", formatBytes(p.tableBytes)],
                ["tp.indexSize", formatBytes(p.indexBytes)],
                ["tp.totalSize", formatBytes(p.totalBytes)],
              ])}
              {section("tp.structure", [
                ["tp.columns", n(p.columns)],
                ["tp.indexes", n(p.indexes)],
                ["tp.primaryKeys", n(p.primaryKeys)],
                ["tp.foreignKeys", n(p.foreignKeys)],
                ["tp.uniques", n(p.uniques)],
                ["tp.checks", n(p.checks)],
                ["tp.triggers", n(p.triggers)],
              ])}
              {!sqlite && section("tp.activity", [
                ["tp.liveRows", n(p.liveTuples)],
                ["tp.deadRows", n(p.deadTuples)],
                ["tp.seqScans", n(p.seqScans)],
                ["tp.idxScans", n(p.indexScans)],
                ["tp.lastVacuum", when(p.lastVacuum)],
                ["tp.lastAutovacuum", when(p.lastAutovacuum)],
                ["tp.lastAnalyze", when(p.lastAnalyze)],
                ["tp.lastAutoanalyze", when(p.lastAutoanalyze)],
              ])}
            </>
          )}
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" onClick={onClose}>{t("tab.close")}</button>
        </div>
      </div>
    </div>
  );
}
