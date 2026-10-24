import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent } from "react";
import type { MessageKey } from "../i18n";
import { useT } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";
import { formatBytes, formatInt, functionLabel, type Category, type DbObject, type NodeContext } from "../model";
import { pg } from "../backend";
import { diagramFromObjects } from "../er";
import { ErDiagram } from "./ErDiagram";
import { PopupMenu } from "./PopupMenu";

export type ViewMode = "list" | "detail" | "icons" | "er";

/** Opções do diagrama ER (menu Exibir). */
export interface ErOptions { showGrid: boolean; snapToGrid: boolean; showFkNames: boolean }

interface ActionDef {
  id: string;
  label: MessageKey;
  icon: SmallIconName;
  /** context: precisa de schema/conexão aberta; selection: 1+ objetos; single: exatamente 1. */
  needs: "context" | "selection" | "single";
  kind?: "open" | "design" | "new" | "export" | "import" | "backup" | "restore" | "extract" | "delete";
}

const ACTIONS: Record<Category, ActionDef[]> = {
  table: [
    { id: "open", label: "act.openTable", icon: "open", needs: "selection", kind: "open" },
    { id: "design", label: "act.designTable", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.newTable", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.deleteTable", icon: "delete", needs: "selection" },
    { id: "import", label: "act.importWizard", icon: "import", needs: "context", kind: "import" },
    { id: "export", label: "act.exportWizard", icon: "export", needs: "single", kind: "export" },
  ],
  view: [
    { id: "open", label: "act.openView", icon: "open", needs: "selection", kind: "open" },
    { id: "design", label: "act.designView", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.newView", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.deleteView", icon: "delete", needs: "selection" },
    { id: "export", label: "act.exportWizard", icon: "export", needs: "single", kind: "export" },
  ],
  matview: [
    { id: "open", label: "act.open", icon: "open", needs: "selection", kind: "open" },
    { id: "design", label: "act.design", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.new", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.delete", icon: "delete", needs: "selection" },
    { id: "export", label: "act.exportWizard", icon: "export", needs: "context" },
  ],
  function: [
    { id: "design", label: "act.designFunction", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.newFunction", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.deleteFunction", icon: "delete", needs: "selection" },
    { id: "execute", label: "act.executeFunction", icon: "execute", needs: "single" },
  ],
  sequence: [
    { id: "design", label: "act.design", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.new", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.delete", icon: "delete", needs: "selection" },
  ],
  type: [
    { id: "design", label: "act.design", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.new", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.delete", icon: "delete", needs: "selection" },
  ],
  extension: [
    { id: "design", label: "act.design", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.new", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.delete", icon: "delete", needs: "selection" },
  ],
  tablespace: [
    { id: "design", label: "act.design", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.new", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.delete", icon: "delete", needs: "selection" },
  ],
  foreignServer: [
    { id: "design", label: "act.design", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.new", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.delete", icon: "delete", needs: "selection" },
  ],
  user: [
    { id: "new", label: "act.newUser", icon: "new", needs: "context", kind: "new" },
    { id: "edit", label: "act.editUser", icon: "design", needs: "single", kind: "design" },
    { id: "delete", label: "act.deleteUser", icon: "delete", needs: "selection" },
    { id: "privileges", label: "act.privilegeManager", icon: "privilege", needs: "context" },
  ],
  query: [
    { id: "design", label: "act.designQuery", icon: "design", needs: "single", kind: "design" },
    { id: "new", label: "act.newQuery", icon: "new", needs: "context", kind: "new" },
    { id: "delete", label: "act.deleteQuery", icon: "delete", needs: "selection" },
  ],
  backup: [
    { id: "new", label: "act.newBackup", icon: "new", needs: "context", kind: "backup" },
    { id: "restore", label: "act.restoreBackup", icon: "open", needs: "context", kind: "restore" },
    { id: "extract", label: "act.extractSql", icon: "export", needs: "context", kind: "extract" },
    { id: "delete", label: "act.deleteBackup", icon: "delete", needs: "selection", kind: "delete" },
  ],
  automation: [
    { id: "new", label: "act.newBatchJob", icon: "new", needs: "context" },
    { id: "edit", label: "act.editBatchJob", icon: "design", needs: "single" },
    { id: "delete", label: "act.deleteBatchJob", icon: "delete", needs: "selection" },
  ],
  model: [
    { id: "new", label: "act.newModel", icon: "new", needs: "context" },
    { id: "open", label: "act.openModel", icon: "open", needs: "single" },
    { id: "delete", label: "act.deleteModel", icon: "delete", needs: "selection" },
  ],
  bi: [
    { id: "new", label: "act.newWorkspace", icon: "new", needs: "context" },
    { id: "open", label: "act.openWorkspace", icon: "open", needs: "single" },
    { id: "delete", label: "act.deleteWorkspace", icon: "delete", needs: "selection" },
  ],
};

