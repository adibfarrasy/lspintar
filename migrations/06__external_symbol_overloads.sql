-- Include metadata in the external symbol unique key so method overloads
-- (same FQN, different parameter lists) are preserved instead of upserted
-- into a single row. Mirrors the internal `symbols` table conflict key.

DROP INDEX IF EXISTS idx_ext_unique;

CREATE UNIQUE INDEX idx_ext_unique
    ON external_symbols(jar_path, source_file_path, fully_qualified_name, metadata);
