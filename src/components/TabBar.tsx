import { useState } from "react";
import { useT } from "../i18n";
import { SmallIcon, type SmallIconName } from "../icons";

export interface DocTab {
  id: string;
  kind: "objects" | "query" | "data" | "designer" | "viewDesigner" | "functionDesigner" | "sequenceDesigner" | "typeDesigner" | "extensionDesigner" | "foreignServerDesigner" | "foreignTableDesigner" | "tablespaceDesigner" | "roleDesigner" | "monitor" | "historyLog" | "find" | "console" | "placeholder";
  /** Aba "data": tabela ou view (view abre só leitura). */
  objectKind?: "table" | "view" | "matview";
  title: string;
  /** Título longo para a barra da janela (ex.: "audit_log @atlas_demo.public (PostgreSQL 9.6) - Table"). */
  windowTitle?: string;
  icon?: SmallIconName;
  connectionId?: string;
  database?: string;
  schema?: string;
  /** Nome do objeto; ausente em "nova tabela". */
  object?: string;
  /** Função: nome puro e argumentos de identidade (`object` é a chave "nome(args)"). */
  fnName?: string;
  fnIdentity?: string;
  sql?: string;
  /** Consulta: executa o `sql` assim que a aba abre. */
  autoRun?: boolean;
  dirty?: boolean;
  /** Texto do placeholder para telas ainda não construídas. */
  note?: string;
}

interface Props {
  tabs: DocTab[];
  activeId: string;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  /** Arrastou a aba `id` para antes (ou depois) da aba `targetId`. */
  onReorder?: (id: string, targetId: string, after: boolean) => void;
}

export function TabBar({ tabs, activeId, onActivate, onClose, onReorder }: Props) {
  const t = useT();
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropAt, setDropAt] = useState<{ id: string; after: boolean } | null>(null);
  return (
    <div className="tab-bar" role="tablist">
      {tabs.map((tab) => {
        const active = tab.id === activeId;
        const closable = tab.kind !== "objects";
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            className={`tab${active ? " is-active" : ""}${dropAt?.id === tab.id && dragId !== tab.id ? (dropAt.after ? " is-drop-after" : " is-drop-before") : ""}`}
            title={tab.title}
            draggable={!!onReorder && closable}
            onDragStart={(e) => {
              setDragId(tab.id);
              e.dataTransfer.effectAllowed = "move";
              e.dataTransfer.setData("text/plain", tab.title);
            }}
            onDragOver={(e) => {
              if (!dragId) return;
              e.preventDefault();
              const r = e.currentTarget.getBoundingClientRect();
              // Nada entra antes da aba Objects, que é sempre a primeira.
              const after = !closable || e.clientX > r.left + r.width / 2;
              if (dropAt?.id !== tab.id || dropAt.after !== after) setDropAt({ id: tab.id, after });
            }}
            onDrop={(e) => {
              e.preventDefault();
              if (dragId && dropAt && dragId !== dropAt.id) onReorder?.(dragId, dropAt.id, dropAt.after);
              setDragId(null);
              setDropAt(null);
            }}
            onDragEnd={() => { setDragId(null); setDropAt(null); }}
            onMouseDown={(e) => {
              if (e.button === 1 && closable) {
                e.preventDefault();
                onClose(tab.id);
              } else if (e.button === 0) onActivate(tab.id);
            }}
          >
            {tab.icon && <SmallIcon name={tab.icon} />}
            <span className="tab-title">{tab.title}{tab.dirty ? " *" : ""}</span>
            {closable && (
              <button
                type="button"
                className="tab-close"
                aria-label={t("tab.close")}
                title={t("tab.close")}
                onMouseDown={(e) => e.stopPropagation()}
                onClick={() => onClose(tab.id)}
              >
                <SmallIcon name="close" size={12} />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}
