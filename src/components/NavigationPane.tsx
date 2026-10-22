import { useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { useT } from "../i18n";
import { ProviderIcon, SmallIcon } from "../icons";
import { parentId, type TreeNode } from "../model";

interface Props {
  nodes: TreeNode[];
  selectedId: string | null;
  expanded: Set<string>;
  openConnections: Set<string>;
  openDatabases: Set<string>;
  connecting: Set<string>;
  /** Ids de nós sendo lidos do servidor (spinner no lugar da seta). */
  loading: Set<string>;
  filter: string;
  onFilter: (v: string) => void;
  onSelect: (id: string) => void;
  onActivate: (node: TreeNode) => void;
  onToggle: (node: TreeNode) => void;
  onCollapseAll: () => void;
  onContextMenu: (node: TreeNode, x: number, y: number) => void;
  starredOnly: boolean;
  onToggleStarredOnly: () => void;
}

/** Id da conexão a partir do id do nó "conn:<id>". */
export const connIdOf = (nodeId: string) => decodeURIComponent(nodeId.slice(5));

export function NavigationPane(p: Props) {
  const t = useT();
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!p.selectedId) return;
    const el = listRef.current?.querySelector<HTMLElement>(`[data-id="${CSS.escape(p.selectedId)}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [p.selectedId]);

  const isExpanded = (n: TreeNode) => {
    if (n.kind === "connection") return p.openConnections.has(connIdOf(n.id)) && p.expanded.has(n.id);
    if (n.kind === "database") return p.openDatabases.has(n.id) && p.expanded.has(n.id);
    return p.expanded.has(n.id);
  };

  const onKeyDown = (e: ReactKeyboardEvent) => {
    const idx = p.nodes.findIndex((n) => n.id === p.selectedId);
    const node = p.nodes[idx];
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        if (idx < p.nodes.length - 1) p.onSelect(p.nodes[idx + 1].id);
        break;
      case "ArrowUp":
        e.preventDefault();
        if (idx > 0) p.onSelect(p.nodes[idx - 1].id);
        break;
      case "Home":
        e.preventDefault();
        p.onSelect(p.nodes[0].id);
        break;
      case "End":
        e.preventDefault();
        p.onSelect(p.nodes[p.nodes.length - 1].id);
        break;
      case "ArrowRight":
        e.preventDefault();
        if (!node) break;
        if (node.expandable && !isExpanded(node)) p.onToggle(node);
        else if (idx < p.nodes.length - 1 && p.nodes[idx + 1].depth > node.depth) p.onSelect(p.nodes[idx + 1].id);
        break;
      case "ArrowLeft": {
        e.preventDefault();
        if (!node) break;
        if (node.expandable && isExpanded(node)) p.onToggle(node);
        else {
          const parent = parentId(node.id);
          if (parent) p.onSelect(parent);
        }
        break;
      }
      case "Enter":
        e.preventDefault();
        if (node) p.onActivate(node);
        break;
    }
  };

  const icon = (n: TreeNode) => {
    switch (n.kind) {
      case "root":
        return <SmallIcon name="myConnections" />;
      case "connection":
        return <ProviderIcon provider={n.provider ?? "postgresql"} open={p.openConnections.has(connIdOf(n.id))} />;
      case "database":
        return <SmallIcon name={p.openDatabases.has(n.id) ? "database" : "databaseClosed"} />;
      case "schema":
        return <SmallIcon name="schema" className={isExpanded(n) ? undefined : "is-muted"} />;
      case "column":
        return <SmallIcon name={n.columnRole === "pk" ? "columnPk" : n.columnRole === "fk" ? "columnFk" : "column"} />;
      default:
        return <SmallIcon name={n.category ?? "table"} />;
    }
  };

  /** Dica do nó: papel da coluna (PK/FK) e o comentário do banco, quando houver. */
  const treeTitle = (n: TreeNode): string | undefined => {
    const role = n.columnRole === "pk" ? t("des.primaryKey") : n.columnRole === "fk" ? t("nav.foreignKey") : undefined;
    const lines = [role, n.comment].filter(Boolean);
    return lines.length ? lines.join("\n") : undefined;
  };

  return (
    <aside className="nav-pane" aria-label={t("nav.myConnections")}>
      <div
        className="nav-tree"
        role="tree"
        tabIndex={0}
        ref={listRef}
        onKeyDown={onKeyDown}
        aria-activedescendant={p.selectedId ? `tree-${p.selectedId}` : undefined}
      >
        {p.nodes.map((n) => {
          const selected = n.id === p.selectedId;
          const expanded = n.expandable && isExpanded(n);
          const busy = (n.kind === "connection" && p.connecting.has(connIdOf(n.id))) || p.loading.has(n.id);
          // Como na referência: conexão/banco fechados não exibem seta até serem abertos.
          const opened =
            (n.kind !== "connection" || p.openConnections.has(connIdOf(n.id))) &&
            (n.kind !== "database" || p.openDatabases.has(n.id));
          return (
            <div
              key={n.id}
              id={`tree-${n.id}`}
              data-id={n.id}
              role="treeitem"
              aria-selected={selected}
              aria-expanded={n.expandable ? expanded : undefined}
              aria-level={n.depth + 1}
              className={`tree-row${n.kind === "root" ? " tree-root" : ""}${selected ? " is-selected" : ""}${n.color ? ` has-color color-${n.color}` : ""}`}
              style={{ paddingLeft: n.kind === "root" ? 16 : 14 + n.depth * 18 }}
              onMouseDown={() => p.onSelect(n.id)}
              onDoubleClick={() => p.onActivate(n)}
              onContextMenu={(e) => {
                e.preventDefault();
                p.onSelect(n.id);
                p.onContextMenu(n, e.clientX, e.clientY);
              }}
            >
              {n.kind !== "root" && (
                <span
                  className="tree-twisty"
                  onMouseDown={(e) => {
                    e.stopPropagation();
                    p.onSelect(n.id);
                    if (n.expandable) p.onToggle(n);
                  }}
                >
                  {(busy || (n.expandable && opened)) && isExpandedOrBusy(expanded, busy)}
                </span>
              )}
              <span className="tree-icon">{icon(n)}</span>
              <span className="tree-label" title={treeTitle(n)}>{n.label}</span>
              {n.starred && <span className="tree-star" aria-label="★"><SmallIcon name="starFilled" size={12} /></span>}
              {busy && n.kind === "connection" && <span className="tree-busy">{t("nav.connecting")}</span>}
            </div>
          );
        })}
      </div>

      <div className="nav-search">
        <SmallIcon name="search" />
        <input
          type="search"
          value={p.filter}
          placeholder={t("nav.search")}
          aria-label={t("nav.search")}
          onChange={(e) => p.onFilter(e.target.value)}
        />
        <button type="button" className="icon-button" title={t("nav.filter")} disabled>
          <SmallIcon name="filter" />
        </button>
        <button
          type="button"
          className={`icon-button${p.starredOnly ? " is-active" : ""}`}
          title={t("nav.showFavorites")}
          aria-pressed={p.starredOnly}
          onClick={p.onToggleStarredOnly}
        >
          <SmallIcon name={p.starredOnly ? "starFilled" : "star"} />
        </button>
        <button type="button" className="icon-button" title={t("nav.collapseAll")} onClick={p.onCollapseAll}>
          <SmallIcon name="collapseAll" />
        </button>
      </div>
    </aside>
  );
}

function isExpandedOrBusy(expanded: boolean, busy: boolean) {
  if (busy) return <span className="spinner" />;
  return <SmallIcon name={expanded ? "chevronDown" : "chevronRight"} size={12} />;
}
