/** Tarefas agendadas (backup e exportação): o que são, quando rodam e como se escrevem na lista. Roda com o app aberto. */

import type { ExportFormat } from "./backend";

export type Schedule =
  /** Todo dia, no horário `time` (`HH:MM`, hora local). */
  | { type: "daily"; time: string }
  /** A cada `minutes` minutos, contados a partir da última execução (ou da criação). */
  | { type: "interval"; minutes: number }
  /** Uma vez, em `at` (`AAAA-MM-DDTHH:MM`, hora local). */
  | { type: "once"; at: string };

export interface BackupJob {
  kind: "backup";
  /** Padrão do nome do arquivo (`%Y %m %d %H %M %S`). */
  namePattern: string;
  comment: string;
}

export interface ExportJob {
  kind: "export";
  table: string;
  /** Tipo de arquivo do assistente de exportação (`csv`, `json`, `xlsx`...). */
  format: string;
  /** Opções do formato como o servidor as recebe; a extensão vem do tipo. */
  exportFormat: ExportFormat;
  extension: string;
  /** Padrão do nome do arquivo, sem a extensão (`%Y %m %d %H %M %S`). */
  namePattern: string;
}

export interface ScheduledTask {
  id: string;
  name: string;
  enabled: boolean;
  connId: string;
  database: string;
  /** Backup sem schema é do banco inteiro; exportação exige schema. */
  schema?: string;
  /** Pasta de destino; vazia = a pasta padrão do app. */
  folder: string;
  job: BackupJob | ExportJob;
  schedule: Schedule;
  /** Quando foi criada ou editada (marca o início da contagem do intervalo). */
  createdMs: number;
  lastRunMs?: number;
  lastStatus?: "ok" | "error";
  lastMessage?: string;
}

const HHMM = /^([01]\d|2[0-3]):([0-5]\d)$/;
const LOCAL = /^(\d{4})-(\d{2})-(\d{2})T([01]\d|2[0-3]):([0-5]\d)$/;

export const isValidTime = (v: string) => HHMM.test(v);
/** Data e hora que existem de verdade (30 de fevereiro não passa, mesmo que o `Date` o "arredonde"). */
export function isValidLocal(v: string): boolean {
  const m = LOCAL.exec(v);
  if (!m) return false;
  const [y, mo, d, h, mi] = m.slice(1).map(Number);
  const date = new Date(y, mo - 1, d, h, mi);
  return date.getFullYear() === y && date.getMonth() === mo - 1 && date.getDate() === d;
}

/** Problema do agendamento (ou `null`). */
export function scheduleProblem(s: Schedule): "time" | "interval" | "once" | null {
  if (s.type === "daily") return isValidTime(s.time) ? null : "time";
  if (s.type === "interval") return Number.isInteger(s.minutes) && s.minutes >= 1 && s.minutes <= 7 * 24 * 60 ? null : "interval";
  return isValidLocal(s.at) ? null : "once";
}

/**
 * Próxima execução (ms desde 1970), ou `null` se não há mais (tarefa "uma vez" que já rodou).
 * Diária: o próximo horário depois da última execução (ou da criação); passou e a tarefa não rodou, ela está vencida e roda já.
 * Intervalo: a última execução (ou a criação) mais o intervalo. Uma vez: o instante marcado, enquanto não rodou.
 */
export function nextRunMs(task: Pick<ScheduledTask, "schedule" | "createdMs" | "lastRunMs">): number | null {
  const s = task.schedule;
  const since = task.lastRunMs ?? task.createdMs;
  if (s.type === "interval") return since + s.minutes * 60_000;
  if (s.type === "once") return task.lastRunMs === undefined ? new Date(s.at).getTime() : null;
  const [h, m] = s.time.split(":").map(Number);
  const d = new Date(since);
  d.setHours(h, m, 0, 0);
  if (d.getTime() <= since) d.setDate(d.getDate() + 1);
  return d.getTime();
}

/** A tarefa deve rodar agora? Só as habilitadas, e só uma vez por vencimento (o atraso não acumula execuções). */
export function isDue(task: ScheduledTask, nowMs: number): boolean {
  if (!task.enabled || scheduleProblem(task.schedule)) return false;
  const next = nextRunMs(task);
  return next !== null && next <= nowMs;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** `dd/MM/aaaa HH:mm` na hora local (a lista mostra assim). */
export function formatWhen(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${d.getFullYear()} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Texto do agendamento para a lista (`unit` traduz minutos/horas/dias; as palavras vêm do i18n na tela). */
export function describeInterval(minutes: number): { n: number; unit: "minutes" | "hours" | "days" } {
  if (minutes % (24 * 60) === 0) return { n: minutes / (24 * 60), unit: "days" };
  if (minutes % 60 === 0) return { n: minutes / 60, unit: "hours" };
  return { n: minutes, unit: "minutes" };
}

/** Caminho do arquivo da exportação: `pasta/nome.extensão`, com o separador que a pasta já usa. */
export function joinPath(folder: string, file: string): string {
  const sep = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
  return folder.endsWith("/") || folder.endsWith("\\") ? `${folder}${file}` : `${folder}${sep}${file}`;
}

/** Nome da tarefa que não repete outra (`Backup diário`, `Backup diário (2)`). */
export function uniqueName(base: string, tasks: ScheduledTask[], exceptId?: string): string {
  const taken = new Set(tasks.filter((t) => t.id !== exceptId).map((t) => t.name.toLowerCase()));
  let name = base;
  for (let k = 2; taken.has(name.toLowerCase()); k++) name = `${base} (${k})`;
  return name;
}
