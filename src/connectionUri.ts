/** Campos de uma conexão PostgreSQL que cabem numa URI (`postgresql://usuario:senha@host:porta/banco?sslmode=...`). */
export interface UriFields {
  host: string;
  port: string;
  user: string;
  password: string;
  database: string;
  /** Vazio = a URI não diz; `disable` = sem TLS. */
  sslMode: string;
}

export type UriResult = { ok: true; fields: UriFields } | { ok: false; error: "scheme" | "host" | "port" | "invalid" };

const decode = (s: string): string | null => {
  try {
    return decodeURIComponent(s);
  } catch {
    return null;
  }
};

const SSL_MODES = ["disable", "allow", "prefer", "require", "verify-ca", "verify-full"];

/** Lê uma URI `postgresql://` ou `postgres://`; qualquer parte pode faltar (o formulário mantém o que já tinha). */
export function parseConnectionUri(raw: string): UriResult {
  const text = raw.trim();
  const m = /^(postgres|postgresql):\/\/(.*)$/i.exec(text);
  if (!m) return { ok: false, error: "scheme" };
  let rest = m[2];
  let query = "";
  const q = rest.indexOf("?");
  if (q >= 0) {
    query = rest.slice(q + 1);
    rest = rest.slice(0, q);
  }
  let path = "";
  const slash = rest.indexOf("/");
  if (slash >= 0) {
    path = rest.slice(slash + 1);
    rest = rest.slice(0, slash);
  }
  let user = "";
  let password = "";
  const at = rest.lastIndexOf("@");
  let hostPort = rest;
  if (at >= 0) {
    const info = rest.slice(0, at);
    hostPort = rest.slice(at + 1);
    const colon = info.indexOf(":");
    const u = decode(colon >= 0 ? info.slice(0, colon) : info);
    const p = colon >= 0 ? decode(info.slice(colon + 1)) : "";
    if (u === null || p === null) return { ok: false, error: "invalid" };
    user = u;
    password = p;
  }
  let host = hostPort;
  let port = "";
  if (hostPort.startsWith("[")) {
    const end = hostPort.indexOf("]");
    if (end < 0) return { ok: false, error: "host" };
    host = hostPort.slice(1, end);
    const after = hostPort.slice(end + 1);
    if (after.startsWith(":")) port = after.slice(1);
    else if (after) return { ok: false, error: "host" };
  } else {
    const colon = hostPort.lastIndexOf(":");
    if (colon >= 0) {
      host = hostPort.slice(0, colon);
      port = hostPort.slice(colon + 1);
    }
  }
  const h = decode(host);
  if (h === null) return { ok: false, error: "host" };
  if (port !== "" && (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535)) return { ok: false, error: "port" };
  const database = decode(path);
  if (database === null) return { ok: false, error: "invalid" };
  let sslMode = "";
  for (const part of query.split("&")) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    if (part.slice(0, eq).toLowerCase() === "sslmode") {
      const v = (decode(part.slice(eq + 1)) ?? "").toLowerCase();
      if (SSL_MODES.includes(v)) sslMode = v;
    }
  }
  return { ok: true, fields: { host: h, port, user, password, database, sslMode } };
}

const enc = (s: string) => encodeURIComponent(s);

/** A URI da conexão, sem a senha (ela não deve aparecer em texto na tela). */
export function formatConnectionUri(f: { host: string; port: string; user: string; database: string; sslMode: string }): string {
  const host = f.host.includes(":") && !f.host.startsWith("[") ? `[${f.host}]` : f.host;
  const user = f.user ? `${enc(f.user)}@` : "";
  const port = f.port ? `:${f.port}` : "";
  const db = f.database ? `/${enc(f.database)}` : "";
  const ssl = f.sslMode && f.sslMode !== "disable" ? `?sslmode=${f.sslMode}` : "";
  return `postgresql://${user}${host}${port}${db}${ssl}`;
}
