import type { ConnectionMock } from "./model";

export interface ShareOptions {
  /** Inclui o servidor e a porta. */
  hosts: boolean;
  /** Inclui um campo de senha em branco para quem recebe preencher (a senha de verdade nunca vai). */
  password: boolean;
}

const enc = encodeURIComponent;

/**
 * Endereço (`atlas://...`) que descreve uma conexão, ou um banco dela, para passar a outra pessoa.
 * Não leva senha nem chave: só o que identifica o servidor e o banco.
 */
export function buildShareUri(conn: Pick<ConnectionMock, "id" | "provider" | "name" | "host" | "port" | "user" | "initialDatabase" | "sslMode" | "ssh">, database: string | null, opts: ShareOptions): string {
  const params: [string, string][] = [];
  if (database !== null) params.push(["Database.Name", database]);
  params.push(["Conn.ConnectionParameters", ""]);
  if (opts.hosts) params.push(["Conn.Host", conn.host]);
  params.push(["Conn.InitialDatabase", conn.initialDatabase || (conn.provider === "postgresql" ? "postgres" : "")], ["Conn.Name", conn.name]);
  if (opts.password) params.push(["Conn.Password", ""]);
  if (opts.hosts) params.push(["Conn.Port", String(conn.port)]);
  params.push(
    ["Conn.UseHTTP", "false"],
    ["Conn.UseSSH", String(!!conn.ssh)],
    ["Conn.UseSSL", String(conn.sslMode === "require" || conn.sslMode === "verify-ca" || conn.sslMode === "verify-full")],
    ["Conn.Username", conn.user],
  );
  const product = ({ postgresql: "pgsql", sqlserver: "mssql", mysql: "mysql", sqlite: "sqlite", firebird: "firebird" } as Record<string, string>)[conn.provider] ?? "pgsql";
  const path = database === null ? `connection.${product}@local/${enc(conn.id)}` : `database.${product}@local/${enc(conn.id)}/${enc(database)}`;
  return `atlas://${path}?${params.map(([k, v]) => `${k}=${enc(v)}`).join("&")}`;
}
