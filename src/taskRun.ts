import { isSqliteId } from "./providerRoute";
import { defaultExportDir, pg } from "./backend";
import { formatBackupName } from "./components/BackupDialog";
import { readToolsDir } from "./components/RestoreDialog";
import { joinPath, type ScheduledTask } from "./tasks";

/**
 * Executa uma tarefa agendada e devolve a mensagem de sucesso (rejeita com o erro do servidor ou da ferramenta).
 * `ensureOpen` abre a conexão se ela estiver fechada (usa a senha do cofre; sem ela a tarefa falha com uma mensagem clara).
 */
export async function runTask(task: ScheduledTask, ensureOpen: (connId: string) => Promise<void>, now = new Date()): Promise<string> {
  await ensureOpen(task.connId);
  const id = `${isSqliteId(task.connId) ? "sqlite_" : ""}${crypto.randomUUID()}`;
  const job = task.job;
  if (job.kind === "backup") {
    const r = await pg.backup({
      jobId: id, sessionId: task.connId, database: task.database, schema: task.schema, toolsDir: readToolsDir() || undefined,
      fileName: formatBackupName(job.namePattern, now), comment: job.comment, exclude: [], folder: task.folder || undefined,
    });
    return `${r.name} (${r.bytes.toLocaleString()} bytes)`;
  }
  const folder = task.folder || (await defaultExportDir());
  if (!folder) throw new Error("no destination folder");
  const path = joinPath(folder, `${formatBackupName(job.namePattern, now)}.${job.extension}`);
  const r = await pg.export({
    exportId: id, sessionId: task.connId, database: task.database, schema: task.schema ?? "public", table: job.table,
    columns: [], orderBy: [], filter: null, format: job.exportFormat, path,
  });
  return `${r.path} (${r.rows.toLocaleString()} rows)`;
}
