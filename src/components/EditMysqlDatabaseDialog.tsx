import { useEffect, useMemo, useState } from "react";
import type { DatabaseDetail } from "../backend";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import { Dropdown } from "./Dropdown";

interface Props {
  detail: DatabaseDetail;
  /** Collations do servidor com o charset de cada uma: [collation, charset]. */
  collations: [string, string][];
  onApply: (charset: string, collation: string) => void;
  onClose: () => void;
}

/** "Editar banco de dados" do MySQL: conjunto de caracteres e collation (o MySQL não renomeia banco). */
export function EditMysqlDatabaseDialog({ detail, collations, onApply, onClose }: Props) {
  const t = useT();
  const [charset, setCharset] = useState(detail.encoding);
  const [collation, setCollation] = useState(detail.collate);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const charsets = useMemo(() => [...new Set(collations.map(([, c]) => c))].sort().map((c) => ({ value: c, label: c })), [collations]);
  const collationItems = useMemo(() => collations.filter(([, c]) => c === charset).map(([n]) => ({ value: n, label: n })), [collations, charset]);
  const changed = charset !== detail.encoding || collation !== detail.collate;

  const pickCharset = (c: string) => {
    setCharset(c);
    // A collation tem de ser do conjunto escolhido: vale a padrão dele (a primeira da lista, que o servidor marca como padrão).
    const first = collations.find(([, cs]) => cs === c);
    setCollation(first ? first[0] : "");
  };

  return (
    <div className="modal-overlay" role="presentation">
      <div className="modal ed-dialog" role="dialog" aria-label={t("cmd.editDatabase")}>
        <div className="modal-header">
          <span className="modal-title">{t("cmd.editDatabase").replace(/\.\.\.$/, "")} — {detail.name}</span>
          <button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button>
        </div>
        <div className="modal-body ed-body">
          <div className="ed-row"><label>{t("ed.dbName")}</label><input className="field" value={detail.name} disabled readOnly /></div>
          <div className="ed-row">
            <label>{t("ed.encoding")}</label>
            <Dropdown className="ed-dd" ariaLabel={t("ed.encoding")} value={charset} items={charsets} onChange={pickCharset} />
          </div>
          <div className="ed-row">
            <label>{t("ed.collate")}</label>
            <Dropdown className="ed-dd" ariaLabel={t("ed.collate")} value={collation} items={collationItems} onChange={setCollation} />
          </div>
          <div className="muted ed-note">{t("ed.mysqlNote")}</div>
        </div>
        <div className="modal-footer">
          <span className="spacer" />
          <button type="button" className="btn btn-primary" disabled={!changed || !collation} onClick={() => onApply(charset, collation)}>{t("common.apply")}</button>
          <button type="button" className="btn" onClick={onClose}>{t("common.cancel")}</button>
        </div>
      </div>
    </div>
  );
}
