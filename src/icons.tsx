/*
 * Iconografia própria do Atlas (SVG desenhado para o projeto).
 * Equivalência semântica com a referência, sem reproduzir ícones proprietários.
 */
import type { ReactElement } from "react";
import { siMariadb, siMongodb, siMysql, siPostgresql, siRedis, siSqlite, type SimpleIcon } from "simple-icons";
import type { Category, ProviderId } from "./model";

const PLUS_BADGE = (
  <g>
    <circle cx="24.5" cy="24.5" r="6" fill="#2e9e4f" stroke="#fff" strokeWidth="1.5" />
    <path d="M24.5 21.5v6M21.5 24.5h6" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" />
  </g>
);

/* ---------- Ícones grandes (32px) da Main Toolbar ---------- */

const large: Record<string, ReactElement> = {
  connection: (
    <>
      <path d="M6 26l5-5" stroke="#6b7480" strokeWidth="3" strokeLinecap="round" />
      <rect x="9" y="9" width="12" height="12" rx="3" transform="rotate(45 15 15)" fill="#8a94a1" />
      <path d="M17.5 8.5l3-3M21.5 12.5l3-3" stroke="#6b7480" strokeWidth="2.6" strokeLinecap="round" />
      {PLUS_BADGE}
    </>
  ),
  newQuery: (
    <>
      <rect x="4" y="5" width="19" height="21" rx="2" fill="#fff" stroke="#3d7fc4" strokeWidth="1.5" />
      <rect x="4" y="5" width="19" height="5" rx="2" fill="#e8574a" />
      <path d="M8 14h11M8 18h8M8 22h10" stroke="#3d7fc4" strokeWidth="1.6" strokeLinecap="round" />
      {PLUS_BADGE}
    </>
  ),
  table: (
    <>
      <rect x="3.5" y="5.5" width="25" height="21" rx="2" fill="#fff" stroke="#2f7fd0" strokeWidth="1.5" />
      <path d="M3.5 7.5a2 2 0 0 1 2-2h21a2 2 0 0 1 2 2V11h-25z" fill="#2f7fd0" />
      <path d="M3.5 16.5h25M3.5 21.5h25M12 11v15.5M20 11v15.5" stroke="#9cc4ec" strokeWidth="1.3" />
    </>
  ),
  view: (
    <>
      <rect x="3.5" y="5.5" width="25" height="15" rx="2" fill="#fff" stroke="#2f7fd0" strokeWidth="1.5" />
      <path d="M3.5 7.5a2 2 0 0 1 2-2h21a2 2 0 0 1 2 2V9.5h-25z" fill="#2f7fd0" />
      <circle cx="10.5" cy="21.5" r="5" fill="#fff" stroke="#24466b" strokeWidth="2" />
      <circle cx="21.5" cy="21.5" r="5" fill="#fff" stroke="#24466b" strokeWidth="2" />
      <path d="M15.5 21h1" stroke="#24466b" strokeWidth="2" />
    </>
  ),
  matview: (
    <>
      <rect x="3.5" y="5.5" width="25" height="15" rx="2" fill="#fbe6e3" stroke="#e8574a" strokeWidth="1.5" />
      <path d="M3.5 7.5a2 2 0 0 1 2-2h21a2 2 0 0 1 2 2V10.5h-25z" fill="#e8574a" />
      <circle cx="10.5" cy="21.5" r="5" fill="#fff" stroke="#8f2a21" strokeWidth="2" />
      <circle cx="21.5" cy="21.5" r="5" fill="#fff" stroke="#8f2a21" strokeWidth="2" />
      <path d="M15.5 21h1" stroke="#8f2a21" strokeWidth="2" />
    </>
  ),
  function: (
    <>
      <text x="16" y="23" textAnchor="middle" fontFamily="Georgia, 'Times New Roman', serif" fontStyle="italic" fontSize="19" fill="#2f6fb5">
        f(x)
      </text>
    </>
  ),
  user: (
    <>
      <circle cx="16" cy="10.5" r="5.5" fill="#f3c27a" stroke="#c98a3a" strokeWidth="1" />
      <path d="M6 27c0-6 4.5-9.5 10-9.5S26 21 26 27z" fill="#e9852f" />
      <path d="M13 18l3 4 3-4" fill="#fff" />
    </>
  ),
  others: (
    <>
      <rect x="4" y="15" width="24" height="12" rx="2" fill="#2f7fd0" />
      <path d="M10 15V6M10 6l-3 3M22 15V8" stroke="#56606b" strokeWidth="2.6" strokeLinecap="round" />
      <circle cx="22" cy="7" r="3" fill="none" stroke="#56606b" strokeWidth="2" />
      <path d="M8 21h16" stroke="#9cc4ec" strokeWidth="1.5" />
    </>
  ),
  query: (
    <>
      <rect x="8.5" y="4.5" width="18" height="16" rx="2" fill="#fff" stroke="#e8574a" strokeWidth="1.5" />
      <rect x="4.5" y="10.5" width="18" height="16" rx="2" fill="#fff" stroke="#2f7fd0" strokeWidth="1.5" />
      <path d="M4.5 12.5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2V15h-18z" fill="#2f7fd0" />
      <path d="M8 19h11M8 22.5h7" stroke="#2f7fd0" strokeWidth="1.5" strokeLinecap="round" />
    </>
  ),
  backup: (
    <>
      <rect x="5" y="5" width="22" height="22" rx="4" fill="#5b6470" />
      <path d="M11 16a5 5 0 1 0 1.6-3.7" fill="none" stroke="#fff" strokeWidth="2" strokeLinecap="round" />
      <path d="M11.5 10v3.3h3.3" fill="none" stroke="#fff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  automation: (
    <>
      <path d="M16 3v4" stroke="#3a4552" strokeWidth="2" />
      <circle cx="16" cy="3.5" r="2" fill="#e8574a" />
      <rect x="6" y="7" width="20" height="14" rx="5" fill="#33b3a6" />
      <circle cx="12" cy="14" r="2.6" fill="#fff" />
      <circle cx="20" cy="14" r="2.6" fill="#fff" />
      <circle cx="12" cy="14" r="1.2" fill="#1f3d4a" />
      <circle cx="20" cy="14" r="1.2" fill="#1f3d4a" />
      <rect x="9" y="22" width="14" height="6" rx="2" fill="#2a8f85" />
      <circle cx="25" cy="25" r="4" fill="#fff" stroke="#3a4552" strokeWidth="1.5" />
      <path d="M25 23v2.2l1.4 1" stroke="#3a4552" strokeWidth="1.3" strokeLinecap="round" fill="none" />
    </>
  ),
  model: (
    <>
      <rect x="3.5" y="3.5" width="11" height="9" rx="1.5" fill="#f2a33a" />
      <rect x="17.5" y="11.5" width="11" height="9" rx="1.5" fill="#e8742e" />
      <rect x="6.5" y="19.5" width="11" height="9" rx="1.5" fill="#f2a33a" />
      <path d="M9 12.5v7M14.5 8h6v3.5M17.5 24h3v-3.5" stroke="#8b4b14" strokeWidth="1.6" fill="none" />
    </>
  ),
  bi: (
    <>
      <rect x="4" y="15" width="5" height="12" rx="1.2" fill="#c13fd6" />
      <rect x="10.5" y="8" width="5" height="19" rx="1.2" fill="#e8408f" />
      <rect x="17" y="12" width="5" height="15" rx="1.2" fill="#8c4fe0" />
      <rect x="23.5" y="5" width="5" height="22" rx="1.2" fill="#d43fb0" />
    </>
  ),
};

