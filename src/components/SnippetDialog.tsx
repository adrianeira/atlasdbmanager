import { useCallback, useEffect, useMemo, useState } from "react";
import { errorText, local, toAtlasError, type Snippet } from "../backend";
import { useT } from "../i18n";
import { SmallIcon } from "../icons";
import { MessageDialog } from "./MessageDialog";

interface Props {
  /** Texto selecionado no editor: abre já como um snippet novo com este corpo. */
  initialBody?: string;
  /** Insere o corpo no cursor do editor (os marcadores `${1:padrão}` viram pontos de tabulação). */
  onInsert: (body: string) => void;
  onClose: () => void;
}

interface Draft {
  /** `null` = snippet novo ainda não gravado. */
  id: string | null;
  name: string;
  body: string;
}

export function SnippetDialog({ initialBody, onInsert, onClose }: Props) {
  const t = useT();
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  const [filter, setFilter] = useState("");
  const [draft, setDraft] = useState<Draft | null>(initialBody?.trim() ? { id: null, name: "", body: initialBody } : null);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [loaded, setLoaded] = useState(false);

  const fail = (e: unknown) => setError(errorText(toAtlasError(e)));

  const reload = useCallback(async (): Promise<Snippet[]> => {
    const list = await local.snippetsList();
    setSnippets(list);
    return list;
  }, []);

  // A listagem já traz os snippets de partida na primeira vez (o núcleo os grava uma única vez).
  useEffect(() => {
    void (async () => {
      try {
        const list = await reload();
        setDraft((d) => d ?? (list[0] ? { id: list[0].id, name: list[0].name, body: list[0].body } : null));
      } catch (e) {
        fail(e);
      } finally {
        setLoaded(true);
      }
    })();
  }, [reload]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !confirmDelete) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, confirmDelete]);

  const saved = draft?.id ? snippets.find((s) => s.id === draft.id) : undefined;
  const dirty = !!draft && (draft.id === null ? draft.name.trim() !== "" || draft.body !== "" : !saved || saved.name !== draft.name || saved.body !== draft.body);
  const visible = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return f ? snippets.filter((s) => s.name.toLowerCase().includes(f) || s.body.toLowerCase().includes(f)) : snippets;
  }, [snippets, filter]);

  const select = (s: Snippet) => { setError(null); setDraft({ id: s.id, name: s.name, body: s.body }); };
  const create = () => { setError(null); setDraft({ id: null, name: "", body: "" }); };

  const save = async () => {
    if (!draft) return;
    if (!draft.name.trim()) return setError(t("snip.needName"));
    try {
      const id = await local.snippetSave(draft.id, draft.name, draft.body);
      await reload();
      setDraft({ ...draft, id, name: draft.name.trim() });
      setError(null);
    } catch (e) {
      fail(e);
    }
  };

  const remove = async () => {
    if (!draft?.id) return;
    try {
      await local.snippetDelete(draft.id);
      const list = await reload();
      setDraft(list[0] ? { id: list[0].id, name: list[0].name, body: list[0].body } : null);
      setError(null);
    } catch (e) {
      fail(e);
    }
  };

  return (
    <div className="modal-overlay" role="presentation" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal snip-dialog" role="dialog" aria-label={t("snip.title")}>
        <div className="modal-header"><span className="modal-title">{t("snip.title")}</span><button type="button" className="modal-close" aria-label={t("tab.close")} title={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button></div>
        <div className="snip-body">
          <div className="snip-side">
            <div className="snip-side-bar">
              <input className="field" value={filter} placeholder={t("snip.filter")} onChange={(e) => setFilter(e.target.value)} spellCheck={false} />
              <button type="button" className="btn" title={t("snip.new")} onClick={create}><SmallIcon name="addRow" /></button>
            </div>
            <div className="snip-list" role="listbox" aria-label={t("snip.title")}>
              {visible.map((s) => (
                <div
                  key={s.id}
                  role="option"
                  aria-selected={s.id === draft?.id}
                  className={`snip-item${s.id === draft?.id ? " is-selected" : ""}`}
                  onClick={() => select(s)}
                  onDoubleClick={() => onInsert(s.body)}
                  title={s.body}
                >
                  {s.name}
                </div>
              ))}
              {loaded && visible.length === 0 && <div className="hist-empty">{t(filter ? "snip.noMatch" : "snip.empty")}</div>}
            </div>
          </div>
          <div className="snip-edit">
            {draft ? (
              <>
                <input className="field snip-name" value={draft.name} placeholder={t("snip.name")} onChange={(e) => setDraft({ ...draft, name: e.target.value })} spellCheck={false} aria-label={t("snip.name")} />
                <textarea
                  className="snip-text"
                  value={draft.body}
                  placeholder={t("snip.bodyPlaceholder")}
                  onChange={(e) => setDraft({ ...draft, body: e.target.value })}
                  spellCheck={false}
                  aria-label={t("snip.body")}
                />
                <div className="muted snip-hint">{t("snip.hint")}</div>
              </>
            ) : (
              <div className="hist-empty">{t("snip.pick")}</div>
            )}
          </div>
        </div>
        {error && <div className="doc-banner is-warn">{error}</div>}
        <div className="modal-footer hist-footer">
          <button type="button" className="btn" disabled={!draft?.id} onClick={() => setConfirmDelete(true)}>{t("snip.delete")}</button>
          <span className="spacer" />
          <button type="button" className="btn" disabled={!draft || !dirty} onClick={() => void save()}>{t("snip.save")}</button>
          <button type="button" className="btn btn-primary" disabled={!draft || !draft.body.trim()} onClick={() => draft && onInsert(draft.body)}>{t("snip.insert")}</button>
          <button type="button" className="btn" onClick={onClose}>{t("tab.close")}</button>
        </div>
      </div>
      {confirmDelete && draft?.id && (
        <MessageDialog
          title={t("snip.delete")}
          message={t("snip.deleteMessage", { name: draft.name })}
          danger
          okLabel={t("snip.delete")}
          onOk={() => { setConfirmDelete(false); void remove(); }}
          onCancel={() => setConfirmDelete(false)}
        />
      )}
    </div>
  );
}
