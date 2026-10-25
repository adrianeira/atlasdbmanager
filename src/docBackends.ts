/** Ligação das abas de tabela e de consulta ao banco real; usada pela janela principal e pelas abas destacadas. */
import { pg } from "./backend";
import { isSqliteId } from "./providerRoute";
import type { FkLookupSource } from "./components/FkLookup";
import type { TableDataBackend } from "./documents/TableDataDoc";
import { tableFromServer } from "./live";
import { quoteIdent, type DbObject } from "./model";

/** Leitura, gravação e transação manual de uma tabela (ou view). Cada chamada cria uma transação própria, então use uma por aba. */
export function makeTableBackend(connId: string, db: string, schema: string, object: string, objectKind: "table" | "view" | "matview" | undefined, base: DbObject): TableDataBackend {
  // Transação manual da aba: enquanto aberta, gravação, leitura e contagem usam a conexão dedicada dela.
  let txId: string | null = null;
  const sqlite = isSqliteId(connId);
  return {
    dialect: sqlite ? "sqlite" : undefined,
    loadDetail: async () => tableFromServer(base, await pg.tableDetail(connId, db, schema, object)),
    profileData: (columns, orderBy, filter, limit) => pg.dataProfile(connId, db, schema, object, columns, orderBy, filter, limit),
    loadProfile: objectKind === "view" ? undefined : () => pg.tableProfile(connId, db, schema, object),
    beginTx: async () => { txId = await pg.txBegin(connId, db); },
    endTx: async (commit) => {
      const id = txId;
      txId = null;
      if (id) await pg.txEnd(id, commit);
    },
    loadRows: async (orderBy, filter, limit, offset) => {
      const r = txId
        ? await pg.txRows(txId, schema, object, orderBy, filter, limit, offset)
        : await pg.tableRows(connId, db, schema, object, orderBy, filter, limit, offset);
      const rs = r.outcome.statements[0]?.result;
      return { sql: r.sql, rows: rs?.rows ?? [], truncated: !!rs?.truncated };
    },
    apply: (statements) => (txId ? pg.txApply(txId, statements) : pg.apply(connId, db, statements)),
    cancel: async () => {
      await pg.cancel(connId, db);
    },
    count: async (filter) => (txId
      ? pg.txCount(txId, schema, object, filter)
      : pg.tableCount(connId, db, schema, object, filter)),
  };
}

/** Fonte do lookup de chave estrangeira no banco real (só SELECT de leitura). */
export function liveLookupSource(connId: string, db: string): FkLookupSource {
  return {
    columns: async (schema, table) =>
      (await pg.tableDetail(connId, db, schema, table)).columns.map((c) => ({ name: c.name, type: c.dataType })),
    rows: async (schema, table, columns, orderBy, limit) => {
      // Só identificadores (com aspas) e um limite numérico entram no texto; nenhum valor do usuário.
      const sql = `SELECT ${columns.map(quoteIdent).join(", ")} FROM ${quoteIdent(schema)}.${quoteIdent(table)}`
        + (orderBy.length ? ` ORDER BY ${orderBy.map(quoteIdent).join(", ")}` : "") + ` LIMIT ${limit}`;
      const out = await pg.execute(connId, db, sql, limit);
      return out.statements[0]?.result?.rows ?? [];
    },
    labels: (schema, table, keyColumn, keys) => pg.fkLabels(connId, db, schema, table, keyColumn, keys),
  };
}