export type LargeIconName = keyof typeof large;

export function LargeIcon({ name }: { name: LargeIconName }) {
  return (
    <svg width="32" height="32" viewBox="0 0 32 32" aria-hidden="true">
      {large[name]}
    </svg>
  );
}

/* ---------- Ícones pequenos (16px) ---------- */

const small: Record<string, ReactElement> = {
  open: <path d="M1.5 4.5v8.5h11l2-6h-10l-2 6M1.5 4.5h4l1.5 1.5h5V7" fill="none" stroke="currentColor" strokeLinejoin="round" />,
  design: (
    <>
      <path d="M3 13l1-3.5 7-7 2.5 2.5-7 7z" fill="none" stroke="currentColor" strokeLinejoin="round" />
      <path d="M9.5 4l2.5 2.5" stroke="currentColor" />
    </>
  ),
  new: (
    <>
      <circle cx="8" cy="8" r="6" fill="none" stroke="#2e9e4f" />
      <path d="M8 5v6M5 8h6" stroke="#2e9e4f" strokeLinecap="round" />
    </>
  ),
  delete: (
    <>
      <circle cx="8" cy="8" r="6" fill="none" stroke="#d13b2f" />
      <path d="M5 8h6" stroke="#d13b2f" strokeLinecap="round" />
    </>
  ),
  import: (
    <>
      <path d="M5.5 2.5h6l2 2v9h-8z" fill="none" stroke="currentColor" />
      <path d="M1.5 9h6M5 6.5L7.5 9 5 11.5" fill="none" stroke="#2e9e4f" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  export: (
    <>
      <path d="M2.5 2.5h6l2 2v9h-8z" fill="none" stroke="currentColor" />
      <path d="M8 9h6.5M12 6.5L14.5 9 12 11.5" fill="none" stroke="#d98a1c" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  execute: <path d="M5 3l8 5-8 5z" fill="#2e9e4f" />,
  privilege: (
    <>
      <rect x="3.5" y="7.5" width="9" height="6.5" rx="1" fill="none" stroke="currentColor" />
      <path d="M5.5 7.5V5.5a2.5 2.5 0 0 1 5 0v2" fill="none" stroke="currentColor" />
    </>
  ),
  search: (
    <>
      <circle cx="6.5" cy="6.5" r="4.2" fill="none" stroke="currentColor" />
      <path d="M9.6 9.6l4 4" stroke="currentColor" strokeLinecap="round" />
    </>
  ),
  filter: <path d="M2 4h12M4 8h8M6.5 12h3" stroke="currentColor" strokeLinecap="round" />,
  star: <path d="M8 1.8l1.9 3.9 4.3.6-3.1 3 .7 4.3L8 11.6l-3.8 2 .7-4.3-3.1-3 4.3-.6z" fill="none" stroke="currentColor" strokeLinejoin="round" />,
  starFilled: <path d="M8 1.8l1.9 3.9 4.3.6-3.1 3 .7 4.3L8 11.6l-3.8 2 .7-4.3-3.1-3 4.3-.6z" fill="#f2b01e" stroke="#c98a0a" strokeLinejoin="round" />,
  colorDot: <circle cx="8" cy="8" r="5" fill="currentColor" />,
  link: (
    <>
      <path d="M6.8 9.2l2.4-2.4" stroke="#2f7fd0" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M7.6 4.6l1.2-1.2a2.5 2.5 0 0 1 3.6 3.6l-1.2 1.2M8.4 11.4l-1.2 1.2a2.5 2.5 0 0 1-3.6-3.6l1.2-1.2" fill="none" stroke="#2f7fd0" strokeWidth="1.6" strokeLinecap="round" />
    </>
  ),
  pin: <path d="M6 2h4l-.6 4 2.1 2.2v1H8.6V14H7.4V9.2H4.5v-1L6.6 6z" fill="none" stroke="currentColor" strokeLinejoin="round" />,
  collapseAll: <path d="M4 3l4 4 4-4M4 9l4 4 4-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" transform="rotate(180 8 8)" />,
  chevronRight: <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />,
  chevronDown: <path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />,
  close: <path d="M4.5 4.5l7 7M11.5 4.5l-7 7" stroke="currentColor" strokeLinecap="round" />,
  check: <path d="M3.5 8.5l3 3 6-7" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />,
  myConnections: (
    <>
      <path d="M3 13l3-3" stroke="#6b7480" strokeWidth="1.8" strokeLinecap="round" />
      <rect x="5" y="5" width="6.5" height="6.5" rx="1.5" transform="rotate(45 8.25 8.25)" fill="#8a94a1" />
      <path d="M10 4.5l1.8-1.8M12 6.5l1.8-1.8" stroke="#6b7480" strokeWidth="1.5" strokeLinecap="round" />
    </>
  ),
  database: (
    <>
      <path d="M3 3.5v9c0 1.1 2.2 2 5 2s5-.9 5-2v-9" fill="#2fb36b" />
      <ellipse cx="8" cy="3.5" rx="5" ry="2" fill="#5fd393" />
      <path d="M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" fill="none" stroke="#1f8a50" strokeWidth=".8" />
    </>
  ),
  databaseClosed: (
    <>
      <path d="M3 3.5v9c0 1.1 2.2 2 5 2s5-.9 5-2v-9" fill="#7d7d7d" />
      <ellipse cx="8" cy="3.5" rx="5" ry="2" fill="#a8a8a8" />
      <path d="M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" fill="none" stroke="#5c5c5c" strokeWidth=".8" />
    </>
  ),
  schema: (
    <>
      <rect x="5.5" y="1.5" width="5" height="4" rx=".8" fill="#2fb36b" />
      <rect x="1.5" y="10.5" width="5" height="4" rx=".8" fill="#2f7fd0" />
      <rect x="9.5" y="10.5" width="5" height="4" rx=".8" fill="#2f7fd0" />
      <path d="M8 5.5v2.5M4 10.5V8h8v2.5" fill="none" stroke="#56606b" />
    </>
  ),
  schemaGray: (
    <>
      <rect x="5.5" y="1.5" width="5" height="4" rx=".8" fill="#8a8a8a" />
      <rect x="1.5" y="10.5" width="5" height="4" rx=".8" fill="#a8a8a8" />
      <rect x="9.5" y="10.5" width="5" height="4" rx=".8" fill="#a8a8a8" />
      <path d="M8 5.5v2.5M4 10.5V8h8v2.5" fill="none" stroke="#6b6b6b" />
    </>
  ),
  column: (
    <>
      <rect x="2.5" y="4.5" width="11" height="7" rx="1" fill="#fff" stroke="#8c8c8c" />
      <path d="M6 4.5v7" stroke="#8c8c8c" />
      <rect x="2.5" y="4.5" width="3.5" height="7" fill="#2f7fd0" opacity=".5" />
    </>
  ),
  // Coluna da chave primária: coluna + chave dourada.
  columnPk: (
    <>
      <rect x="1.5" y="2.5" width="10" height="6.5" rx="1" fill="#fff" stroke="#8c8c8c" />
      <path d="M4.8 2.5v6.5" stroke="#8c8c8c" />
      <rect x="1.5" y="2.5" width="3.3" height="6.5" fill="#2f7fd0" opacity=".5" />
      <circle cx="9.3" cy="10.3" r="2.1" fill="#fff" stroke="#d99a0b" strokeWidth="1.5" />
      <path d="M10.8 11.8l3.4 3.2M12.6 13.5l1-1" stroke="#d99a0b" strokeWidth="1.5" strokeLinecap="round" />
    </>
  ),
  // Coluna de chave estrangeira: coluna + elo azul.
  columnFk: (
    <>
      <rect x="1.5" y="2.5" width="10" height="6.5" rx="1" fill="#fff" stroke="#8c8c8c" />
      <path d="M4.8 2.5v6.5" stroke="#8c8c8c" />
      <rect x="1.5" y="2.5" width="3.3" height="6.5" fill="#2f7fd0" opacity=".5" />
      <path d="M10.6 12.4l1.6-1.6" stroke="#1f6fc4" strokeWidth="1.4" strokeLinecap="round" />
      <path d="M11.2 9.7l.8-.8a1.7 1.7 0 0 1 2.4 2.4l-.8.8M11.6 13.5l-.8.8a1.7 1.7 0 0 1-2.4-2.4l.8-.8" fill="#fff" stroke="#1f6fc4" strokeWidth="1.4" strokeLinecap="round" />
    </>
  ),
  // Formatos do assistente de exportação (desenho próprio).
  fmtCsv: (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#6b7785" strokeLinejoin="round" />
      <path d="M9.5 1.5v3h3" fill="none" stroke="#6b7785" strokeLinejoin="round" />
      <path d="M5.2 7.2h5.6M5.2 9.6h5.6M5.2 12h5.6M7.9 7.2V12" stroke="#2f9e8f" strokeWidth="1.2" />
    </>
  ),
  fmtTxt: (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#6b7785" strokeLinejoin="round" />
      <path d="M9.5 1.5v3h3" fill="none" stroke="#6b7785" strokeLinejoin="round" />
      <path d="M5.2 7h5.6M5.2 9h5.6M5.2 11h3.4" stroke="#8a94a0" strokeWidth="1.1" strokeLinecap="round" />
    </>
  ),
  fmtXml: (
    <>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#6b7785" strokeLinejoin="round" />
      <path d="M9.5 1.5v3h3" fill="none" stroke="#6b7785" strokeLinejoin="round" />
      <path d="M6.6 7.4L4.9 9.6l1.7 2.2M9.4 7.4l1.7 2.2-1.7 2.2" fill="none" stroke="#8a56c2" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  fmtHtml: (
    <>
      <circle cx="8" cy="8" r="6" fill="#fff" stroke="#e2762c" />
      <path d="M2 8h12M8 2c-2 1.7-2 10.3 0 12M8 2c2 1.7 2 10.3 0 12" fill="none" stroke="#e2762c" strokeWidth=".9" />
    </>
  ),
  fmtJson: (
    <path
      d="M5.6 2c-1.6 0-1.9.9-1.9 2.2v1.6c0 1-.5 1.6-1.4 2.2.9.6 1.4 1.2 1.4 2.2v1.6c0 1.3.3 2.2 1.9 2.2M10.4 2c1.6 0 1.9.9 1.9 2.2v1.6c0 1 .5 1.6 1.4 2.2-.9.6-1.4 1.2-1.4 2.2v1.6c0 1.3-.3 2.2-1.9 2.2"
      fill="none" stroke="#d98a1c" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"
    />
  ),
  fmtSql: (
    <>
      <ellipse cx="8" cy="4" rx="5" ry="2" fill="#dbe9f8" stroke="#2f7fd0" />
      <path d="M3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4" fill="none" stroke="#2f7fd0" />
      <path d="M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" fill="none" stroke="#2f7fd0" />
    </>
  ),
  fmtXlsx: (
    <>
      <rect x="2" y="2" width="12" height="12" rx="1.5" fill="#1f8a4c" />
      <path d="M2 6h12M2 10h12M6 2v12" stroke="#fff" strokeWidth="1" />
      <rect x="2" y="2" width="12" height="4" rx="1.5" fill="#fff" opacity=".18" />
    </>
  ),
  key: (
    <>
      <circle cx="5" cy="6" r="3" fill="none" stroke="#e0a412" strokeWidth="1.8" />
      <path d="M7.2 8.2l5.3 5.3M10.5 11.5l1.5-1.5M12 13l1.3-1.3" stroke="#e0a412" strokeWidth="1.8" strokeLinecap="round" />
    </>
  ),
  save: (
    <>
      <path d="M2.5 2.5h9l2 2v9h-11z" fill="#fff" stroke="#2f7fd0" strokeLinejoin="round" />
      <rect x="4.5" y="2.5" width="6" height="4" fill="#9cc4ec" stroke="#2f7fd0" />
      <rect x="4.5" y="9" width="7" height="4.5" fill="#fff" stroke="#2f7fd0" />
    </>
  ),
  run: <path d="M4.5 2.5l9 5.5-9 5.5z" fill="none" stroke="#2f7fd0" strokeLinejoin="round" />,
  stop: <rect x="3" y="3" width="10" height="10" rx="1.5" fill="none" stroke="#d13b2f" />,
  explain: (
    <>
      <rect x="1.5" y="2" width="5" height="4" fill="none" stroke="#2f7fd0" />
      <rect x="9.5" y="8" width="5" height="4" fill="none" stroke="#2f7fd0" />
      <path d="M4 6v4h5.5" fill="none" stroke="#56606b" />
      <path d="M2 13.5h4" stroke="#2fb36b" strokeWidth="1.5" />
    </>
  ),
  queryBuilder: <path d="M3 3h10M8 3v11M5.5 14h5" stroke="#d98a1c" strokeWidth="1.6" strokeLinecap="round" />,
  preview: (
    <>
      <path d="M1.5 8C3 5.2 5.3 3.8 8 3.8s5 1.4 6.5 4.2C13 10.8 10.7 12.2 8 12.2S3 10.8 1.5 8z" fill="none" stroke="#2f7fd0" strokeLinejoin="round" />
      <circle cx="8" cy="8" r="2" fill="#2f7fd0" />
    </>
  ),
  beautify: (
    <>
      <path d="M3 13l7-7" stroke="#56606b" strokeWidth="1.4" strokeLinecap="round" />
      <path d="M11 1.5l.6 1.4 1.4.6-1.4.6-.6 1.4-.6-1.4-1.4-.6 1.4-.6zM13.5 7l.4.9.9.4-.9.4-.4.9-.4-.9-.9-.4.9-.4z" fill="#2f7fd0" />
    </>
  ),
  snippet: <path d="M6 2.5c-2 0-2 1.5-2 3s-1.5 2.5-1.5 2.5S4 9 4 10.5s0 3 2 3M10 2.5c2 0 2 1.5 2 3s1.5 2.5 1.5 2.5S12 9 12 10.5s0 3-2 3" fill="none" stroke="#2f7fd0" />,
  ai: (
    <>
      <rect x="1.5" y="3.5" width="9" height="7" rx="2" fill="none" stroke="#2f7fd0" />
      <path d="M4 7h1M7 7h1" stroke="#2f7fd0" strokeLinecap="round" />
      <path d="M12.5 8.5l.7 1.6 1.6.7-1.6.7-.7 1.6-.7-1.6-1.6-.7 1.6-.7z" fill="#d98a1c" />
    </>
  ),
  transaction: (
    <>
      <rect x="2.5" y="2.5" width="11" height="11" rx="1" fill="none" stroke="#56606b" />
      <path d="M5 6h6M5 8.5h6M5 11h3" stroke="#56606b" />
    </>
  ),
  cellEditor: (
    <>
      <rect x="1.5" y="4.5" width="10" height="8" fill="none" stroke="#56606b" />
      <path d="M8 11l1-3 5-5 2 2-5 5z" fill="#fff" stroke="#2f7fd0" strokeLinejoin="round" />
    </>
  ),
  filterSort: (
    <>
      <path d="M1.5 3h8l-3 4v4l-2 1V7z" fill="none" stroke="#2f7fd0" strokeLinejoin="round" />
      <path d="M12.5 3v9M10.5 10l2 2 2-2" fill="none" stroke="#56606b" strokeLinecap="round" />
    </>
  ),
  columns: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="none" stroke="#2f7fd0" />
      <path d="M6 2.5v11M10.5 2.5v11" stroke="#2f7fd0" />
    </>
  ),
  profiling: (
    <>
      <path d="M2 14V9M5.5 14V5M9 14V7.5M12.5 14V3" stroke="#56606b" strokeWidth="1.8" />
    </>
  ),
  dataGen: (
    <>
      <rect x="1.5" y="2.5" width="10" height="9" fill="none" stroke="#2f7fd0" />
      <path d="M1.5 5.5h10M5 2.5v9" stroke="#2f7fd0" />
      <path d="M13 9.5l.8 1.7 1.7.8-1.7.8-.8 1.7-.8-1.7-1.7-.8 1.7-.8z" fill="#d98a1c" />
    </>
  ),
  biCreate: (
    <>
      <path d="M2.5 14V9M6 14V5M9.5 14V8M13 14V3" stroke="#8c4fe0" strokeWidth="1.8" />
    </>
  ),
  addRow: <path d="M8 3.5v9M3.5 8h9" stroke="currentColor" strokeLinecap="round" />,
  removeRow: <path d="M3.5 8h9" stroke="currentColor" strokeLinecap="round" />,
  apply: <path d="M3 8.5l3 3 7-7" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />,
  discard: <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeLinecap="round" />,
  history: (
    <>
      <path d="M2.5 8a5.5 5.5 0 1 0 1.7-4" fill="none" stroke="currentColor" strokeLinecap="round" />
      <path d="M2.4 1.8v3h3" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M8 5v3.2l2 1.3" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  refresh: (
    <>
      <path d="M13 8a5 5 0 1 1-1.6-3.7" fill="none" stroke="currentColor" strokeLinecap="round" />
      <path d="M11.8 1.8v2.8H9" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  stopSmall: <rect x="4" y="4" width="8" height="8" fill="currentColor" opacity=".55" />,
  first: <path d="M4 3.5v9M12 3.5L6.5 8l5.5 4.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />,
  prev: <path d="M13 8H3.5M7 4.5L3.5 8 7 11.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />,
  next: <path d="M3 8h9.5M9 4.5l3.5 3.5L9 11.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />,
  last: <path d="M12 3.5v9M4 3.5L9.5 8 4 12.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />,
  // Engrenagem: 8 dentes, furo central (evenodd).
  settings: (
    <path
      fillRule="evenodd"
      fill="currentColor"
      d="M13.24 6.70L15.13 6.99L15.13 9.01L13.24 9.30L12.63 10.79L13.76 12.32L12.32 13.76L10.79 12.63L9.30 13.24L9.01 15.13L6.99 15.13L6.70 13.24L5.21 12.63L3.68 13.76L2.24 12.32L3.37 10.79L2.76 9.30L0.87 9.01L0.87 6.99L2.76 6.70L3.37 5.21L2.24 3.68L3.68 2.24L5.21 3.37L6.70 2.76L6.99 0.87L9.01 0.87L9.30 2.76L10.79 3.37L12.32 2.24L13.76 3.68L12.63 5.21Z M8 5.6A2.4 2.4 0 1 0 8 10.4A2.4 2.4 0 1 0 8 5.6Z"
    />
  ),
  gridView: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="none" stroke="currentColor" />
      <path d="M1.5 6h13M1.5 9.5h13M6 2.5v11M10.5 2.5v11" stroke="currentColor" />
    </>
  ),
  formView: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="none" stroke="currentColor" />
      <path d="M4 5.5h3M8.5 5.5h4M4 8h3M8.5 8h4M4 10.5h3M8.5 10.5h4" stroke="currentColor" />
    </>
  ),
  attach: (
    <>
      <rect x="2.5" y="2.5" width="11" height="11" rx="1" fill="none" stroke="currentColor" />
      <path d="M10.5 5.5L6.2 9.8M6 6.8v3.2h3.2" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  fullscreen: <path d="M2 6V2h4M10 2h4v4M14 10v4h-4M6 14H2v-4" fill="none" stroke="currentColor" />,
  splitH: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="none" stroke="currentColor" />
      <path d="M1.5 8h13" stroke="currentColor" />
    </>
  ),
  splitV: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="none" stroke="currentColor" />
      <path d="M8 2.5v11" stroke="currentColor" />
    </>
  ),
  warning: (
    <>
      <path d="M8 1.8l6.7 12H1.3z" fill="#e5304b" />
      <path d="M8 6v3.6" stroke="#fff" strokeWidth="1.6" strokeLinecap="round" />
      <circle cx="8" cy="11.8" r=".9" fill="#fff" />
    </>
  ),
  success: (
    <>
      <circle cx="8" cy="8" r="6.5" fill="#2fb36b" />
      <path d="M5 8.2l2 2 4-4.4" fill="none" stroke="#fff" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </>
  ),
  ellipsis: <path d="M4 9h.01M8 9h.01M12 9h.01" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />,
  sidebar: (
    <>
      <rect x="2.5" y="2.5" width="11" height="11" fill="none" stroke="currentColor" />
      <rect x="2.5" y="2.5" width="4" height="11" fill="currentColor" opacity=".55" />
    </>
  ),
  plusSquare: (
    <>
      <rect x="2.5" y="2.5" width="11" height="11" fill="none" stroke="#2f7fd0" />
      <path d="M8 5v6M5 8h6" stroke="#2f7fd0" />
    </>
  ),
  tableDesign: (
    <>
      <rect x="1.5" y="2.5" width="11" height="10" rx="1" fill="#fff" stroke="#2f7fd0" />
      <path d="M1.5 3.5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1V5.5h-11z" fill="#2f7fd0" />
      <path d="M8.5 14l.8-2.6 4.6-4.6 1.8 1.8-4.6 4.6z" fill="#fff" stroke="#56606b" strokeLinejoin="round" />
    </>
  ),
  table: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" fill="#fff" stroke="#2f7fd0" />
      <path d="M1.5 3.5a1 1 0 0 1 1-1h11a1 1 0 0 1 1 1V6h-13z" fill="#2f7fd0" />
      <path d="M1.5 9.5h13M6 6v7.5M10.5 6v7.5" stroke="#9cc4ec" />
    </>
  ),
  view: (
    <>
      <rect x="1.5" y="2.5" width="13" height="8" rx="1" fill="#fff" stroke="#2f7fd0" />
      <circle cx="5" cy="11.5" r="2.5" fill="#fff" stroke="#24466b" strokeWidth="1.3" />
      <circle cx="11" cy="11.5" r="2.5" fill="#fff" stroke="#24466b" strokeWidth="1.3" />
    </>
  ),
  matview: (
    <>
      <rect x="1.5" y="2.5" width="13" height="8" rx="1" fill="#fbe6e3" stroke="#e8574a" />
      <path d="M1.5 5.5h13" stroke="#e8574a" strokeWidth="2" />
      <circle cx="5" cy="11.5" r="2.5" fill="#fff" stroke="#b5372c" strokeWidth="1.3" />
      <circle cx="11" cy="11.5" r="2.5" fill="#fff" stroke="#b5372c" strokeWidth="1.3" />
    </>
  ),
  function: <text x="8" y="12.5" textAnchor="middle" fontFamily="Georgia, serif" fontStyle="italic" fontSize="11" fill="#2f6fb5">fx</text>,
  sequence: (
    <>
      <text x="8" y="12" textAnchor="middle" fontFamily="Segoe UI, sans-serif" fontWeight="700" fontSize="8.5" fill="#2f6fb5">123</text>
      <rect x="1.5" y="3.5" width="13" height="10" rx="1.5" fill="none" stroke="#2f7fd0" />
    </>
  ),
  type: <text x="8" y="12.5" textAnchor="middle" fontFamily="Segoe UI, sans-serif" fontWeight="700" fontSize="10" fill="#7a4fc4">T</text>,
  // Extensão: peça de quebra-cabeça (módulo que se encaixa no banco).
  connGroup: (
    <>
      <path d="M1.5 4.2a1 1 0 0 1 1-1h3.2l1.4 1.5h6.4a1 1 0 0 1 1 1v6.6a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z" fill="#f3c969" stroke="#c79a2c" strokeWidth=".9" />
    </>
  ),
  tablespace: (
    <>
      <ellipse cx="8" cy="4.2" rx="5.5" ry="2.2" fill="#cfd8e3" stroke="#6b7480" strokeWidth=".9" />
      <path d="M2.5 4.2v7.2c0 1.2 2.5 2.2 5.5 2.2s5.5-1 5.5-2.2V4.2" fill="#e4eaf1" stroke="#6b7480" strokeWidth=".9" />
      <path d="M2.5 7.8c0 1.2 2.5 2.2 5.5 2.2s5.5-1 5.5-2.2" fill="none" stroke="#6b7480" strokeWidth=".9" />
      <circle cx="11" cy="11.6" r=".7" fill="#2f7fd0" />
    </>
  ),
  foreignTable: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" fill="#fff" stroke="#2f7fd0" strokeDasharray="2 1.2" />
      <path d="M1.5 3.5a1 1 0 0 1 1-1h11a1 1 0 0 1 1 1V6h-13z" fill="#2f7fd0" />
      <path d="M1.5 9h13M5.8 6v7.5M10.2 6v7.5" stroke="#9cc4ec" strokeWidth=".9" />
      <path d="M9.5 14.5l3.5-3.5M11 11h2v2" stroke="#e07b00" strokeWidth="1.1" fill="none" />
    </>
  ),
  foreignServer: (
    <>
      <rect x="2.5" y="2" width="11" height="4.2" rx="1" fill="#e8f0fb" stroke="#2f7fd0" strokeWidth="1.1" />
      <rect x="2.5" y="9.8" width="11" height="4.2" rx="1" fill="#e8f0fb" stroke="#2f7fd0" strokeWidth="1.1" />
      <path d="M8 6.2v3.6" stroke="#2f7fd0" strokeWidth="1.1" strokeDasharray="1.4 1.2" />
      <circle cx="4.8" cy="4.1" r=".7" fill="#2f7fd0" />
      <circle cx="4.8" cy="11.9" r=".7" fill="#2f7fd0" />
    </>
  ),
  extension: (
    <path
      d="M2.5 6h3a1.8 1.8 0 1 1 3.6 0h3.4v3.4a1.8 1.8 0 1 1 0 3.6V14.5h-9z"
      fill="#e6f4ea" stroke="#2f9e57" strokeWidth="1.2" strokeLinejoin="round"
    />
  ),
  user: (
    <>
      <circle cx="8" cy="5.5" r="3" fill="#f3c27a" stroke="#c98a3a" strokeWidth=".8" />
      <path d="M3 14c0-3 2.2-5 5-5s5 2 5 5z" fill="#e9852f" />
    </>
  ),
  query: (
    <>
      <rect x="4.5" y="1.5" width="9" height="8" rx="1" fill="#fff" stroke="#e8574a" />
      <rect x="2.5" y="5.5" width="9" height="9" rx="1" fill="#fff" stroke="#2f7fd0" />
      <path d="M4.5 9h5M4.5 11.5h3.5" stroke="#2f7fd0" strokeLinecap="round" />
    </>
  ),
  backup: (
    <>
      <rect x="2" y="2" width="12" height="12" rx="2.5" fill="#5b6470" />
      <path d="M5.5 8.2a2.7 2.7 0 1 0 .9-2" fill="none" stroke="#fff" strokeWidth="1.3" strokeLinecap="round" />
      <path d="M5.7 4.8v1.6h1.6" fill="none" stroke="#fff" strokeWidth="1.3" strokeLinecap="round" />
    </>
  ),
  automation: (
    <>
      <rect x="2.5" y="4" width="11" height="8" rx="2.5" fill="#33b3a6" />
      <circle cx="6" cy="8" r="1.3" fill="#fff" />
      <circle cx="10" cy="8" r="1.3" fill="#fff" />
    </>
  ),
  model: (
    <>
      <rect x="1.5" y="1.5" width="6" height="5" rx="1" fill="#f2a33a" />
      <rect x="8.5" y="5.5" width="6" height="5" rx="1" fill="#e8742e" />
      <rect x="3" y="10" width="6" height="5" rx="1" fill="#f2a33a" />
    </>
  ),
  bi: (
    <>
      <rect x="2" y="8" width="3" height="6" fill="#c13fd6" />
      <rect x="6.5" y="4" width="3" height="10" fill="#e8408f" />
      <rect x="11" y="2" width="3" height="12" fill="#8c4fe0" />
    </>
  ),
  viewList: <path d="M2 3.5h2M6 3.5h8M2 8h2M6 8h8M2 12.5h2M6 12.5h8" stroke="currentColor" />,
  viewDetail: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="none" stroke="currentColor" />
      <path d="M1.5 6h13M1.5 9.5h13M6 2.5v11" stroke="currentColor" />
    </>
  ),
  viewIcons: (
    <>
      <rect x="2" y="2" width="5" height="5" fill="none" stroke="currentColor" />
      <rect x="9" y="2" width="5" height="5" fill="none" stroke="currentColor" />
      <rect x="2" y="9" width="5" height="5" fill="none" stroke="currentColor" />
      <rect x="9" y="9" width="5" height="5" fill="none" stroke="currentColor" />
    </>
  ),
  viewEr: (
    <>
      <rect x="1.5" y="2.5" width="6" height="5" fill="none" stroke="currentColor" />
      <rect x="8.5" y="9" width="6" height="5" fill="none" stroke="currentColor" />
      <path d="M7.5 5h2.5v4" fill="none" stroke="currentColor" />
    </>
  ),
  paneNav: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="none" stroke="currentColor" />
      <rect x="1.5" y="2.5" width="4.5" height="11" fill="currentColor" />
    </>
  ),
  paneInfo: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" fill="none" stroke="currentColor" />
      <rect x="10" y="2.5" width="4.5" height="11" fill="currentColor" />
    </>
  ),
};

