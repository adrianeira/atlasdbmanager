import { useEffect, useRef } from "react";
import { defaultKeymap, history, historyKeymap, indentWithTab, toggleComment } from "@codemirror/commands";
import { MSSQL, MySQL, PostgreSQL, SQLite, sql, type SQLNamespace } from "@codemirror/lang-sql";
import { bracketMatching, foldGutter, HighlightStyle, indentOnInput, indentUnit, syntaxHighlighting } from "@codemirror/language";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import {
  Decoration, drawSelection, EditorView, highlightActiveLineGutter, keymap, lineNumbers, placeholder as cmPlaceholder,
} from "@codemirror/view";
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, snippet } from "@codemirror/autocomplete";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { tags } from "@lezer/highlight";
import { onZoom, zoomIn, zoomOut, zoomReset } from "../editorZoom";
import { useOptions, type AppOptions } from "../options";
import type { AtlasCompletion } from "../sqlCompletion";

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
  "&": { height: "100%", fontSize: "calc(var(--editor-font-size, 13px) * var(--editor-zoom, 1))", backgroundColor: "var(--editor-bg, var(--c-surface))", color: "var(--editor-fg, var(--c-text))" },
  ".cm-content": { fontFamily: "var(--editor-font, var(--font-mono))", caretColor: "var(--editor-fg, var(--c-text))" },
  ".cm-scroller": { fontFamily: "var(--editor-font, var(--font-mono))", lineHeight: "1.45" },
  ".cm-gutters": { backgroundColor: "var(--c-surface-alt)", color: "var(--c-text-disabled)", border: "none", borderRight: "1px solid var(--c-border)" },
  // Número centralizado na calha inteira: a coluna de dobrar (14 px, fora do zoom) fica à direita, então o número ganha 14 px a mais à esquerda.
  ".cm-lineNumbers .cm-gutterElement": { textAlign: "center", padding: "0 3px 0 17px", minWidth: "20px" },
  ".cm-foldGutter": { width: "14px", fontSize: "var(--editor-font-size, 13px)" },
  ".cm-foldGutter .cm-gutterElement": { padding: "0", textAlign: "center" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--c-text)" },
  ".cm-mark-add": { backgroundColor: "var(--diff-add)" },
  ".cm-mark-remove": { backgroundColor: "var(--diff-remove)" },
  "&.cm-focused": { outline: "none" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--editor-selection) !important" },
  ".cm-tooltip": { border: "1px solid var(--c-border-strong)", backgroundColor: "var(--c-surface)" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--c-selected)", color: "var(--c-text)" },
  // O CodeMirror sublinha a parte digitada em cada sugestão; sem sublinhado.
  ".cm-completionMatchedText": { textDecoration: "none" },
});

export interface SqlEditorHandle {
  getText: () => string;
  getSelection: () => string;
  replaceAll: (text: string) => void;
  /** Insere no cursor (ou no lugar da seleção); `${1:padrão}` vira ponto de tabulação (Tab/Shift+Tab). */
  insertSnippet: (template: string) => void;
  /** Troca a seleção pelo texto e deixa o texto novo selecionado. */
  replaceSelection: (text: string) => void;
  /** Seleciona o trecho final do documento (para executar só ele). */
  selectLast: (text: string) => void;
  focus: () => void;
  /** Cursor e rolagem, para reconstruir o editor igual depois. */
  getViewState: () => { anchor: number; head: number; scrollTop: number };
  setViewState: (s: { anchor: number; head: number; scrollTop: number }) => void;
}

interface Props {
  value: string;
  readOnly?: boolean;
  /** Nomes para o autocomplete: tabelas (com colunas), views e funções; ver `completionNamespace`. */
  schema?: SQLNamespace;
  /** Dialeto da consulta; ausente = PostgreSQL. */
  dialect?: "sqlite" | "mssql" | "mysql";
  defaultSchema?: string;
  placeholder?: string;
  /** Linhas (a partir de 1) com o fundo marcado: `add` em verde e `remove` em vermelho (comparação de DDL). */
  lineMarks?: { add?: number[]; remove?: number[] };
  onChange?: (text: string) => void;
  onCursor?: (line: number, col: number) => void;
  onRun?: (selectionOnly: boolean) => void;
  handleRef?: (h: SqlEditorHandle | null) => void;
}

