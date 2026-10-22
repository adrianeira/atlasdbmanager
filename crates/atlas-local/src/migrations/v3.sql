-- Atlas DB Manager — schema local, migração v3: trechos de código (snippets) do editor SQL.
CREATE TABLE snippets (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  body TEXT NOT NULL,
  updated_utc TEXT NOT NULL
);
CREATE INDEX idx_snippets_name ON snippets(name COLLATE NOCASE);
