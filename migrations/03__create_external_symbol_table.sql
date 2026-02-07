CREATE TABLE external_symbols (
    id INTEGER PRIMARY KEY,
    jar_path TEXT NOT NULL,
    source_file_path TEXT NOT NULL,
    short_name TEXT NOT NULL,
    fully_qualified_name TEXT NOT NULL,
    package_name TEXT NOT NULL,
    parent_name TEXT,
    symbol_type TEXT NOT NULL,
    modifiers TEXT NOT NULL DEFAULT '[]',
    
    -- Locations
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    char_start INTEGER NOT NULL,
    char_end INTEGER NOT NULL,
    ident_line_start INTEGER NOT NULL,
    ident_line_end INTEGER NOT NULL,
    ident_char_start INTEGER NOT NULL,
    ident_char_end INTEGER NOT NULL,
    
    -- Metadata
    metadata TEXT NOT NULL DEFAULT '{}',
    needs_decompilation BOOLEAN NOT NULL,
    last_modified INTEGER NOT NULL
);

CREATE INDEX idx_ext_fqn ON external_symbols(fully_qualified_name);
CREATE INDEX idx_ext_short_name ON external_symbols(short_name);
CREATE INDEX idx_ext_parent ON external_symbols(parent_name);
CREATE INDEX idx_ext_type ON external_symbols(symbol_type);
CREATE UNIQUE INDEX idx_ext_unique ON external_symbols(jar_path, source_file_path, fully_qualified_name);
