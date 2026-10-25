import { useEffect, useState } from "react";
import { copyToClipboard } from "../clipboard";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import type { ShareOptions } from "../shareUri";

interface Props {
  /** Nome do banco (ou da conexão) que está sendo compartilhado e o que ele é. */
  name: string;
  kind: "database" | "connection";
  /** O endereço para as opções escolhidas. */
  uriFor: (opts: ShareOptions) => string;
  onClose: () => void;
}

/** "Compartilhar": mostra o endereço que descreve a conexão ou o banco, para copiar e passar a outra pessoa. */
export function ShareDialog({ name, kind, uriFor, onClose }: Props) {
  const t = useT();
  const [opts, setOpts] = useState<ShareOptions>({ hosts: true, password: false });
  const [copied, setCopied] = useState(false);
  const uri = uriFor(opts);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const copy = async () => {
    try {
      await copyToClipboard(uri);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch { /* sem área de transferência: o texto continua selecionável */ }
  };

  return (
    <div className="modal-overlay" role="presentation" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal share-dialog" role="dialog" aria-label={t("share.title")}>
        <div className="modal-header">
          <span className="modal-title"><SmallIcon name="link" /> {t("share.title")}</span>
          <button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button>
        </div>
        <div className="modal-body">
          <div className="share-head">
            <SmallIcon name={kind === "database" ? "database" : "connection"} size={48} />
            <div>
              <div className="share-name">{name}</div>
              <div className="muted">{t(kind === "database" ? "share.database" : "share.connection")}</div>
            </div>
          </div>
          <div>{t("share.intro", { name })}</div>
          <textarea className="field share-uri" readOnly spellCheck={false} value={uri} onFocus={(e) => e.currentTarget.select()} />
          <div className="share-foot">
            <div className="share-opts">
              <label className="check-row"><input type="checkbox" checked={opts.hosts} onChange={(e) => setOpts({ ...opts, hosts: e.target.checked })} /><span>{t("share.hosts")}</span></label>
              <label className="check-row">
                <input type="checkbox" checked={opts.password} onChange={(e) => setOpts({ ...opts, password: e.target.checked })} />
                <span>{t("share.password")}</span>
                <span className="share-info" title={t("share.passwordHint")} aria-label={t("share.passwordHint")}>i</span>
              </label>
            </div>
            <button type="button" className="btn share-copy" onClick={() => void copy()}>{copied ? t("share.copied") : t("share.copy")}</button>
          </div>
        </div>
      </div>
    </div>
  );
}
