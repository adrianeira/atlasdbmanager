-- Atlas DB Manager — schema local, migração v2: histórico de consultas completo.
-- A tabela sql_history da v1 não guardava banco, schema, nome da conexão, mensagem de erro nem favorito.
ALTER TABLE sql_history ADD COLUMN connection_name TEXT NULL;
ALTER TABLE sql_history ADD COLUMN database_name TEXT NULL;
ALTER TABLE sql_history ADD COLUMN schema_name TEXT NULL;
ALTER TABLE sql_history ADD COLUMN error_message TEXT NULL;
ALTER TABLE sql_history ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_sql_history_time ON sql_history(executed_utc DESC);
CREATE INDEX idx_sql_history_favorite ON sql_history(favorite, executed_utc DESC);
