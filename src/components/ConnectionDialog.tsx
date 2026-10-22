import { useEffect, useMemo, useRef, useState, type ReactNode, type Ref } from "react";
import type { MessageKey } from "../i18n";
import { useT } from "../i18n";
import { PROVIDER_STYLE, ProviderIcon, ProviderTile, SmallIcon } from "../icons";
import type { ConnectionMock, ProviderId } from "../model";

/* ---------- Tipos de conexão ---------- */

interface ProviderOption {
  id: ProviderId | "oracle" | "mongodb" | "redis";
  name: string;
  defaultPort: number;
  defaultUser: string;
  defaultDb: string;
  available: boolean;
}

const PROVIDERS: ProviderOption[] = [
  { id: "mysql", name: "MySQL", defaultPort: 3306, defaultUser: "root", defaultDb: "", available: true },
  { id: "postgresql", name: "PostgreSQL", defaultPort: 5432, defaultUser: "postgres", defaultDb: "postgres", available: true },
  { id: "sqlserver", name: "SQL Server", defaultPort: 1433, defaultUser: "sa", defaultDb: "master", available: true },
  { id: "sqlite", name: "SQLite", defaultPort: 0, defaultUser: "", defaultDb: "", available: true },
  { id: "mariadb", name: "MariaDB", defaultPort: 3306, defaultUser: "root", defaultDb: "", available: true },
  // Fora da primeira release (spec §2.1): listados como "em breve", sem fluxo falso.
  { id: "oracle", name: "Oracle", defaultPort: 1521, defaultUser: "", defaultDb: "", available: false },
  { id: "mongodb", name: "MongoDB", defaultPort: 27017, defaultUser: "", defaultDb: "", available: false },
  { id: "redis", name: "Redis", defaultPort: 6379, defaultUser: "", defaultDb: "", available: false },
];

const FUTURE_STYLE: Record<string, [string, string]> = { oracle: ["#c74634", "Ora"], mongodb: ["#3f8f4a", "Mgo"], redis: ["#d6453d", "Rds"] };

function tileStyle(p: ProviderOption): [string, string] {
  if (p.id in PROVIDER_STYLE) {
    const [color, , label] = PROVIDER_STYLE[p.id as ProviderId];
    return [color, label];
  }
  return FUTURE_STYLE[p.id];
}

/* ---------- Formulário ---------- */

export type Environment = "local" | "development" | "test" | "staging" | "production" | "custom";

export interface ConnectionForm {
  provider: ProviderId;
  name: string;
  host: string;
  port: string;
  initialDatabase: string;
  user: string;
  password: string;
  savePassword: boolean;
  environment: Environment;
  charset: string;
  keepalive: boolean;
  keepaliveSec: string;
  autoConnect: boolean;
  limitSessions: boolean;
  sessionLimit: string;
  parameters: string;
  initialQueries: string;
  customDbList: boolean;
  databases: { name: string; autoOpen: boolean }[];
  useSsl: boolean;
  sslMode: string;
  sslAuth: boolean;
  sslClientKey: string;
  sslClientCert: string;
  sslRootCert: string;
  sslCrl: string;
  useSsh: boolean;
  sshHost: string;
  sshPort: string;
  sshUser: string;
  sshAuth: "password" | "publickey";
  sshPassword: string;
  sshSavePassword: boolean;
  sshKeyPath: string;
  remarks: string;
}

function defaultForm(p: ProviderOption): ConnectionForm {
  return {
    provider: p.id as ProviderId,
    name: "", host: p.id === "sqlite" ? "" : "localhost", port: p.defaultPort ? String(p.defaultPort) : "",
    initialDatabase: p.defaultDb, user: p.defaultUser, password: "", savePassword: true, environment: "development",
    charset: "Auto", keepalive: false, keepaliveSec: "240", autoConnect: false, limitSessions: false, sessionLimit: "10",
    parameters: "", initialQueries: "", customDbList: false, databases: [],
    useSsl: false, sslMode: "verify-full", sslAuth: false, sslClientKey: "", sslClientCert: "", sslRootCert: "", sslCrl: "",
    useSsh: false, sshHost: "", sshPort: "22", sshUser: "", sshAuth: "password", sshPassword: "", sshSavePassword: true, sshKeyPath: "",
    remarks: "",
  };
}