interface Column {
  key: string;
  label: MessageKey;
  width: number;
  align?: "right";
  value: (o: DbObject) => string;
}

/** Colunas da lista de objetos de uma categoria (chave e rótulo), para "Ordenar" e "Escolher colunas" do menu Exibir. */
export function objectColumns(category: Category, lang: string, provider?: string): { key: string; label: MessageKey }[] {
  return columnsFor(category, lang, provider).map((c) => ({ key: c.key, label: c.label }));
}

function columnsFor(category: Category, lang: string, provider?: string): Column[] {
  const name: Column = { key: "name", label: "col.name", width: 248, value: (o) => o.name };
  const comment: Column = { key: "comment", label: "col.comment", width: 220, value: (o) => o.comment ?? "" };
  const owner: Column = { key: "owner", label: "col.owner", width: 120, value: (o) => o.owner ?? "" };
  const modified: Column = { key: "modified", label: "col.modified", width: 140, value: (o) => o.modified ?? "" };
  const yesNo = (b?: boolean) => (b === undefined ? "" : b ? "✓" : "");
  switch (category) {
    case "table":
      // PostgreSQL: Name, Rows, Comment (CONFIRMADO_EM_CAPTURA "Expand Table.png").
      if (provider === "postgresql") {
        return [
          name,
          { key: "rows", label: "col.rows", width: 90, align: "right", value: (o) => formatInt(o.rows, lang) },
          { key: "comment", label: "col.comment", width: 760, value: (o) => o.comment ?? "" },
        ];
      }
      return [
        name,
        { key: "rows", label: "col.rows", width: 90, align: "right", value: (o) => formatInt(o.rows, lang) },
        { key: "dataLength", label: "col.dataLength", width: 110, align: "right", value: (o) => formatBytes(o.dataLength) },
        { key: "engine", label: "col.engine", width: 90, value: () => "" },
        modified,
        comment,
      ];
    case "view":
    case "matview":
      return [name, owner, comment];
    case "function":
      return [
        { ...name, value: functionLabel },
        { key: "returnType", label: "col.returnType", width: 120, value: (o) => o.returnType ?? "" },
        { key: "language", label: "col.language", width: 100, value: (o) => o.language ?? "" },
        owner,
      ];
    case "sequence":
    case "type":
      return [name, { key: "dataType", label: "col.dataType", width: 120, value: (o) => o.dataType ?? "" }, owner];
    case "extension":
      return [
        name,
        { key: "version", label: "col.version", width: 90, value: (o) => o.version ?? "" },
        { key: "schema", label: "col.schema", width: 120, value: (o) => o.extSchema ?? "" },
        owner,
        { key: "comment", label: "col.comment", width: 420, value: (o) => o.comment ?? "" },
      ];
    case "tablespace":
      return [
        name,
        owner,
        { key: "location", label: "col.location", width: 360, value: (o) => o.location ?? "" },
        { key: "comment", label: "col.comment", width: 320, value: (o) => o.comment ?? "" },
      ];
    case "foreignServer":
      return [
        name,
        { key: "wrapper", label: "col.wrapper", width: 160, value: (o) => o.wrapper ?? "" },
        owner,
        { key: "comment", label: "col.comment", width: 420, value: (o) => o.comment ?? "" },
      ];
    case "user":
      return [
        name,
        { key: "canLogin", label: "col.canLogin", width: 90, value: (o) => yesNo(o.canLogin) },
        { key: "superuser", label: "col.superuser", width: 100, value: (o) => yesNo(o.superuser) },
      ];
    case "query":
      return [
        name,
        modified,
        { key: "size", label: "col.fileSize", width: 140, align: "right", value: (o) => formatBytes(o.dataLength) },
        { key: "modifiedUser", label: "col.modifiedUser", width: 140, value: (o) => o.modifiedUser ?? "" },
      ];
    case "backup":
      return [name, modified, { key: "size", label: "col.size", width: 100, align: "right", value: (o) => formatBytes(o.dataLength) }, comment];
    default:
      return [name, modified, comment];
  }
}

