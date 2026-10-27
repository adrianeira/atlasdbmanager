# Atlas DB Manager

**English** · [Português](README.md)

A native, cross-platform desktop database manager (Windows, Linux and macOS), built with
**Rust + Tauri 2 + React**. It focuses on everyday database work: browsing objects, editing data
in a fast grid, writing SQL, designing tables and views, and moving data between databases —
even of **different products** — with the safety of reviewing what is about to run.

> **Status:** version 0.1.0, pre-alpha. It works with **PostgreSQL**, **SQL Server**, **MySQL**,
> **SQLite** and **Firebird**. MariaDB, Oracle, MongoDB and Redis are planned and postponed.

![Atlas DB Manager: connection tree on the left and the data grid of a Firebird table](docs/home.png)

## Supported databases

| Database | Notes |
|---|---|
| **PostgreSQL** | The most complete: also materialized views, sequences, types, extensions, roles, tablespaces, foreign servers and tables, PL/pgSQL functions |
| **SQL Server** | Schemas, procedures and functions, `IDENTITY` |
| **MySQL** | Databases as schemas, procedures and functions, `AUTO_INCREMENT`. Older server versions do not keep `CHECK` |
| **SQLite** | Pick or create the file on the connection screen |
| **Firebird** | **Pure-Rust** client (no `fbclient.dll` needed); opens `.fdb` and `.gdb`; identity or a generator plus trigger, depending on the server version. |

## Features

**Connections**
- Connection test, colors and favorites, connection **groups (folders)** in the tree, SSL and an
  **SSH tunnel** (password or private key, with the server key remembered on first use).
- New connection from a **URI**, import/export of connections (file without passwords) and
  sharing a connection through an `atlas://` address.
- Passwords stored in the **operating system's credential vault** (Windows Credential Manager,
  macOS Keychain, Secret Service on Linux) — never in a file and never sent to the UI.
- Local settings in SQLite inside the user profile, **no administrator rights required**.

**Navigation**
- Tree of connections → databases → schemas → tables, views, functions and saved queries, with
  columns and dedicated primary-key and foreign-key icons; advanced tree filter.
- Object list (details, list and icons), information pane and context menus.

**Data (grid)**
- Virtualized grid with name + type header, striped rows and highlighted `(Null)`.
- In-place editing with **automatic save when moving to another record**, manual transaction
  (Begin Transaction), Apply/Discard and editing of tables without a primary key.
- Values are always sent as **parameters** (nothing concatenated into SQL) with optimistic
  concurrency: the `UPDATE` checks the original value and fails if someone else changed the row.
- Multi-field filter and sort, column chooser, Cell Editor, foreign-key lookup with the record
  label, pagination and a configurable record limit.
- Cell context menu: set NULL/empty, generate UUID, copy, copy as `INSERT`/`UPDATE`/TSV, paste,
  delete record.
- **Data Profiling** (nulls, distinct, unique, minimum and maximum, most frequent values) and
  **Table Profile** (structure, sizes and statistics).

**SQL**
- SQL editor (CodeMirror) with the database's dialect highlighting, autocomplete, run the
  selection, `EXPLAIN`, format, server-side query cancel, query **history** and **snippets**.
- **Query Builder** in its own window: tables dragged onto a canvas, joins from foreign keys,
  filters, grouping, sorting and live SQL in the database's dialect.
- **Console** (`F6`) in the style of each database's terminal, with Tab completion and help
  commands.
- Atomic **Execute SQL File**, honoring SQL Server `GO` batches and Firebird `SET TERM` blocks.

**Structure**
- **Table designer** (fields, indexes, foreign keys, uniques, checks, triggers, comment) with SQL
  preview and each database's **auto increment**: `IDENTITY` on PostgreSQL and SQL Server,
  `AUTO_INCREMENT` on MySQL, `INTEGER PRIMARY KEY` on SQLite and identity or a generator with a
  trigger on Firebird.
- **View designer** and **routine designer** (procedures, functions and, on Firebird, triggers by
  their `CREATE OR ALTER` text).
- Create, delete, rename (where the database allows it), duplicate and empty objects, **new and
  delete database**; structure commands are always reviewed before they run.
- Schema **ER diagram**, **Find in Database** (structure and data, by text, whole word, prefix or
  regular expression) and **Data Dictionary** (PDF document with tables, views and routines).

**Import and export**
- **Export** wizard in steps, for **several tables at once**, to **TXT, CSV, JSON, XML, HTML, SQL
  (`INSERT`), Excel (`.xlsx` and `.xls`) and dBase (`.dbf`)**; from the grid it carries the
  filter, the sort and the visible columns; from the SQL editor it exports the **full result** of
  the query. **Streaming** writes, with progress and **Stop**.
- 7-step **import** wizard, with several files or sheets at once, new tables with inferred types
  and six modes (append, update, append or update, append without update, delete, copy),
  continue-on-error per row and saved profiles.

**Backup and restore**
- **SQL dump** (structure and/or data), **backup** with a comment and a destination folder,
  **restore** and extracting the SQL of a backup. PostgreSQL uses the server's own tools; on the
  other databases Atlas generates and runs the script itself, with no external tools.

**Tools (Tools menu)**
- **Data Transfer**: copies structure and data from a schema to another database or server, with
  table and field mapping, filters, saved profiles and a log.