export type SmallIconName = keyof typeof small;

export function SmallIcon({ name, size = 16, className }: { name: SmallIconName; size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden="true" className={`icon16${className ? ` ${className}` : ""}`}>
      {small[name]}
    </svg>
  );
}

/** Cores e siglas por provedor (usadas quando não há logotipo disponível, ex.: SQL Server). */
export const PROVIDER_STYLE: Record<ProviderId, [string, string, string]> = {
  postgresql: ["#336791", "#dce8f3", "PG"],
  sqlserver: ["#c0392b", "#f6dede", "SQL"],
  mysql: ["#00758f", "#d8eef3", "My"],
  mariadb: ["#8a5a3c", "#efe3da", "Ma"],
  sqlite: ["#0f80cc", "#dceefa", "Lite"],
  firebird: ["#d6481f", "#f8e0d8", "FB"],
};

/*
 * Logotipos dos SGBDs via Simple Icons (dados SVG CC0; as marcas pertencem aos
 * respectivos donos e são usadas só para identificar o banco). SQL Server e
 * Oracle não estão no Simple Icons a pedido dos donos: usam o ladrilho com sigla.
 */
const BRANDS: Record<string, SimpleIcon> = {
  postgresql: siPostgresql,
  mysql: siMysql,
  mariadb: siMariadb,
  sqlite: siSqlite,
  mongodb: siMongodb,
  redis: siRedis,
};