/** Colunas exibidas sem nenhuma conexão selecionada, iguais à captura de referência. */
const EMPTY_TABLE_COLUMNS: Column[] = [
  { key: "name", label: "col.name", width: 248, value: () => "" },
  { key: "rows", label: "col.rows", width: 90, align: "right", value: () => "" },
  { key: "dataLength", label: "col.dataLength", width: 110, align: "right", value: () => "" },
  { key: "engine", label: "col.engine", width: 90, value: () => "" },
  { key: "modified", label: "col.modified", width: 140, value: () => "" },
  { key: "comment", label: "col.comment", width: 140, value: () => "" },
];

export function objectsFor(category: Category, ctx: NodeContext, connectionOpen: boolean): DbObject[] | null {
  if (category === "user") return ctx.connection && connectionOpen ? ctx.connection.roles : null;
  if (category === "backup" && !ctx.schema && ctx.database) return ctx.database.backups ?? [];
  if (!ctx.schema) return null;
  return ctx.schema.objects[category] ?? [];
}

interface Props {
  category: Category;
  ctx: NodeContext;
  connectionOpen: boolean;
  viewMode: ViewMode;
  erOptions: ErOptions;
  /** Ordenação da lista (vale em todas as categorias; sem a coluna, ordena pelo nome). */
  sort: { key: string; dir: 1 | -1 };
  onSort: (s: { key: string; dir: 1 | -1 }) => void;
  /** Chaves das colunas ocultas (a primeira, o nome, nunca some). */
  hiddenColumns: string[];
  lang: string;
  selected: string[];
  onSelect: (names: string[]) => void;
  onAction: (kind: "open" | "design" | "new" | "export" | "import" | "backup" | "restore" | "extract" | "delete", objects: string[]) => void;
  /** Botão direito: `name` do objeto, ou null na área vazia (menu da pasta). */
  onContextMenu?: (name: string | null, x: number, y: number) => void;
}

