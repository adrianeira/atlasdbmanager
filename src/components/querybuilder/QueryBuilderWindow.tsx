import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ToolWindow } from "../ToolWindow";
import { QueryBuilderDialog } from "./QueryBuilderDialog";

interface Params {
  id: string;
  connId: string;
  database: string;
  defaultSchema: string;
  initialSql: string;
  title: string;
}

/** Conteúdo da janela separada do Query Builder: o mesmo componente, ocupando a janela inteira. */
export function QueryBuilderWindow() {
  const params = (window as unknown as { __ATLAS_QB__: Params }).__ATLAS_QB__;
  const close = () => void getCurrentWindow().close();
  return (
    <ToolWindow title={params.title}>
      <QueryBuilderDialog
        connId={params.connId}
        database={params.database}
        defaultSchema={params.defaultSchema}
        initialSql={params.initialSql}
        onClose={close}
        onBuild={(sql, run, replaced) => {
          void emit("qb-build", { id: params.id, sql, run, replaced }).finally(close);
        }}
      />
    </ToolWindow>
  );
}
