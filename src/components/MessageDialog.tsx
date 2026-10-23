import { useEffect, useRef, useState, type ReactNode } from "react";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";

interface Props {
  title: string;
  message?: ReactNode;
  /** Exige digitar este texto para confirmar (operações destrutivas em produção). */
  confirmText?: string;
  /** Campo de texto livre (ex.: nome da nova tabela). */
  prompt?: { label: string; initial: string };
  danger?: boolean;
  okLabel?: string;
  /** Operação em andamento (OK desabilitado, spinner). */
  busy?: boolean;
  /** Erro devolvido pelo servidor ao confirmar; o diálogo continua aberto. */
  error?: string;
  /** Campo de prompt como senha (mascarado). */
  secret?: boolean;
  /** Campo de prompt pode ficar vazio (ex.: apagar um apelido). */
  allowEmpty?: boolean;
  /** Abre com o texto do prompt selecionado. */
  selectText?: boolean;
  onOk: (value?: string) => void;
  onCancel: () => void;
}

export function MessageDialog({ title, message, confirmText, prompt, danger, okLabel, busy, error, secret, allowEmpty, selectText, onOk, onCancel }: Props) {
  const t = useT();
  const [typed, setTyped] = useState("");
  const [value, setValue] = useState(prompt?.initial ?? "");
  const okRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    (inputRef.current ?? okRef.current)?.focus();
    if (selectText) inputRef.current?.select();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onCancel();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const canOk = !busy && (!confirmText || typed === confirmText) && (!prompt || secret || allowEmpty || value.trim() !== "");
  const ok = () => canOk && onOk(prompt ? (secret ? value : value.trim()) : undefined);

  return (
    <div className="modal-overlay" role="presentation">
      <div className="window window-small" role="alertdialog" aria-modal="true" aria-label={title}>
        <div className="window-title">
          <SmallIcon name={danger ? "warning" : "myConnections"} />
          <span>{title}</span>
          <button type="button" className="window-close" aria-label={t("tab.close")} onClick={onCancel}><SmallIcon name="close" /></button>
        </div>
        <div className="msg-body">
          {message && <div className="msg-text">{message}</div>}
          {prompt && (
            <label className="msg-prompt">
              <span>{prompt.label}</span>
              <input ref={inputRef} className="field" type={secret ? "password" : "text"} autoComplete="off" value={value} onChange={(e) => setValue(e.target.value)} onKeyDown={(e) => e.key === "Enter" && ok()} spellCheck={false} />
            </label>
          )}
          {confirmText && (
            <label className="msg-prompt">
              <span>{t("msg.typeToConfirm", { name: confirmText })}</span>
              <input ref={inputRef} className="field" value={typed} onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => e.key === "Enter" && ok()} spellCheck={false} />
            </label>
          )}
          {error && <div className="msg-error" role="alert">{error}</div>}
        </div>
        <div className="window-footer">
          {busy && <span className="spinner" />}
          <span className="spacer" />
          <button ref={okRef} type="button" className={`btn btn-primary${danger ? " btn-danger" : ""}`} disabled={!canOk} onClick={ok}>{okLabel ?? "OK"}</button>
          <button type="button" className="btn" onClick={onCancel}>{t("conn.cancel")}</button>
        </div>
      </div>
    </div>
  );
}