export function ObjectsView({ category, ctx, connectionOpen, viewMode, erOptions, sort, onSort, hiddenColumns, lang, selected, onSelect, onAction, onContextMenu }: Props) {
  const t = useT();
  const [search, setSearch] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [newMenu, setNewMenu] = useState(false);
  const newRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!newMenu) return;
    const close = (e: MouseEvent) => !newRef.current?.contains(e.target as Node) && setNewMenu(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [newMenu]);

  // O diagrama ER só existe para tabelas de um schema escolhido; nos demais casos cai no modo Detalhe.
  const er = viewMode === "er" && category === "table" && ctx.connection && ctx.database && ctx.schema
    ? { conn: ctx.connection, db: ctx.database, schema: ctx.schema }
    : null;
  const mode: ViewMode = viewMode === "er" && !er ? "detail" : viewMode;
  const all = objectsFor(category, ctx, connectionOpen);
  const hasContext = all !== null;
  // Com conexão selecionada (mesmo sem schema) as colunas já seguem o provedor, como na referência.
  const allColumns = ctx.connection ? columnsFor(category, lang, ctx.connection.provider) : EMPTY_TABLE_COLUMNS;
  const columns = allColumns.filter((c, i) => i === 0 || !hiddenColumns.includes(c.key));
  /** Tabelas externas (foreign) ganham ícone próprio na lista. */
  const rowIcon = (o: { dataType?: string }) => (category === "table" && o.dataType === "foreign" ? "foreignTable" : category);
  const objects = useMemo(() => {
    const q = search.trim().toLowerCase();
    const list = (all ?? []).filter((o) => !q || o.name.toLowerCase().includes(q));
    const col = allColumns.find((c) => c.key === sort.key);
    const numeric = sort.key === "rows" || sort.key === "size" || sort.key === "dataLength";
    return [...list].sort((a, b) => {
      if (numeric) {
        const va = sort.key === "rows" ? a.rows ?? 0 : a.dataLength ?? 0;
        const vb = sort.key === "rows" ? b.rows ?? 0 : b.dataLength ?? 0;
        return (va - vb) * sort.dir;
      }
      const va = col ? col.value(a) : a.name;
      const vb = col ? col.value(b) : b.name;
      return va.localeCompare(vb, lang) * sort.dir;
    });
  }, [all, search, sort, allColumns, lang]);

  const toggleSort = (key: string) => onSort(sort.key === key ? { key, dir: sort.dir === 1 ? -1 : 1 } : { key, dir: 1 });

  const enabled = (a: ActionDef) => {
    if (!hasContext || !a.kind) return false;
    if (a.needs === "context") return true;
    if (a.needs === "single") return selected.length === 1;
    return selected.length > 0;
  };

  const clickRow = (e: ReactMouseEvent, name: string) => {
    // Botão direito sobre um item já selecionado mantém a seleção (como no Explorer).
    if (e.button === 2 && selected.includes(name)) return;
    if (e.ctrlKey || e.metaKey) {
      onSelect(selected.includes(name) ? selected.filter((s) => s !== name) : [...selected, name]);
    } else if (e.shiftKey && selected.length) {
      const names = objects.map((o) => o.name);
      const a = names.indexOf(selected[selected.length - 1]);
      const b = names.indexOf(name);
      onSelect(names.slice(Math.min(a, b), Math.max(a, b) + 1));
    } else {
      onSelect([name]);
    }
  };

  const openDefault = (name: string) => {
    const first = ACTIONS[category].find((a) => a.kind === "open" || a.kind === "design");
    if (first?.kind) onAction(first.kind, [name]);
  };

  const onKeyDown = (e: ReactKeyboardEvent) => {
    if (!objects.length) return;
    const names = objects.map((o) => o.name);
    const cur = names.indexOf(selected[selected.length - 1]);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = e.key === "ArrowDown" ? Math.min(cur + 1, names.length - 1) : Math.max(cur - 1, 0);
      onSelect([names[next]]);
    } else if (e.key === "Enter" && cur >= 0) {
      e.preventDefault();
      openDefault(names[cur]);
    } else if (e.key === "a" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      onSelect(names);
    }
  };

  return (
    <div className="objects-view">
      <div className="object-toolbar" role="toolbar">
        {ACTIONS[category].map((a) => {
          const on = enabled(a);
          const btn = (
            <button
              key={a.id}
              type="button"
              className="ot-button"
              disabled={!on}
              title={hasContext && !a.kind ? t("notImplemented") : t(a.label)}
              onClick={() => a.kind && onAction(a.kind, selected)}
            >
              <SmallIcon name={a.icon} />
              <span>{t(a.label)}</span>
            </button>
          );
          // "New Table ▾": Normal / Foreign (CONFIRMADO_EM_CAPTURA).
          if (category === "table" && a.id === "new" && ctx.connection?.provider === "postgresql") {
            return (
              <div key={a.id} className="split-button" ref={newRef}>
                {btn}
                <button type="button" className="ot-button split-arrow" disabled={!on} onClick={() => setNewMenu((o) => !o)} aria-label={t(a.label)}>
                  <SmallIcon name="chevronDown" size={10} />
                </button>
                {newMenu && (
                  <PopupMenu
                    commands={{}}
                    onClose={() => setNewMenu(false)}
                    items={[
                      { kind: "item", text: t("cmd.newTableNormal"), run: () => onAction("new", []) },
                      { kind: "item", text: t("cmd.newTableForeign") },
                    ]}
                  />
                )}
              </div>
            );
          }
          return btn;
        })}
        <div className="ot-spacer" />
        {searchOpen && (
          <input
            className="ot-search"
            type="search"
            autoFocus
            value={search}
            placeholder={t("act.searchObjects")}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && (setSearch(""), setSearchOpen(false))}
          />
        )}
        <button type="button" className="icon-button" title={t("act.searchObjects")} onClick={() => setSearchOpen((s) => !s)}>
          <SmallIcon name="search" />
        </button>
      </div>

      <div
        className={`object-list${er ? " is-er" : ""}${hasContext ? "" : " no-context"}`}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onMouseDown={(e) => e.target === e.currentTarget && onSelect([])}
        onContextMenu={(e) => {
          e.preventDefault();
          if (!hasContext || !onContextMenu) return;
          const item = (e.target as HTMLElement).closest("[data-name]");
          const name = item?.getAttribute("data-name") ?? null;
          if (!name) onSelect([]);
          onContextMenu(name, e.clientX, e.clientY);
        }}
      >
        {er && (
          <ErDiagram
            storageKey={`${er.conn.id}/${er.db.name}/${er.schema.name}`}
            schema={er.schema.name}
            load={() => (er.conn.live
              ? pg.schemaDiagram(er.conn.id, er.db.name, er.schema.name)
              : Promise.resolve(diagramFromObjects(er.schema.objects.table ?? [], er.schema.name)))}
            showGrid={erOptions.showGrid}
            snapToGrid={erOptions.snapToGrid}
            showFkNames={erOptions.showFkNames}
            selected={selected}
            onSelect={onSelect}
            onOpen={openDefault}
          />
        )}

        {mode === "detail" && (
          <table className="grid-table" role="grid">
            <colgroup>
              {columns.map((c) => <col key={c.key} style={{ width: c.width }} />)}
              <col />
            </colgroup>
            <thead>
              <tr>
                {columns.map((c) => (
                  <th
                    key={c.key}
                    className={`sortable${c.align === "right" ? " align-right" : ""}`}
                    aria-sort={sort.key === c.key ? (sort.dir === 1 ? "ascending" : "descending") : undefined}
                    onClick={() => hasContext && toggleSort(c.key)}
                  >
                    {sort.key === c.key && hasContext && <span className={`sort-arrow${sort.dir === -1 ? " is-desc" : ""}`} />}
                    {t(c.label)}
                  </th>
                ))}
                <th className="filler" />
              </tr>
            </thead>
            <tbody>
              {objects.map((o) => (
                <tr
                  key={o.name}
                  data-name={o.name}
                  className={selected.includes(o.name) ? "is-selected" : undefined}
                  onMouseDown={(e) => clickRow(e, o.name)}
                  onDoubleClick={() => openDefault(o.name)}
                >
                  {columns.map((c, i) => (
                    <td key={c.key} className={c.align === "right" ? "align-right" : undefined}>
                      {i === 0 ? (
                        <span className="cell-name">
                          <SmallIcon name={rowIcon(o)} />
                          <span>{c.value(o)}</span>
                        </span>
                      ) : (
                        c.value(o)
                      )}
                    </td>
                  ))}
                  <td className="filler" />
                </tr>
              ))}
            </tbody>
          </table>
        )}

        {mode === "list" && (
          <div className="list-flow">
            {objects.map((o) => (
              <div
                key={o.name}
                data-name={o.name}
                className={`list-item${selected.includes(o.name) ? " is-selected" : ""}`}
                onMouseDown={(e) => clickRow(e, o.name)}
                onDoubleClick={() => openDefault(o.name)}
              >
                <SmallIcon name={rowIcon(o)} />
                <span>{o.name}</span>
              </div>
            ))}
          </div>
        )}

        {mode === "icons" && (
          <div className="icon-flow">
            {objects.map((o) => (
              <div
                key={o.name}
                data-name={o.name}
                className={`icon-item${selected.includes(o.name) ? " is-selected" : ""}`}
                onMouseDown={(e) => clickRow(e, o.name)}
                onDoubleClick={() => openDefault(o.name)}
              >
                <SmallIcon name={category} size={32} />
                <span>{o.name}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
