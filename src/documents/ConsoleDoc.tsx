import { useCallback, useEffect, useRef, useState } from "react";
import { errorText, pg, toAtlasError, type ConsoleOptions } from "../backend";

interface Props {
  sessionId: string;
  database: string;
  active: boolean;
  /** `\q`: fecha a aba. */
  onQuit: () => void;
}

/** Um terminal no estilo do psql: o texto sai do Rust (`console_run`) já formatado, aqui ficam a rolagem e a digitação. */
export function ConsoleDoc({ sessionId, database, active, onQuit }: Props) {
  const [db, setDb] = useState(database);
  const [prompt, setPrompt] = useState(`${database}=#`);
  const [scroll, setScroll] = useState("");
  const [buffer, setBuffer] = useState("");
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [options, setOptions] = useState<ConsoleOptions>({ timing: false, expanded: false });
  const history = useRef<string[]>([]);
  const cursor = useRef(-1);
  const field = useRef<HTMLTextAreaElement>(null);
  const body = useRef<HTMLDivElement>(null);

  const refreshPrompt = useCallback(async (name: string) => {
    try {
      setPrompt(await pg.consolePrompt(sessionId, name));
    } catch {
      setPrompt(`${name}=#`);
    }
  }, [sessionId]);

  useEffect(() => { void refreshPrompt(database); }, [database, refreshPrompt]);
  useEffect(() => { if (active) field.current?.focus(); }, [active, busy]);
  useEffect(() => { if (body.current) body.current.scrollTop = body.current.scrollHeight; }, [scroll, buffer, input]);

  const continuation = prompt.replace(/[=][#>]$/, (m) => `-${m[1]}`);

  const submit = async () => {
    if (busy) return;
    const typed = input;
    const lines = typed.split("\n");
    const echoed = lines.map((l, i) => `${i === 0 && !buffer ? prompt : continuation} ${l}`).join("\n");
    const pending = buffer ? `${buffer}\n${typed}` : typed;
    setInput("");
    cursor.current = -1;
    if (!pending.trim()) {
      setScroll((s) => `${s}${echoed}\n`);
      setBuffer("");
      return;
    }
    let complete = false;
    try {
      complete = await pg.consoleIsComplete(pending);
    } catch { /* trata como incompleto */ }
    if (!complete) {
      setScroll((s) => `${s}${echoed}\n`);
      setBuffer(pending);
      return;
    }
    setScroll((s) => `${s}${echoed}\n`);
    setBuffer("");
    history.current = [...history.current.filter((h) => h !== pending), pending].slice(-200);
    setBusy(true);
    try {
      const out = await pg.consoleRun(sessionId, db, pending, options);
      setOptions(out.options);
      setScroll((s) => `${s}${out.text}`);
      if (out.quit) { onQuit(); return; }
      if (out.switchDb) {
        setDb(out.switchDb);
        await refreshPrompt(out.switchDb);
      } else {
        await refreshPrompt(db);
      }
    } catch (e) {
      setScroll((s) => `${s}${errorText(toAtlasError(e))}\n`);
    } finally {
      setBusy(false);
    }
  };

  const onKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    } else if (e.key === "c" && e.ctrlKey && !e.shiftKey && field.current && field.current.selectionStart === field.current.selectionEnd) {
      // Ctrl+C sem seleção descarta o que está sendo digitado, como no psql.
      e.preventDefault();
      setScroll((s) => `${s}${buffer ? continuation : prompt} ${input}^C\n`);
      setInput("");
      setBuffer("");
    } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !input.includes("\n")) {
      e.preventDefault();
      const list = history.current;
      if (!list.length) return;
      cursor.current = e.key === "ArrowUp" ? Math.min(cursor.current + 1, list.length - 1) : Math.max(cursor.current - 1, -1);
      setInput(cursor.current < 0 ? "" : list[list.length - 1 - cursor.current]);
    }
  };

  return (
    <div className="con-doc" onMouseDown={(e) => { if ((e.target as HTMLElement).tagName !== "TEXTAREA" && !window.getSelection()?.toString()) setTimeout(() => field.current?.focus(), 0); }}>
      <div className="con-body" ref={body}>
        <pre className="con-out">{scroll}</pre>
        <div className="con-line">
          <span className="con-prompt">{buffer ? continuation : prompt}</span>
          <textarea
            ref={field}
            className="con-input"
            value={input}
            rows={Math.max(1, input.split("\n").length)}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            disabled={busy}
            aria-label="console"
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={onKey}
          />
        </div>
      </div>
    </div>
  );
}
