import { quoteIdent } from "../model";

/** Estado editável de uma view no designer. */
export interface ViewState {
  /** O SELECT, como o usuário escreveu (sem `;` final ao gerar o DDL). */
  definition: string;
  owner: string;
  comment: string;
  checkOption: "" | "LOCAL" | "CASCADED";
  securityBarrier: boolean;
}

export const emptyView = (): ViewState => ({ definition: "", owner: "", comment: "", checkOption: "", securityBarrier: false });

/** Remove espaços e o `;` final: o SELECT entra no meio do CREATE VIEW. */
export function cleanDefinition(sql: string): string {
  return sql.trim().replace(/;+\s*$/, "").trimEnd();
}

const literal = (s: string) => `'${s.replace(/'/g, "''")}'`;

function defineSql(schema: string, name: string, s: ViewState, replace: boolean, versionNum: number): string {
  // security_barrier: 9.2+; CHECK OPTION: 9.4+. Em versões antigas a opção não é gerada.
  const withOpts = s.securityBarrier && versionNum >= 90200 ? " WITH (security_barrier = true)" : "";
  const check = s.checkOption && versionNum >= 90400 ? `\n  WITH ${s.checkOption} CHECK OPTION` : "";
  return `CREATE ${replace ? "OR REPLACE " : ""}VIEW ${quoteIdent(schema)}.${quoteIdent(name)}${withOpts} AS\n${cleanDefinition(s.definition)}${check};`;
}

/** DDL de uma view nova (também a prévia "Salvar como"). */
export function createViewStatements(schema: string, name: string, s: ViewState, versionNum: number): string[] {
  const target = `${quoteIdent(schema)}.${quoteIdent(name)}`;
  const out = [defineSql(schema, name, s, false, versionNum)];
  if (s.owner) out.push(`ALTER VIEW ${target} OWNER TO ${quoteIdent(s.owner)};`);
  if (s.comment) out.push(`COMMENT ON VIEW ${target} IS ${literal(s.comment)};`);
  return out;
}

/**
 * Só o que mudou. Definição/opções viram CREATE OR REPLACE VIEW (o PostgreSQL exige que as
 * colunas existentes continuem com o mesmo nome e tipo; o erro do servidor é mostrado).
 */
export function alterViewStatements(schema: string, name: string, before: ViewState, after: ViewState, versionNum: number): string[] {
  const target = `${quoteIdent(schema)}.${quoteIdent(name)}`;
  const out: string[] = [];
  const defChanged =
    cleanDefinition(before.definition) !== cleanDefinition(after.definition) ||
    before.checkOption !== after.checkOption ||
    before.securityBarrier !== after.securityBarrier;
  if (defChanged) out.push(defineSql(schema, name, after, true, versionNum));
  if (after.owner && after.owner !== before.owner) out.push(`ALTER VIEW ${target} OWNER TO ${quoteIdent(after.owner)};`);
  if (after.comment !== before.comment) out.push(`COMMENT ON VIEW ${target} IS ${after.comment ? literal(after.comment) : "NULL"};`);
  return out;
}