export const brandOf = (id: string): SimpleIcon | undefined => BRANDS[id];

/** Ícone de conexão por provedor: logotipo do SGBD (cinza quando a conexão está fechada, como na referência). */
export function ProviderIcon({ provider, open }: { provider: ProviderId; open: boolean }) {
  const brand = BRANDS[provider];
  if (brand) {
    return (
      <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true" className={`icon16 brand-logo${open ? "" : " is-closed"}`}>
        <path d={brand.path} fill={`#${brand.hex}`} />
      </svg>
    );
  }
  const [stroke, fill, label] = PROVIDER_STYLE[provider];
  const s = open ? stroke : "#8c8c8c";
  const f = open ? fill : "#ececec";
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" className="icon16">
      <rect x="1.5" y="1.5" width="13" height="13" rx="2.5" fill={f} stroke={s} />
      <text x="8" y="11" textAnchor="middle" fontFamily="Segoe UI, sans-serif" fontWeight="700" fontSize={label.length > 2 ? 5.5 : 7} fill={s}>
        {label}
      </text>
    </svg>
  );
}

/** Ladrilho grande do seletor de tipo de conexão: logotipo branco sobre a cor da marca, ou sigla. */
export function ProviderTile({ color, label, size = 48, provider, disabled }: { color: string; label: string; size?: number; provider?: string; disabled?: boolean }) {
  const brand = provider ? BRANDS[provider] : undefined;
  const bg = disabled ? "#b9b9b9" : brand ? `#${brand.hex}` : color;
  return (
    <svg width={size} height={size} viewBox="0 0 48 48" aria-hidden="true">
      <rect x="1" y="1" width="46" height="46" rx="6" fill={bg} />
      {brand ? (
        <path d={brand.path} fill="#fff" transform="translate(10 10) scale(1.1667)" />
      ) : (
        <>
          <ellipse cx="24" cy="15" rx="12" ry="4.5" fill="none" stroke="#fff" strokeWidth="2" opacity=".85" />
          <path d="M12 15v17c0 2.5 5.4 4.5 12 4.5s12-2 12-4.5V15" fill="none" stroke="#fff" strokeWidth="2" opacity=".85" />
          <text x="24" y="30" textAnchor="middle" fontFamily="Segoe UI, sans-serif" fontWeight="700" fontSize={label.length > 3 ? 9 : 11} fill="#fff">
            {label}
          </text>
        </>
      )}
    </svg>
  );
}

export function categoryIcon(cat: Category): SmallIconName {
  return cat;
}
