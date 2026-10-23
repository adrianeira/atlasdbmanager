import { useCallback, useEffect, useMemo, useState } from "react";
import { chooseExportPath, errorText, pg, toAtlasError, type DataTransferConn, type DataTransferParams } from "../../backend";
import { useT } from "../../i18n";
import { ProviderIcon, SmallIcon } from "../../icons";
import { Dropdown } from "../Dropdown";
import { MessageDialog } from "../MessageDialog";

export interface Endpoint {
  connId: string;
  database: string;
  schema: string;
}

export function Arrow() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
      <path d="M2 8h10M8.5 4.5 12 8l-3.5 3.5" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/**
 * Origem e destino das janelas Data Transfer e Data Synchronization: conexão, banco e schema de cada lado, abrir a conexão
 * escolhida (senha do cofre; pede só se faltar), listas de bancos e schemas, cabeçalho, painéis de informação e a troca.
 */
export function useEndpoints(params: DataTransferParams, opts: { allowFile: boolean; onSourceChange?: () => void }) {
  const t = useT();
  // Conexões que a janela abriu sozinha (a janela principal continua achando que estão fechadas, e tudo bem).
  const [opened, setOpened] = useState<Record<string, { version: string; versionNum: number }>>({});
  const [connecting, setConnecting] = useState<string | null>(null);
  const [pwPrompt, setPwPrompt] = useState<{ connId: string; retry?: string } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const conns = useMemo(
    () => params.connections.map((c) => (opened[c.id] ? { ...c, open: true, version: opened[c.id].version, versionNum: opened[c.id].versionNum } : c)),
    [params.connections, opened],
  );
  const connOf = useCallback((id: string) => conns.find((c) => c.id === id), [conns]);
  const first = conns.find((c) => c.open) ?? conns[0];
  const initial: Endpoint = { connId: params.source?.connId ?? first?.id ?? "", database: params.source?.database ?? "", schema: params.source?.schema ?? "" };

  const [source, setSource] = useState<Endpoint>(initial);
  const [target, setTarget] = useState<Endpoint>(initial);
  const [targetKind, setTargetKind] = useState<"connection" | "file">("connection");
  const [filePath, setFilePath] = useState("");
  const [lists, setLists] = useState<Record<string, string[]>>({});
  const sourceConn = connOf(source.connId);
  const targetConn = connOf(target.connId);

  const openConn = async (id: string, password?: string) => {
    const c = params.connections.find((x) => x.id === id);
    if (!c || c.open || opened[id] || connecting === id) return;
    if (password === undefined && !c.hasPassword) {
      setPwPrompt((p) => p ?? { connId: id });
      return;
    }
    setConnecting(id);
    setMessage(null);
    try {
      const info = await pg.open(id, { host: c.host, port: c.port, database: c.initialDatabase || "postgres", user: c.user, password: password ?? "", sslMode: c.sslMode ?? "disable" }, id);
      setOpened((o) => ({ ...o, [id]: { version: info.version, versionNum: info.versionNum } }));
    } catch (e) {
      const err = toAtlasError(e);
      if (err.kind === "authenticationFailed") setPwPrompt({ connId: id, retry: errorText(err) });
      else setMessage(errorText(err));
    } finally {
      setConnecting(null);
    }
  };

  useEffect(() => {
    for (const id of new Set([source.connId, targetKind === "connection" ? target.connId : ""])) if (id) void openConn(id);
    // Só reage à escolha da conexão.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source.connId, target.connId, targetKind]);

  const loadDatabases = useCallback(async (connId: string) => {
    if (!conns.find((x) => x.id === connId)?.open) return;
    try {
      const r = await pg.listDatabases(connId);
      setLists((l) => ({ ...l, [`db:${connId}`]: r.databases }));
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  }, [conns]);

  const loadSchemas = useCallback(async (connId: string, database: string) => {
    if (!database || !conns.find((x) => x.id === connId)?.open) return;
    try {
      const r = await pg.listSchemas(connId, database);
      setLists((l) => ({ ...l, [`sc:${connId}/${database}`]: r.map((x) => (typeof x === "string" ? x : (x as { name: string }).name)) }));
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  }, [conns]);

  useEffect(() => { void loadDatabases(source.connId); }, [source.connId, loadDatabases]);
  useEffect(() => { void loadSchemas(source.connId, source.database); }, [source.connId, source.database, loadSchemas]);
  useEffect(() => { if (targetKind === "connection") void loadDatabases(target.connId); }, [target.connId, targetKind, loadDatabases]);
  useEffect(() => { if (targetKind === "connection") void loadSchemas(target.connId, target.database); }, [target.connId, target.database, targetKind, loadSchemas]);

  // Escolhe o primeiro banco/schema quando a lista chega e a escolha atual não existe nela.
  const pick = (list: string[] | undefined, cur: string, prefer?: string) => (!list || list.length === 0 || list.includes(cur) ? cur : list.includes(prefer ?? "") ? (prefer as string) : list[0]);
  const sourceDbs = lists[`db:${source.connId}`];
  const sourceSchemas = lists[`sc:${source.connId}/${source.database}`];
  const targetDbs = lists[`db:${target.connId}`];
  const targetSchemas = lists[`sc:${target.connId}/${target.database}`];
  useEffect(() => {
    const db = pick(sourceDbs, source.database);
    if (db !== source.database) setSource((s) => ({ ...s, database: db, schema: "" }));
  }, [sourceDbs, source.database]);
  useEffect(() => {
    const sc = pick(sourceSchemas, source.schema, "public");
    if (sc !== source.schema) setSource((s) => ({ ...s, schema: sc }));
  }, [sourceSchemas, source.schema]);
  useEffect(() => {
    const db = pick(targetDbs, target.database, source.database);
    if (db !== target.database) setTarget((s) => ({ ...s, database: db, schema: "" }));
  }, [targetDbs, target.database, source.database]);
  useEffect(() => {
    const sc = pick(targetSchemas, target.schema, source.schema);
    if (sc !== target.schema) setTarget((s) => ({ ...s, schema: sc }));
  }, [targetSchemas, target.schema, source.schema]);

  const sourceReady = !!sourceConn?.open && !!source.database && !!source.schema;
  const targetReady = targetKind === "file" ? true : !!targetConn?.open && !!target.database && !!target.schema;
  const sameTarget = targetKind === "connection" && source.connId === target.connId && source.database === target.database && source.schema === target.schema;

  /** O que falta para sair da primeira etapa (origem e destino escolhidos e abertos), ou `null`. */
  const endpointProblem = (): string | null => {
    if (!sourceReady) return sourceConn && !sourceConn.open ? t("dt.needOpen", { name: sourceConn.name }) : t("dt.needSource");
    if (targetKind === "connection" && targetConn && !targetConn.open) return t("dt.needOpen", { name: targetConn.name });
    if (!targetReady) return t("dt.needTarget");
    if (sameTarget) return t("dt.sameTarget");
    return null;
  };

  const swap = () => {
    if (targetKind !== "connection") return;
    const s = source;
    setSource(target);
    setTarget(s);
    opts.onSourceChange?.();
  };

  const chooseFile = async () => {
    try {
      const picked = await chooseExportPath(`${source.schema || "transfer"}.sql`, ["sql"]);
      if (picked) setFilePath(picked);
    } catch (e) {
      setMessage(errorText(toAtlasError(e)));
    }
  };

  const connItems = conns.map((c) => ({ value: c.id, label: c.name }));
  const connIcon = (c?: DataTransferConn) => (c ? <ProviderIcon provider={c.provider as never} open={c.open} /> : <SmallIcon name="myConnections" />);

  const info = (c?: DataTransferConn) => (
    <dl className="dt-info">
      <dt>{t("dt.info.cloud")}</dt><dd>--</dd>
      <dt>{t("dt.info.project")}</dt><dd>{t("nav.myConnections")}</dd>
      <dt>{t("dt.info.type")}</dt><dd>{c ? "PostgreSQL" : "--"}</dd>
      <dt>{t("dt.info.name")}</dt><dd>{c?.name ?? "--"}</dd>
      <dt>{t("dt.info.host")}</dt><dd>{c?.host ?? "--"}</dd>
      <dt>{t("dt.info.port")}</dt><dd>{c?.port ?? "--"}</dd>
      <dt>{t("dt.info.version")}</dt><dd>{c?.versionNum || "--"}</dd>
    </dl>
  );

  const label = (conn: DataTransferConn | undefined, e: Endpoint, file = false) =>
    file ? [t("dt.file"), filePath ? filePath.split(/[\\/]/).pop() ?? "" : "—"] : [conn?.name ?? "—", e.database && e.schema ? `${e.database}.${e.schema}` : "—"];
  const [srcA, srcB] = label(sourceConn, source);
  const [dstA, dstB] = label(targetConn, target, targetKind === "file");

  const header = (
    <div className="dt-head">
      <div className="dt-end is-left"><span>{srcA}</span><span>{srcB}</span></div>
      <SmallIcon name="database" size={26} />
      <span className="dt-arrow"><Arrow /></span>
      <span className="dt-target-icon"><SmallIcon name="database" size={26} /></span>
      <div className="dt-end"><span>{dstA}</span><span>{dstB}</span></div>
    </div>
  );

  const changed = () => opts.onSourceChange?.();
  const connectStep = (
    <div className="dt-columns">
      <section className="dt-side">
        <h2 className="dt-h">{t("dt.source")}</h2>
        <label>{t("dt.connection")}</label>
        <Dropdown className="dt-dd" value={source.connId} items={connItems} leftIcon={connIcon(sourceConn)} onChange={(v) => { setSource({ connId: v, database: "", schema: "" }); changed(); }} />
        <label>{t("dt.database")}</label>
        <Dropdown className="dt-dd" value={source.database} items={(sourceDbs ?? []).map((d) => ({ value: d, label: d }))} leftIcon={<SmallIcon name="database" />}
          onChange={(v) => { setSource((s) => ({ ...s, database: v, schema: "" })); changed(); }} disabled={!sourceConn?.open} />
        <label>{t("dt.schema")}</label>
        <Dropdown className="dt-dd" value={source.schema} items={(sourceSchemas ?? []).map((d) => ({ value: d, label: d }))} leftIcon={<SmallIcon name="schema" />}
          onChange={(v) => { setSource((s) => ({ ...s, schema: v })); changed(); }} disabled={!sourceConn?.open} />
        {connecting === source.connId && <div className="muted dt-loading"><span className="spinner" />{t("dt.connecting")}</div>}
        <h2 className="dt-h dt-h-gap">{t("dt.information")}</h2>
        {info(sourceConn)}
      </section>
      <button type="button" className="dt-swap" title={t("dt.swap")} aria-label={t("dt.swap")} onClick={swap} disabled={targetKind !== "connection"}>
        <svg width="26" height="26" viewBox="0 0 26 26" aria-hidden="true"><path d="M5 9h15M16 5l4 4-4 4M21 17H6M10 13l-4 4 4 4" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      <section className="dt-side">
        <h2 className="dt-h">{t("dt.target")}</h2>
        {opts.allowFile ? (
          <div className="dt-radios">
            <label><input type="radio" name="dt-kind" checked={targetKind === "connection"} onChange={() => setTargetKind("connection")} />{t("dt.connection")}</label>
            <label><input type="radio" name="dt-kind" checked={targetKind === "file"} onChange={() => setTargetKind("file")} />{t("dt.file")}</label>
          </div>
        ) : <label>{t("dt.connection")}</label>}
        {targetKind === "connection" ? (
          <>
            <Dropdown className="dt-dd" value={target.connId} items={connItems} leftIcon={connIcon(targetConn)} onChange={(v) => setTarget({ connId: v, database: "", schema: "" })} />
            <label>{t("dt.database")}</label>
            <Dropdown className="dt-dd" value={target.database} items={(targetDbs ?? []).map((d) => ({ value: d, label: d }))} leftIcon={<SmallIcon name="database" />}
              onChange={(v) => setTarget((s) => ({ ...s, database: v, schema: "" }))} disabled={!targetConn?.open} />
            <label>{t("dt.schema")}</label>
            <Dropdown className="dt-dd" value={target.schema} items={(targetSchemas ?? []).map((d) => ({ value: d, label: d }))} leftIcon={<SmallIcon name="schema" />}
              onChange={(v) => setTarget((s) => ({ ...s, schema: v }))} disabled={!targetConn?.open} />
          </>
        ) : (
          <>
            <label>{t("dt.fileSql")}</label>
            <div className="dt-file-row">
              <input className="field" value={filePath} readOnly placeholder={t("dt.pickFile")} spellCheck={false} onClick={() => void chooseFile()} />
              <button type="button" className="btn" onClick={() => void chooseFile()}>…</button>
            </div>
          </>
        )}
        {targetKind === "connection" && connecting === target.connId && <div className="muted dt-loading"><span className="spinner" />{t("dt.connecting")}</div>}
        <h2 className="dt-h dt-h-gap">{t("dt.information")}</h2>
        {targetKind === "connection" ? info(targetConn) : <p className="muted">{t("dt.fileNote")}</p>}
      </section>
    </div>
  );

  const passwordDialog = pwPrompt && (
    <MessageDialog
      title={t("dt.title")}
      message={[t("dt.passwordFor", { name: connOf(pwPrompt.connId)?.name ?? "" }), pwPrompt.retry].filter(Boolean).join("\n\n")}
      prompt={{ label: t("dt.password"), initial: "" }}
      secret
      allowEmpty
      onOk={(pw) => { const id = pwPrompt.connId; setPwPrompt(null); void openConn(id, pw ?? ""); }}
      onCancel={() => setPwPrompt(null)}
    />
  );

  return {
    conns, connOf, source, setSource, target, setTarget, targetKind, setTargetKind, filePath, setFilePath, sourceConn, targetConn,
    message, setMessage, header, connectStep, passwordDialog, endpointProblem, sourceReady,
  };
}
