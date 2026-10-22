-- Atlas DB Manager — schema local, migração v1 (spec §9.2).
-- Somente configurações e metadados do aplicativo; nunca dados de usuário nem senhas.
CREATE TABLE connection_groups (
  id TEXT PRIMARY KEY, parent_id TEXT NULL REFERENCES connection_groups(id),
  name TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0,
  created_utc TEXT NOT NULL, updated_utc TEXT NOT NULL
);
CREATE TABLE connection_profiles (
  id TEXT PRIMARY KEY, group_id TEXT NULL REFERENCES connection_groups(id),
  display_name TEXT NOT NULL, provider_id TEXT NOT NULL,
  host TEXT NULL, port INTEGER NULL, database_name TEXT NULL,
  username TEXT NULL, secret_ref TEXT NULL,
  environment TEXT NOT NULL DEFAULT 'development',
  options_json TEXT NOT NULL DEFAULT '{}', sort_order INTEGER NOT NULL DEFAULT 0,
  created_utc TEXT NOT NULL, updated_utc TEXT NOT NULL
);
CREATE TABLE workspaces (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, is_default INTEGER NOT NULL DEFAULT 0,
  layout_json TEXT NOT NULL DEFAULT '{}', created_utc TEXT NOT NULL, updated_utc TEXT NOT NULL
);
CREATE TABLE documents (
  id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  connection_profile_id TEXT NULL REFERENCES connection_profiles(id) ON DELETE SET NULL,
  kind TEXT NOT NULL, title TEXT NOT NULL, file_path TEXT NULL,
  content_json TEXT NOT NULL DEFAULT '{}', is_pinned INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL DEFAULT 0, updated_utc TEXT NOT NULL
);
CREATE TABLE sql_history (
  id TEXT PRIMARY KEY, connection_profile_id TEXT NULL REFERENCES connection_profiles(id) ON DELETE SET NULL,
  executed_utc TEXT NOT NULL, duration_ms INTEGER NULL,
  status TEXT NOT NULL, statement_redacted TEXT NULL,
  row_count INTEGER NULL, error_code TEXT NULL
);
CREATE TABLE saved_queries (
  id TEXT PRIMARY KEY, title TEXT NOT NULL, provider_id TEXT NULL,
  connection_profile_id TEXT NULL REFERENCES connection_profiles(id) ON DELETE SET NULL,
  sql_text TEXT NOT NULL, tags_json TEXT NOT NULL DEFAULT '[]', updated_utc TEXT NOT NULL
);
CREATE TABLE preferences (
  key TEXT PRIMARY KEY, value_json TEXT NOT NULL, updated_utc TEXT NOT NULL
);
CREATE TABLE job_definitions (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL,
  connection_profile_id TEXT NULL REFERENCES connection_profiles(id) ON DELETE SET NULL,
  schedule TEXT NOT NULL, timezone TEXT NOT NULL,
  params_json TEXT NOT NULL DEFAULT '{}', enabled INTEGER NOT NULL DEFAULT 0,
  created_utc TEXT NOT NULL, updated_utc TEXT NOT NULL
);
CREATE TABLE job_runs (
  id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES job_definitions(id) ON DELETE CASCADE,
  started_utc TEXT NOT NULL, ended_utc TEXT NULL, status TEXT NOT NULL,
  summary_json TEXT NOT NULL DEFAULT '{}', sanitized_error TEXT NULL
);
CREATE TABLE diagram_models (
  id TEXT PRIMARY KEY, title TEXT NOT NULL, connection_profile_id TEXT NULL
    REFERENCES connection_profiles(id) ON DELETE SET NULL,
  model_json TEXT NOT NULL, updated_utc TEXT NOT NULL
);
CREATE TABLE installed_plugins (
  id TEXT PRIMARY KEY, version TEXT NOT NULL, manifest_json TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 0, installed_utc TEXT NOT NULL
);
CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY, applied_utc TEXT NOT NULL, checksum TEXT NOT NULL
);
CREATE INDEX idx_profiles_group ON connection_profiles(group_id, sort_order);
CREATE INDEX idx_docs_workspace ON documents(workspace_id, position);
CREATE INDEX idx_history_time ON sql_history(executed_utc DESC);
CREATE INDEX idx_runs_job_time ON job_runs(job_id, started_utc DESC);
