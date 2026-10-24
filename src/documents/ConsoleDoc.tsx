import { useCallback, useEffect, useRef, useState } from "react";
import { errorText, pg, toAtlasError, type ConsoleOptions } from "../backend";

interface Props {
  sessionId: string;
  database: string;
  active: boolean;
  /** `\q`: fecha a aba. */
  onQuit: () => void;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** O começo que todas as opções têm em comum (sem diferenciar maiúsculas; usa a grafia da primeira). */
function commonPrefix(list: string[]): string {
  let prefix = list[0] ?? "";
  for (const item of list.slice(1)) {
    let n = 0;
    while (n < prefix.length && n < item.length && prefix[n].toLowerCase() === item[n].toLowerCase()) n++;
    prefix = prefix.slice(0, n);
  }
  return prefix;
}

/** As opções em colunas, como o psql lista no Tab duplo. */
function columns(list: string[], width = 100): string {
  const cell = Math.max(...list.map((x) => x.length)) + 2;
  const perLine = Math.max(1, Math.floor(width / cell));
  const lines: string[] = [];
  for (let i = 0; i < list.length; i += perLine) lines.push(list.slice(i, i + perLine).map((x) => x.padEnd(cell)).join("").trimEnd());
  return `${lines.join("\n")}\n`;
}

/** Um terminal no estilo do psql: o texto sai do Rust (`console_run`) já formatado, aqui ficam a rolagem e a digitação. */
export function ConsoleDoc({ sessionId, database, active, onQuit }: Props) {
  const [db, setDb] = useState(database);
  const [prompt, setPrompt] = useState(`${database}=#`);
  const [scroll, setScroll] = useState("");
  const [buffer, setBuffer] = useState("");
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [watching, setWatching] = useState(false);
  const [options, setOptions] = useState<ConsoleOptions>({ timing: false, expanded: false, vars: {} });
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const history = useRef<string[]>([]);
  const cursor = useRef(-1);
  const lastSql = useRef("");
  const watchOn = useRef(false);
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

  // Ctrl+C interrompe o \watch (a caixa de texto fica desligada enquanto ele roda).
  useEffect(() => {
    if (!watching) return;
    const stop = (e: KeyboardEvent) => {
      if (e.key === "c" && e.ctrlKey) {
        e.preventDefault();
        watchOn.current = false;
      }
    };
    window.addEventListener("keydown", stop);
    return () => window.removeEventListener("keydown", stop);
  }, [watching]);

  const continuation = prompt.replace(/[=][#>]$/, (m) => `-${m[1]}`);

  /** `\watch`: repete o último comando a cada `secs` segundos até o Ctrl+C. */
  const watch = async (secs: number) => {
    if (!lastSql.current) {
      setScroll((s) => `${s}\\watch cannot be used with an empty query\n`);
      return;
    }
    watchOn.current = true;
    setWatching(true);
    try {
      while (watchOn.current) {
        const out = await pg.consoleRun(sessionId, db, lastSql.current, optionsRef.current);
        setScroll((s) => `${s}${new Date().toLocaleString()} (every ${secs}s)\n\n${out.text}`);
        for (let waited = 0; watchOn.current && waited < secs * 1000; waited += 100) await sleep(100);
      }
    } catch (e) {
      setScroll((s) => `${s}${errorText(toAtlasError(e))}\n`);
    } finally {
      watchOn.current = false;
      setWatching(false);
      setScroll((s) => `${s}^C\n`);
    }
  };

  const submit = async () => {
    if (busy || watching) return;
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
    setScroll((s) => `${s}${echoed}\n`);
    if (!complete) {
      setBuffer(pending);
      return;
    }
    setBuffer("");
    history.current = [...history.current.filter((h) => h !== pending), pending].slice(-200);
    const isMeta = pending.trimStart().startsWith("\\");
    if (!isMeta) lastSql.current = pending;
    setBusy(true);
    let again: number | null = null;
    try {
      const out = await pg.consoleRun(sessionId, db, pending, optionsRef.current);
      setOptions(out.options);
      setScroll((s) => `${s}${out.text}`);
      if (out.quit) { onQuit(); return; }
      if (out.switchDb) {
        setDb(out.switchDb);
        await refreshPrompt(out.switchDb);
      } else {
        await refreshPrompt(db);
      }
      again = out.watch;
    } catch (e) {
      setScroll((s) => `${s}${errorText(toAtlasError(e))}\n`);
    } finally {
      setBusy(false);
    }
    if (again !== null) await watch(again);
  };

  /** Tab: completa a palavra antes do cursor ou lista as opções. */
  const complete = async () => {
    const el = field.current;
    const pos = el?.selectionStart ?? input.length;
    const before = input.slice(0, pos);
    try {
      const c = await pg.consoleComplete(sessionId, db, before);
      if (!c.candidates.length) return;
      const typed = before.slice(c.start);
      let insert: string | null = null;
      if (c.candidates.length === 1) insert = c.candidates[0] + c.append;
      else {
        const common = commonPrefix(c.candidates);
        if (common.length > typed.length) insert = common;
      }
      if (insert === null) {
        // Sem como avançar: mostra o que dá para escolher, como o psql.
        setScroll((s) => `${s}${buffer ? continuation : prompt} ${input}\n${columns(c.candidates)}`);
        return;
      }
      const next = input.slice(0, c.start) + insert + input.slice(pos);
      setInput(next);
      const caret = c.start + insert.length;
      requestAnimationFrame(() => field.current?.setSelectionRange(caret, caret));
    } catch { /* sem opções */ }
  };

  const onKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    } else if (e.key === "Tab" && !e.shiftKey) {
      e.preventDefault();
      void complete();
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
            disabled={busy || watching}
            aria-label="console"
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={onKey}
          />
        </div>
      </div>
    </div>
  );
}
