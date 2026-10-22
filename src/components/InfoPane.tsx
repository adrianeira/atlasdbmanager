import { useT } from "../i18n";
import { formatBytes, formatInt, type Category, type DbObject, type NodeContext } from "../model";

interface Props {
  ctx: NodeContext;
  category: Category;
  connectionOpen: boolean;
  object: DbObject | null;
  lang: string;
}

export function InfoPane({ ctx, category, connectionOpen, object, lang }: Props) {
  const t = useT();
  const rows: [string, string][] = [];

  if (object) {
    rows.push([t("col.name"), object.name], [t("info.type"), category]);
    if (object.rows !== undefined) rows.push([t("col.rows"), formatInt(object.rows, lang)]);
    if (object.dataLength !== undefined) rows.push([t("col.dataLength"), formatBytes(object.dataLength)]);
    if (object.owner) rows.push([t("col.owner"), object.owner]);
    if (object.returnType) rows.push([t("col.returnType"), object.returnType]);
    if (object.modified) rows.push([t("col.modified"), object.modified]);
    if (object.comment) rows.push([t("col.comment"), object.comment]);
  } else if (ctx.connection) {
    const c = ctx.connection;
    rows.push(
      [t("col.name"), c.name],
      [t("info.server"), `${c.host}:${c.port}`],
      [t("info.version"), connectionOpen ? c.serverVersion : "-"],
      [t("info.state"), connectionOpen ? t("info.open") : t("info.closed")],
    );
    if (ctx.database) rows.push([t("info.database"), ctx.database.name]);
    if (ctx.schema) rows.push([t("info.schema"), ctx.schema.name]);
  }

  return (
    <aside className="info-pane" aria-label={t("info.title")}>
      <div className="info-header">{t("info.general")}</div>
      {rows.length === 0 ? (
        <div className="info-empty">{t("info.nothing")}</div>
      ) : (
        <dl className="info-list">
          {rows.map(([k, v]) => (
            <div key={k} className="info-row">
              <dt>{k}</dt>
              <dd title={v}>{v}</dd>
            </div>
          ))}
        </dl>
      )}
    </aside>
  );
}