export function formFromConnection(c: ConnectionMock): ConnectionForm {
  const p = PROVIDERS.find((x) => x.id === c.provider) ?? PROVIDERS[1];
  return { ...defaultForm(p), name: c.name, host: c.host, port: String(c.port), initialDatabase: c.initialDatabase, user: c.user };
}

type TestState = { kind: "idle" } | { kind: "running" } | { kind: "ok"; version: number } | { kind: "fail"; message: string };

/** Mock do teste de conexão. No M1 vira o comando IPC `connection_test` (driver real, TLS, versão). */
function mockTest(f: ConnectionForm): Promise<TestState> {
  return new Promise((resolve) =>
    window.setTimeout(() => {
      const port = Number(f.port);
      if (f.provider !== "sqlite" && !f.host.trim()) resolve({ kind: "fail", message: 'could not translate host name "" to address: Unknown host' });
      else if (f.provider !== "sqlite" && (!Number.isInteger(port) || port < 1 || port > 65535)) resolve({ kind: "fail", message: `invalid port number: "${f.port}"` });
      else if (f.provider !== "sqlite" && !f.password) {
        const addr = f.host === "localhost" ? "(::1)" : "";
        resolve({ kind: "fail", message: `connection to server at "${f.host}" ${addr}, port ${f.port} failed: fe_sendauth: no password supplied` });
      } else resolve({ kind: "ok", version: port === 5433 ? 180000 : port === 5432 ? 90619 : 160004 });
    }, 650),
  );
}

type Tab = "general" | "advanced" | "databases" | "ssl" | "ssh" | "remarks";
const TABS: { id: Tab; label: MessageKey }[] = [
  { id: "general", label: "conn.tab.general" },
  { id: "advanced", label: "conn.tab.advanced" },
  { id: "databases", label: "conn.tab.databases" },
  { id: "ssl", label: "conn.tab.ssl" },
  { id: "ssh", label: "conn.tab.ssh" },
  { id: "remarks", label: "conn.tab.remarks" },
];

interface Props {
  /** Conexão existente para editar; ausente = nova conexão (começa no seletor de tipo). */
  editing?: ConnectionMock;
  recentProviders: ProviderId[];
  onSave: (form: ConnectionForm, serverVersion: number | null) => void;
  onClose: () => void;
}

