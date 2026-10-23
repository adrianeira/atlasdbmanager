import { useEffect, useRef } from "react";
import { defaultKeymap, history, historyKeymap, indentWithTab, toggleComment } from "@codemirror/commands";
import { PostgreSQL, sql } from "@codemirror/lang-sql";
import { bracketMatching, foldGutter, HighlightStyle, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import {
  drawSelection, EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers, placeholder as cmPlaceholder,
} from "@codemirror/view";
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, snippet } from "@codemirror/autocomplete";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { tags } from "@lezer/highlight";

/* Cores de sintaxe próprias, legíveis nos temas claro e escuro (via variáveis CSS). */
const atlasHighlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.operatorKeyword], color: "var(--sql-keyword)" },
  { tag: [tags.typeName, tags.standard(tags.name)], color: "var(--sql-type)" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--sql-string)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--sql-number)" },
  { tag: [tags.lineComment, tags.blockComment], color: "var(--sql-comment)", fontStyle: "italic" },
  { tag: tags.special(tags.name), color: "var(--sql-quoted)" },
]);

const atlasTheme = EditorView.theme({
  "&": { height: "100%", fontSize: "var(--editor-font-size, 13px)", backgroundColor: "var(--c-surface)", color: "var(--c-text)" },
  ".cm-content": { fontFamily: "var(--font-mono)", caretColor: "var(--c-text)" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.45" },
  ".cm-gutters": { backgroundColor: "var(--c-surface-alt)", color: "var(--c-text-disabled)", border: "none", borderRight: "1px solid var(--c-border)" },
  ".cm-activeLine": { backgroundColor: "var(--editor-active-line)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--c-text)" },
  "&.cm-focused": { outline: "none" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--editor-selection) !important" },
  ".cm-tooltip": { border: "1px solid var(--c-border-strong)", backgroundColor: "var(--c-surface)" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--c-selected)", color: "var(--c-text)" },
});

export interface SqlEditorHandle {
  getText: () => string;
  getSelection: () => string;
  replaceAll: (text: string) => void;
  /** Insere no cursor (ou no lugar da seleção); `${1:padrão}` vira ponto de tabulação (Tab/Shift+Tab). */
  insertSnippet: (template: string) => void;
  /** Seleciona o trecho final do documento (para executar só ele). */
  selectLast: (text: string) => void;
  focus: () => void;
}

interface Props {
  value: string;
  readOnly?: boolean;
  /** tabela → colunas, para o autocomplete. */
  schema?: Record<string, string[]>;
  defaultSchema?: string;
  placeholder?: string;
  onChange?: (text: string) => void;
  onCursor?: (line: number, col: number) => void;
  onRun?: (selectionOnly: boolean) => void;
  handleRef?: (h: SqlEditorHandle | null) => void;
}

export function SqlEditor({ value, readOnly, schema, defaultSchema, placeholder, onChange, onCursor, onRun, handleRef }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const langConf = useRef(new Compartment());
  const cbs = useRef({ onChange, onCursor, onRun });
  cbs.current = { onChange, onCursor, onRun };

  useEffect(() => {
    if (!host.current) return;
    const runKeys = keymap.of([
      { key: "Mod-r", preventDefault: true, run: () => (cbs.current.onRun?.(false), true) },
      { key: "F5", preventDefault: true, run: () => (cbs.current.onRun?.(false), true) },
      { key: "Mod-Enter", preventDefault: true, run: () => (cbs.current.onRun?.(true), true) },
      { key: "Mod-Shift-r", preventDefault: true, run: () => (cbs.current.onRun?.(true), true) },
      { key: "Mod-/", run: toggleComment },
    ]);
    const extensions: Extension[] = [
      lineNumbers(),
      highlightActiveLineGutter(),
      foldGutter(),
      history(),
      drawSelection(),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      search({ top: true }),
      syntaxHighlighting(atlasHighlight),
      atlasTheme,
      langConf.current.of(sql({ dialect: PostgreSQL, schema, defaultSchema, upperCaseKeywords: true })),
      autocompletion({ activateOnTyping: true }),
      runKeys,
      keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
      EditorState.readOnly.of(!!readOnly),
      EditorView.editable.of(!readOnly),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) cbs.current.onChange?.(u.state.doc.toString());
        if (u.selectionSet || u.docChanged) {
          const pos = u.state.selection.main.head;
          const line = u.state.doc.lineAt(pos);
          cbs.current.onCursor?.(line.number, pos - line.from + 1);
        }
      }),
    ];
    if (placeholder) extensions.push(cmPlaceholder(placeholder));
    const v = new EditorView({ state: EditorState.create({ doc: value, extensions }), parent: host.current });
    view.current = v;
    handleRef?.({
      getText: () => v.state.doc.toString(),
      getSelection: () => {
        const { from, to } = v.state.selection.main;
        return v.state.sliceDoc(from, to);
      },
      replaceAll: (text) => v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: text } }),
      insertSnippet: (template) => {
        const { from, to } = v.state.selection.main;
        snippet(template)({ state: v.state, dispatch: v.dispatch }, null, from, to);
        v.focus();
      },
      selectLast: (text) => {
        const end = v.state.doc.length;
        v.dispatch({ selection: { anchor: Math.max(0, end - text.length), head: end } });
      },
      focus: () => v.focus(),
    });
    return () => {
      handleRef?.(null);
      v.destroy();
      view.current = null;
    };
    // O editor é criado uma vez; mudanças de texto externas usam replaceAll.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readOnly]);

  // Valor controlado de fora (ex.: SQL Preview regenerado).
  useEffect(() => {
    const v = view.current;
    if (v && readOnly && v.state.doc.toString() !== value) v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
  }, [value, readOnly]);

  // Autocomplete acompanha o schema selecionado.
  useEffect(() => {
    view.current?.dispatch({ effects: langConf.current.reconfigure(sql({ dialect: PostgreSQL, schema, defaultSchema, upperCaseKeywords: true })) });
  }, [schema, defaultSchema]);

  return <div className="sql-editor" ref={host} />;
}
