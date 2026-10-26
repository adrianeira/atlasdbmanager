export type RoutineDialect = "mssql" | "mysql";

const q = (d: RoutineDialect, name: string) => (d === "mssql" ? `[${name.replace(/]/g, "]]")}]` : `\`${name.replace(/`/g, "``")}\``);

/** O modelo de uma rotina nova. */
export function routineTemplate(dialect: RoutineDialect, schema: string): string {
  return dialect === "mssql"
    ? `CREATE OR ALTER PROCEDURE ${q(dialect, schema)}.[new_procedure]\nAS\nBEGIN\n  SET NOCOUNT ON;\n  SELECT 1 AS x;\nEND`
    : `CREATE PROCEDURE ${q(dialect, schema)}.\`new_procedure\`()\nBEGIN\n  SELECT 1 AS x;\nEND`;
}

/** O nome da rotina que o texto cria (sem o schema nem as aspas), ou `null` se o texto não começa com `CREATE`. */
export function routineName(text: string): string | null {
  const m = /^\s*(?:--[^\n]*\n|\/\*[\s\S]*?\*\/|\s)*CREATE\s+(?:OR\s+(?:ALTER|REPLACE)\s+)?(?:DEFINER\s*=\s*\S+\s+)?(?:PROCEDURE|PROC|FUNCTION)\s+(?:IF\s+NOT\s+EXISTS\s+)?((?:[`[\]"\w$]+\.)?[`[\]"\w$]+)/i.exec(text);
  if (!m) return null;
  const last = m[1].split(".").pop() ?? "";
  return last.replace(/^[`[\]"]|[`[\]"]$/g, "") || null;
}

/** O tipo da rotina que o texto cria. */
const kindOf = (text: string): "PROCEDURE" | "FUNCTION" => (/^\s*(?:--[^\n]*\n|\/\*[\s\S]*?\*\/|\s)*CREATE\s+(?:OR\s+\w+\s+)?(?:DEFINER\s*=\s*\S+\s+)?FUNCTION\b/i.test(text) ? "FUNCTION" : "PROCEDURE");

/**
 * O script que grava a rotina. No SQL Server `CREATE OR ALTER` já substitui; no MySQL a rotina existente é apagada e criada de novo.
 */
export function routineSaveScript(dialect: RoutineDialect, text: string): string {
  const body = text.trim().replace(/;+\s*$/, "");
  if (dialect === "mssql") return body.replace(/^(\s*CREATE)\s+(PROC|PROCEDURE|FUNCTION)\b/i, "$1 OR ALTER $2");
  const name = routineName(text) ?? "";
  return `DROP ${kindOf(text)} IF EXISTS ${q(dialect, name)};\n${body};`;
}