- **Data Synchronization** and **Structure Synchronization**: compare two sides, show the
  differences, build the deployment script and run it (in a transaction, with continue-on-error).
- **Between different products**, for any pair of PostgreSQL, SQL Server, MySQL, SQLite and
  Firebird: a neutral model translates types, values, simple `CHECK`s, views and automatic
  numbering. Triggers and routines only cross between databases of the same product.
- **Data Generation**: test data with per-type generators, references between tables and
  dependency order.
- **Server Monitor**: running connections and commands, variables and counters, and ending a
  connection.

**Interface**
- Light/dark theme (follows the system), Portuguese and English, hideable panes and separate
  system windows for the big tools.

## Requirements

- [Rust](https://rustup.rs) 1.90 or later
- [Node.js](https://nodejs.org) 20 or later
- Tauri 2 prerequisites for your system
  ([official guide](https://v2.tauri.app/start/prerequisites/)):
  - **Windows:** Microsoft C++ Build Tools and WebView2 (already included in Windows 10/11).
  - **Linux (Debian/Ubuntu):**
    `sudo apt install build-essential pkg-config libssl-dev libwebkit2gtk-4.1-dev librsvg2-dev libdbus-1-dev`
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`).

## Running

```bash
npm install

# Desktop app in development mode
npm run tauri dev

# UI only, in the browser (demo mode, with fictitious data)
npm run dev        # http://localhost:1420
```

## Building the app

```bash
# Installer (MSI/NSIS on Windows, .deb/.rpm/AppImage on Linux, .app/.dmg on macOS)
npm run tauri build

# Just the executable, no installer (portable folder)
npm run tauri build -- --no-bundle
```

Packages end up in `target/release/bundle/`; the bare executable in `target/release/`.

## Tests

```bash
npx tsc --noEmit -p .        # frontend types
cargo test --workspace       # Rust tests
```

Tests against a real database only run when that database's variables exist (credentials never
live in the repository). They only use temporary objects:

```bash
# PostgreSQL
ATLAS_TEST_PG_HOST=localhost ATLAS_TEST_PG_PORT=5432 \
ATLAS_TEST_PG_USER=postgres ATLAS_TEST_PG_PASSWORD=... \
cargo test -p atlas-pg --test live_postgres

# MySQL, SQL Server and Firebird: ATLAS_TEST_MYSQL_*, ATLAS_TEST_MSSQL_* and ATLAS_TEST_FIREBIRD_*
# (HOST, PORT, USER, PASSWORD and, for Firebird, DATABASE)
ATLAS_TEST_FIREBIRD_HOST=localhost ATLAS_TEST_FIREBIRD_PORT=3050 \
ATLAS_TEST_FIREBIRD_DATABASE=/path/to/db.fdb ATLAS_TEST_FIREBIRD_USER=SYSDBA \
ATLAS_TEST_FIREBIRD_PASSWORD=... cargo test -p atlas-firebird
```

The credential-vault test is opt-in: `ATLAS_TEST_KEYRING=1 cargo test -p atlas-local`.

## Layout

```
crates/
  atlas-core/      domain and contracts (types, errors, secret that never leaks into logs)
  atlas-pg/        PostgreSQL provider (tokio-postgres)
  atlas-mssql/     SQL Server provider (tiberius)
  atlas-mysql/     MySQL provider (mysql_async)
  atlas-sqlite/    SQLite provider (rusqlite)
  atlas-firebird/  Firebird provider (pure-Rust client)
  atlas-local/     local storage (SQLite) and the OS credential vault
  atlas-export/    writing results as CSV, JSON, SQL, Excel and more (streaming)
  atlas-import/    reading files for the import wizard
  atlas-docs/      Data Dictionary document generation (PDF)
  atlas-ssh/       SSH tunnel
src-tauri/         desktop app (Tauri commands, sessions, cross-product tools)
src/               React + TypeScript interface
vendor/            patched copy of the pure-Rust Firebird client (see ATLAS-CHANGES.md)
scripts/           utilities (e.g. third-party license list generation)
```

## Where local data lives

| System | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\dev.atlas.dbmanager\` |
| Linux | `~/.local/share/dev.atlas.dbmanager/` |
| macOS | `~/Library/Application Support/dev.atlas.dbmanager/` |

The `atlas.db` file holds connections and preferences — **no passwords**, which stay in the system
vault. It does not live next to the executable: replacing the app with a new version keeps the
connections.

## License

Released under the **MIT** license — see [LICENSE](LICENSE).

Third-party dependencies keep their authors' licenses (MIT, Apache-2.0, BSD, Zlib, Unicode-3.0,
MPL-2.0, among others). The full list is in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) and
can be regenerated with `node scripts/third-party-licenses.mjs`. The few **MPL-2.0** components
(used by Tauri) stay under that license; it applies file by file and does not affect Atlas code.

Database logos come from [Simple Icons](https://simpleicons.org) (CC0-1.0); the SQL Server and
Firebird ones come from [Devicon](https://github.com/devicons/devicon) (MIT). PostgreSQL, SQL
Server, MySQL, SQLite, Firebird and all other names and logos are trademarks of their respective
owners; they are used here only to identify the connection type and do not imply affiliation or
endorsement.