export function ConnectionDialog({ editing, recentProviders, onSave, onClose }: Props) {
  const t = useT();
  const [step, setStep] = useState<"type" | "form">(editing ? "form" : "type");
  const [picked, setPicked] = useState<ProviderOption>(PROVIDERS.find((p) => p.id === (editing?.provider ?? recentProviders[0] ?? "postgresql")) ?? PROVIDERS[1]);
  const [form, setForm] = useState<ConnectionForm>(() => (editing ? formFromConnection(editing) : defaultForm(picked)));
  const [tab, setTab] = useState<Tab>("general");
  const [test, setTest] = useState<TestState>({ kind: "idle" });
  const [profilesOpen, setProfilesOpen] = useState(!!editing);
  const [typeFilter, setTypeFilter] = useState<Set<string>>(new Set());
  const [typeSearch, setTypeSearch] = useState("");
  const [nameError, setNameError] = useState(false);
  const firstField = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (step === "form" && tab === "general") firstField.current?.focus();
  }, [step, tab]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const set = <K extends keyof ConnectionForm>(k: K, v: ConnectionForm[K]) => {
    setForm((f) => ({ ...f, [k]: v }));
    if (test.kind !== "running") setTest({ kind: "idle" });
  };

  const runTest = async () => {
    setTest({ kind: "running" });
    setTest(await mockTest(form));
  };

  const submit = () => {
    const port = Number(form.port);
    if (form.provider !== "sqlite" && (!Number.isInteger(port) || port < 1 || port > 65535)) {
      setTab("general");
      setTest({ kind: "fail", message: t("conn.invalidPort") });
      return;
    }
    const name = form.name.trim() || form.host.trim();
    if (!name) {
      setTab("general");
      setNameError(true);
      firstField.current?.focus();
      return;
    }
    onSave({ ...form, name }, test.kind === "ok" ? test.version : null);
  };

  const providerName = PROVIDERS.find((p) => p.id === form.provider)?.name ?? "";
  const title = step === "type"
    ? t("conn.newTitle")
    : editing
      ? `${editing.name} - ${t("conn.editTitle")} (${providerName})`
      : `${t("conn.newTitle")} (${providerName})`;

  /* ---------- Passo 1: tipo ---------- */
  const visibleProviders = useMemo(() => {
    const q = typeSearch.trim().toLowerCase();
    return PROVIDERS.filter((p) => (!typeFilter.size || typeFilter.has(p.id)) && (!q || p.name.toLowerCase().includes(q)));
  }, [typeFilter, typeSearch]);

  const tile = (p: ProviderOption, key: string) => {
    const [color, label] = tileStyle(p);
    return (
      <button
        key={key}
        type="button"
        className={`conn-tile${picked.id === p.id ? " is-selected" : ""}${p.available ? "" : " is-unavailable"}`}
        onClick={() => p.available && setPicked(p)}
        onDoubleClick={() => p.available && goForm(p)}
        title={p.available ? p.name : t("conn.comingSoon")}
        aria-disabled={!p.available}
      >
        <ProviderTile color={color} label={label} provider={p.id} disabled={!p.available} />
        <span>{p.name}</span>
        {!p.available && <span className="conn-soon">{t("conn.comingSoon")}</span>}
      </button>
    );
  };

  const goForm = (p: ProviderOption) => {
    setForm(defaultForm(p));
    setTest({ kind: "idle" });
    setTab("general");
    setStep("form");
  };

  const typeStep = (
    <div className="conn-type">
      <aside className="conn-type-filter">
        <h3>{t("conn.filterTitle")}</h3>
        {PROVIDERS.map((p) => {
          const [color, label] = tileStyle(p);
          return (
            <label key={p.id} className="check-row">
              <input
                type="checkbox"
                checked={typeFilter.has(p.id)}
                onChange={(e) => {
                  const s = new Set(typeFilter);
                  if (e.target.checked) s.add(p.id);
                  else s.delete(p.id);
                  setTypeFilter(s);
                }}
              />
              <ProviderTile color={color} label={label} size={16} provider={p.id} />
              <span>{p.name}</span>
            </label>
          );
        })}
      </aside>
      <section className="conn-type-main">
        <div className="conn-type-head">
          <span>{t("conn.selectType")}</span>
          <input type="search" className="field conn-type-search" placeholder={t("nav.search")} value={typeSearch} onChange={(e) => setTypeSearch(e.target.value)} />
        </div>
        <div className="conn-type-scroll">
          {recentProviders.length > 0 && !typeSearch && !typeFilter.size && (
            <>
              <h4>{t("conn.recents")}</h4>
              <div className="conn-grid">
                {recentProviders.map((id) => PROVIDERS.find((p) => p.id === id)).filter((p): p is ProviderOption => !!p).map((p) => tile(p, `r-${p.id}`))}
              </div>
            </>
          )}
          <h4>{t("conn.all")}</h4>
          <div className="conn-grid">{visibleProviders.map((p) => tile(p, p.id))}</div>
        </div>
      </section>
    </div>
  );

  /* ---------- Passo 2: formulário ---------- */
  const row = (label: MessageKey, control: ReactNode, disabled = false) => (
    <div className={`form-row${disabled ? " is-disabled" : ""}`}>
      <label>{t(label)}:</label>
      <div className="form-control">{control}</div>
    </div>
  );
  const text = (k: keyof ConnectionForm, opts: { width?: number; disabled?: boolean; type?: string; ref?: Ref<HTMLInputElement> } = {}) => (
    <input
      ref={opts.ref}
      className="field"
      type={opts.type ?? "text"}
      style={{ width: opts.width }}
      disabled={opts.disabled}
      value={form[k] as string}
      onChange={(e) => set(k, e.target.value as never)}
      spellCheck={false}
      autoComplete="off"
    />
  );
  const check = (k: keyof ConnectionForm, label: MessageKey, disabled = false) => (
    <label className={`check-row${disabled ? " is-disabled" : ""}`}>
      <input type="checkbox" disabled={disabled} checked={form[k] as boolean} onChange={(e) => set(k, e.target.checked as never)} />
      <span>{t(label)}</span>
    </label>
  );
  const browse = (k: keyof ConnectionForm, disabled: boolean) => (
    <div className="field-with-button">
      {text(k, { disabled })}
      <button type="button" className="btn btn-ellipsis" disabled title={t("notImplemented")}>…</button>
    </div>
  );

  const diagram = (
    <div className={`conn-diagram is-${test.kind}`}>
      <div className="conn-diagram-node">
        <svg width="44" height="44" viewBox="0 0 44 44" aria-hidden="true">
          <circle cx="22" cy="22" r="17" fill="none" stroke="currentColor" strokeWidth="3" />
          <path d="M22 5v34M5 22h34" stroke="currentColor" strokeWidth="2" opacity=".6" />
          <ellipse cx="22" cy="22" rx="8" ry="17" fill="none" stroke="currentColor" strokeWidth="2" opacity=".6" />
        </svg>
        <span>Atlas</span>
      </div>
      <div className="conn-diagram-line">
        {test.kind === "fail" && <span className="conn-diagram-badge"><SmallIcon name="warning" /></span>}
        {test.kind === "running" && <span className="spinner" />}
      </div>
      <div className="conn-diagram-node">
        <svg width="44" height="44" viewBox="0 0 44 44" aria-hidden="true">
          <ellipse cx="22" cy="10" rx="13" ry="5" fill="none" stroke="currentColor" strokeWidth="2.6" />
          <path d="M9 10v24c0 2.8 5.8 5 13 5s13-2.2 13-5V10M9 18c0 2.8 5.8 5 13 5s13-2.2 13-5M9 26c0 2.8 5.8 5 13 5s13-2.2 13-5" fill="none" stroke="currentColor" strokeWidth="2.6" />
        </svg>
        <span>{t("conn.database")}</span>
      </div>
    </div>
  );

  const generalTab = (
    <div className="form-grid">
      <div className={`form-row${nameError ? " has-error" : ""}`}>
        <label>{t("conn.name")}:</label>
        <div className="form-control">
          <input ref={firstField} className="field" value={form.name} onChange={(e) => { setNameError(false); set("name", e.target.value); }} aria-invalid={nameError} spellCheck={false} />
        </div>
      </div>
      <div className="form-gap" />
      {form.provider === "sqlite" ? (
        row("conn.dbFile", browse("host", false))
      ) : (
        <>
          {row("conn.host", text("host"))}
          {row("conn.port", text("port", { width: 48 }))}
          {row("conn.initialDb", text("initialDatabase"))}
          {row("conn.user", text("user", { width: 200 }))}
          {row("conn.password", text("password", { width: 200, type: "password" }))}
          <div className="form-row"><label /><div className="form-control">{check("savePassword", "conn.savePassword")}</div></div>
        </>
      )}
      <div className="form-gap" />
      {row("conn.environment", (
        <select className="field" style={{ width: 200 }} value={form.environment} onChange={(e) => set("environment", e.target.value as Environment)}>
          {(["local", "development", "test", "staging", "production", "custom"] as Environment[]).map((env) => (
            <option key={env} value={env}>{t(`env.${env}` as MessageKey)}</option>
          ))}
        </select>
      ))}
      {form.environment === "production" && <div className="form-note form-note-warn">{t("conn.productionNote")}</div>}
      {form.savePassword && form.provider !== "sqlite" && <div className="form-note">{t("conn.vaultNote")}</div>}
    </div>
  );

  const advancedTab = (
    <div className="form-grid">
      {row("conn.charset", (
        <select className="field" style={{ width: 248 }} value={form.charset} onChange={(e) => set("charset", e.target.value)}>
          {["Auto", "UTF8", "LATIN1", "WIN1252", "SQL_ASCII"].map((c) => <option key={c}>{c}</option>)}
        </select>
      ))}
      <div className="form-row"><label>{check("keepalive", "conn.keepalive")}</label><div className="form-control">{text("keepaliveSec", { width: 72, disabled: !form.keepalive })}</div></div>
      <div className="form-row"><label>{check("autoConnect", "conn.autoConnect")}</label><div className="form-control" /></div>
      <div className="form-row"><label>{check("limitSessions", "conn.limitSessions")}</label><div className="form-control">{text("sessionLimit", { width: 72, disabled: !form.limitSessions })}</div></div>
      {row("conn.parameters", browse("parameters", false))}
      {row("conn.initialQueries", browse("initialQueries", false))}
    </div>
  );

  const databasesTab = (
    <div className="form-grid form-grid-wide">
      {check("customDbList", "conn.customDbList")}
      <div className="mini-grid">
        <div className="mini-grid-head"><span>{t("info.database")}</span><span>{t("conn.autoOpen")}</span></div>
        {form.databases.map((d) => (
          <div key={d.name} className="mini-grid-row"><span>{d.name}</span><span><input type="checkbox" checked={d.autoOpen} readOnly /></span></div>
        ))}
      </div>
      <div className="form-buttons-left">
        <button type="button" className="btn" disabled>{t("conn.addDb")}</button>
        <button type="button" className="btn" disabled>{t("conn.removeDb")}</button>
      </div>
    </div>
  );

  const sslTab = (
    <div className="form-grid form-grid-wide">
      {check("useSsl", "conn.useSsl")}
      {row("conn.sslMode", (
        <select className="field" style={{ width: 200 }} disabled={!form.useSsl} value={form.sslMode} onChange={(e) => set("sslMode", e.target.value)}>
          {["require", "verify-ca", "verify-full"].map((m) => <option key={m}>{m}</option>)}
        </select>
      ), !form.useSsl)}
      <fieldset className="group" disabled={!form.useSsl}>
        <legend>{t("conn.authentication")}</legend>
        {check("sslAuth", "conn.useAuth", !form.useSsl)}
        {row("conn.clientKey", browse("sslClientKey", !form.useSsl || !form.sslAuth), !form.useSsl || !form.sslAuth)}
        {row("conn.clientCert", browse("sslClientCert", !form.useSsl || !form.sslAuth), !form.useSsl || !form.sslAuth)}
        {row("conn.rootCert", browse("sslRootCert", !form.useSsl || !form.sslAuth), !form.useSsl || !form.sslAuth)}
        {row("conn.crl", browse("sslCrl", !form.useSsl || !form.sslAuth), !form.useSsl || !form.sslAuth)}
      </fieldset>
      {form.useSsl && form.sslMode === "require" && <div className="form-note form-note-warn">{t("conn.sslRequireNote")}</div>}
    </div>
  );

  const sshOff = !form.useSsh;
  const sshTab = (
    <div className="form-grid">
      {check("useSsh", "conn.useSsh")}
      {row("conn.host", text("sshHost", { disabled: sshOff }), sshOff)}
      {row("conn.port", text("sshPort", { width: 48, disabled: sshOff }), sshOff)}
      {row("conn.user", text("sshUser", { width: 200, disabled: sshOff }), sshOff)}
      {row("conn.authMethod", (
        <select className="field" style={{ width: 200 }} disabled={sshOff} value={form.sshAuth} onChange={(e) => set("sshAuth", e.target.value as "password" | "publickey")}>
          <option value="password">{t("conn.authPassword")}</option>
          <option value="publickey">{t("conn.authPublicKey")}</option>
        </select>
      ), sshOff)}
      {form.sshAuth === "password"
        ? row("conn.password", text("sshPassword", { width: 200, type: "password", disabled: sshOff }), sshOff)
        : row("conn.privateKey", browse("sshKeyPath", sshOff), sshOff)}
      <div className="form-row"><label /><div className="form-control">{check("sshSavePassword", "conn.savePassword", sshOff)}</div></div>
      {!sshOff && <div className="form-note">{t("conn.sshHostKeyNote")}</div>}
    </div>
  );

  const remarksTab = (
    <div className="form-grid form-grid-wide form-grid-fill">
      <label>{t("conn.remarks")}:</label>
      <textarea className="field remarks" value={form.remarks} onChange={(e) => set("remarks", e.target.value)} />
    </div>
  );

  const formStep = (
    <div className="conn-form-wrap">
      {profilesOpen && (
        <aside className="conn-profiles">
          <div className="conn-profiles-head">
            <span>{t("conn.profiles")}</span>
            <button type="button" className="icon-button" disabled title={t("notImplemented")}><SmallIcon name="new" /></button>
          </div>
          <div className="conn-profile is-active">{t("conn.mainProfile")} <span className="muted">({t("conn.active")})</span></div>
        </aside>
      )}
      <div className="conn-form">
        <div className="dlg-tabs" role="tablist">
          {TABS.map((x) => (
            <button key={x.id} type="button" role="tab" aria-selected={tab === x.id} className={`dlg-tab${tab === x.id ? " is-active" : ""}`} onClick={() => setTab(x.id)}>
              {t(x.label)}
            </button>
          ))}
        </div>
        <div className="dlg-tab-body">
          {diagram}
          {tab === "general" && generalTab}
          {tab === "advanced" && advancedTab}
          {tab === "databases" && databasesTab}
          {tab === "ssl" && sslTab}
          {tab === "ssh" && sshTab}
          {tab === "remarks" && remarksTab}
        </div>
      </div>
    </div>
  );

  return (
    <div className="modal-overlay" role="presentation">
      <div className="window" role="dialog" aria-modal="true" aria-label={title} style={{ width: 902, height: 732 }}>
        <div className="window-title">
          {step === "form" ? <ProviderIcon provider={form.provider} open /> : <SmallIcon name="myConnections" />}
          <span>{title}</span>
          <button type="button" className="window-close" aria-label={t("tab.close")} onClick={onClose}><SmallIcon name="close" /></button>
        </div>

        <div className="window-body">{step === "type" ? typeStep : formStep}</div>

        {step === "form" && test.kind === "ok" && (
          <div className="test-strip is-ok">
            <SmallIcon name="success" />
            <strong>{t("conn.testOk")}</strong>
            <span>{t("conn.serverVersion")}: {test.version}</span>
          </div>
        )}
        {step === "form" && test.kind === "fail" && (
          <div className="test-strip is-fail" role="alert">
            <SmallIcon name="warning" />
            <strong>{t("conn.testFailed")}</strong>
            <span className="test-strip-msg">{test.message}</span>
            <button type="button" className="icon-button" onClick={() => setTest({ kind: "idle" })} aria-label={t("tab.close")}><SmallIcon name="close" /></button>
          </div>
        )}

        <div className="window-footer">
          {step === "type" ? (
            <>
              <button type="button" className="btn" disabled title={t("notImplemented")}>{t("conn.newWithUri")}</button>
              <span className="spacer" />
              <button type="button" className="btn btn-primary" disabled={!picked.available} onClick={() => goForm(picked)}>{t("conn.next")}</button>
              <button type="button" className="btn" onClick={onClose}>{t("conn.cancel")}</button>
            </>
          ) : (
            <>
              <button type="button" className={`btn btn-icon${profilesOpen ? " is-active" : ""}`} title={t("conn.profiles")} onClick={() => setProfilesOpen((o) => !o)}><SmallIcon name="sidebar" /></button>
              <button type="button" className="btn btn-wide" onClick={runTest} disabled={test.kind === "running"}>{t("conn.test")}</button>
              <button type="button" className="btn" disabled title={t("notImplemented")}>URI...</button>
              <span className="spacer" />
              {!editing && <button type="button" className="btn" onClick={() => setStep("type")}>{t("conn.back")}</button>}
              <button type="button" className="btn btn-primary" onClick={submit}>OK</button>
              <button type="button" className="btn" onClick={onClose}>{t("conn.cancel")}</button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