/** O que a tela Opções → Editor / Completar código controla; fica num compartimento para valer sem recriar o editor. */
function optionExtensions(o: AppOptions): Extension[] {
  return [
    o.showLineNumbers ? lineNumbers() : [],
    o.showLineNumbers ? highlightActiveLineGutter() : [],
    o.codeFolding ? foldGutter() : [],
    o.braceHighlight ? bracketMatching() : [],
    o.syntaxHighlight ? syntaxHighlighting(atlasHighlight) : [],
    o.wordWrap ? EditorView.lineWrapping : [],
    EditorState.tabSize.of(o.tabWidth),
    indentUnit.of(o.insertSpaces ? " ".repeat(o.tabWidth) : "	"),
    autocompletion({
      activateOnTyping: o.codeCompletion,
      selectOnOpen: o.autoSelectFirst,
      // O schema (ou a tabela, no caso de coluna) no canto direito de cada sugestão.
      addToOptions: [{
        position: 90,
        render: (c) => {
          const where = (c as AtlasCompletion).where;
          if (!where) return null;
          const el = document.createElement("span");
          el.className = "cm-completionWhere";
          el.textContent = where;
          return el;
        },
      }],
    }),
  ];
}

export function SqlEditor({ value, readOnly, schema, dialect, defaultSchema, placeholder, lineMarks, onChange, onCursor, onRun, handleRef }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const langConf = useRef(new Compartment());
  const marksConf = useRef(new Compartment());
  const optConf = useRef(new Compartment());
  const options = useOptions();
  const optionsRef = useRef(options);
  optionsRef.current = options;
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
      { key: "Mod-=", preventDefault: true, run: () => (zoomIn(), true) },
      { key: "Mod-+", preventDefault: true, run: () => (zoomIn(), true) },
      { key: "Mod--", preventDefault: true, run: () => (zoomOut(), true) },
      { key: "Mod-0", preventDefault: true, run: () => (zoomReset(), true) },
    ]);
    const extensions: Extension[] = [
      optConf.current.of(optionExtensions(optionsRef.current)),
      history(),
      drawSelection(),
      indentOnInput(),
      closeBrackets(),
      highlightSelectionMatches(),
      search({ top: true }),
      atlasTheme,
      langConf.current.of(sql({ dialect: dialect === "sqlite" ? SQLite : dialect === "mssql" ? MSSQL : dialect === "mysql" ? MySQL : PostgreSQL, schema, defaultSchema, upperCaseKeywords: true })),
      marksConf.current.of([]),
      runKeys,
      keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
      // Ctrl + roda do mouse: zoom do texto.
      EditorView.domEventHandlers({
        wheel: (e) => {
          if (!e.ctrlKey) return false;
          e.preventDefault();
          if (e.deltaY < 0) zoomIn();
          else if (e.deltaY > 0) zoomOut();
          return true;
        },
      }),
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
    const stopZoom = onZoom(() => v.requestMeasure());
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
      replaceSelection: (text) => {
        const { from, to } = v.state.selection.main;
        v.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from, head: from + text.length } });
      },
      selectLast: (text) => {
        const end = v.state.doc.length;
        v.dispatch({ selection: { anchor: Math.max(0, end - text.length), head: end } });
      },
      focus: () => v.focus(),
      getViewState: () => ({ anchor: v.state.selection.main.anchor, head: v.state.selection.main.head, scrollTop: v.scrollDOM.scrollTop }),
      setViewState: (s) => {
        const max = v.state.doc.length;
        v.dispatch({ selection: { anchor: Math.min(s.anchor, max), head: Math.min(s.head, max) } });
        window.requestAnimationFrame(() => { v.scrollDOM.scrollTop = s.scrollTop; });
      },
    });
    return () => {
      stopZoom();
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

  // Marcas de linha (depois do valor, que pode ter trocado o texto).
  useEffect(() => {
    const v = view.current;
    if (!v) return;
    const doc = v.state.doc;
    const ranges = [
      ...(lineMarks?.add ?? []).map((n) => ["cm-mark-add", n] as const),
      ...(lineMarks?.remove ?? []).map((n) => ["cm-mark-remove", n] as const),
    ]
      .filter(([, n]) => n >= 1 && n <= doc.lines)
      .sort((a, b) => a[1] - b[1])
      .map(([cls, n]) => Decoration.line({ class: cls }).range(doc.line(n).from));
    v.dispatch({ effects: marksConf.current.reconfigure(EditorView.decorations.of(Decoration.set(ranges))) });
  }, [lineMarks, value, readOnly]);

  useEffect(() => {
    view.current?.dispatch({ effects: optConf.current.reconfigure(optionExtensions(options)) });
  }, [options]);

  // Autocomplete acompanha o schema selecionado.
  useEffect(() => {
    view.current?.dispatch({ effects: langConf.current.reconfigure(sql({ dialect: dialect === "sqlite" ? SQLite : dialect === "mssql" ? MSSQL : dialect === "mysql" ? MySQL : PostgreSQL, schema, defaultSchema, upperCaseKeywords: true })) });
  }, [schema, defaultSchema, dialect]);

  return <div className="sql-editor" ref={host} />;
}
