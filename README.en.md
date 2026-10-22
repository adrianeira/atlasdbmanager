# Atlas DB Manager

**English** · [Português](README.md)

A native, cross-platform desktop database manager (Windows, Linux and macOS), built with
**Rust + Tauri 2 + React**. It focuses on everyday database work: browsing objects, editing data
in a fast grid, writing SQL and designing tables and views — with the safety of reviewing what
is about to run.

> **Status:** under development (version 0.1, pre-alpha). The first provider is **PostgreSQL**;
> **Firebird** (version 1.5 and later) is planned.

## Features

**Connections**
- PostgreSQL connections with SSL, connection test, and per-connection colors/favorites.
- Passwords stored in the **operating system's credential vault** (Windows Credential Manager,
  macOS Keychain, Secret Service on Linux) — never in a file and never sent to the UI.
- Local settings in SQLite inside the user profile, **no administrator rights required**.

**Navigation**
- Tree of connections → databases → schemas → tables, views, materialized views and functions,
  with columns and dedicated primary-key and foreign-key icons.
- Object list (details, list and icons) with context menus in both.

**Data (grid)**
- Virtualized grid with name + type header, striped rows and highlighted `(Null)`.
- In-place editing with **automatic save when moving to another record**; Apply/Discard buttons.
- Values are always sent as **parameters** (nothing concatenated into SQL) with optimistic
  concurrency: the `UPDATE` checks the original value and fails if someone else changed the row.
- Required (`NOT NULL`) fields cannot be left empty; booleans shown as checkboxes.
- Foreign-key lookup (`…` button in the cell), paging and a configurable record limit.
- Cell context menu: set NULL/empty, generate UUID (v4/v7), copy, copy as
  `INSERT`/`UPDATE`/TSV, paste, delete record.

**Export**
- **Step-by-step** export wizard (format → tables and files → fields → summary with log and progress)
  for **several tables at once**, as **TXT, CSV, JSON, XML, HTML, SQL (`INSERT`) and Excel (`.xlsx`)**;
  export profiles you can save and reuse.
- From the grid it starts with that table's filter, sort order and visible columns; in the SQL editor it
  exports the **complete result** of the query (without the on-screen row limit).
- **Streaming** writes (flat memory even with millions of rows), with progress and **Stop**;
  the file only appears once the export finishes successfully.

**SQL and structure**
- SQL editor (CodeMirror) with PostgreSQL dialect highlighting, autocomplete, run, `EXPLAIN`,
  formatting and server-side query cancel.
- Table designer (fields, indexes, FKs, uniques, checks, triggers, comment) with SQL preview.
- View designer (definition, options, comment) and view data browsing.
- Structure commands (create/drop/rename/duplicate) are always reviewed before running.

**Interface**
- Light/dark theme (follows the system), Portuguese and English.

## Requirements

- [Rust](https://rustup.rs) 1.90 or later
- [Node.js](https://nodejs.org) 20 or later
- Tauri 2 prerequisites for your OS ([official guide](https://v2.tauri.app/start/prerequisites/)):
  - **Windows:** Microsoft C++ Build Tools and WebView2 (bundled with Windows 10/11).
  - **Linux (Debian/Ubuntu):**
    `sudo apt install build-essential pkg-config libssl-dev libwebkit2gtk-4.1-dev librsvg2-dev libdbus-1-dev`
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`).

## Running

```bash
npm install

# Desktop app in development mode
npm run tauri dev

# UI only, in the browser (demo mode with sample data)
npm run dev        # http://localhost:1420
```

## Building the installer

```bash
npm run tauri build
```

Bundles are written to `target/release/bundle/` (MSI/NSIS on Windows, `.deb`/`.rpm`/AppImage on
Linux, `.app`/`.dmg` on macOS).

## Tests

```bash
npx tsc --noEmit -p .        # frontend types
cargo test --workspace       # Rust tests
```

Tests against a real PostgreSQL server only run when the variables below are set (credentials
never live in the repository). They only use temporary objects:

```bash
ATLAS_TEST_PG_HOST=localhost ATLAS_TEST_PG_PORT=5432 \
ATLAS_TEST_PG_USER=postgres ATLAS_TEST_PG_PASSWORD=... \
cargo test -p atlas-pg --test live_postgres
```

The credential-vault test is opt-in: `ATLAS_TEST_KEYRING=1 cargo test -p atlas-local`.

## Layout

```
crates/
  atlas-core/    domain and contracts (types, errors, a secret type that never leaks into logs)
  atlas-pg/      PostgreSQL provider (tokio-postgres): connection, catalog, execution, writes
  atlas-local/   local storage (SQLite) and the OS credential vault
  atlas-export/  writes results as CSV, JSON, SQL and Excel (streaming)
src-tauri/       desktop app (Tauri commands, sessions, window)
src/             React + TypeScript UI
scripts/         utilities (e.g. third-party license list generator)
```

## Where local data lives

| OS | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\dev.atlas.dbmanager\` |
| Linux | `~/.local/share/dev.atlas.dbmanager/` |
| macOS | `~/Library/Application Support/dev.atlas.dbmanager/` |

`atlas.db` stores connections and preferences — **without passwords**, which live in the OS vault.

## License

Released under the **MIT** license — see [LICENSE](LICENSE).

Third-party dependencies keep their authors' licenses (MIT, Apache-2.0, BSD, Zlib, Unicode-3.0,
MPL-2.0, among others). The full list is in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)
and can be regenerated with `node scripts/third-party-licenses.mjs`. The few **MPL-2.0**
components (used by Tauri) remain under that license; it applies file by file and does not
affect Atlas's own code.

Database logos come from the [Simple Icons](https://simpleicons.org) project (CC0-1.0).
PostgreSQL, Firebird and other names and logos are trademarks of their respective owners; they
are used only to identify the connection type and do not imply affiliation or endorsement.
