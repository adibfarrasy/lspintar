CREATE TABLE symbols (
    id INTEGER PRIMARY KEY,
    vcs_branch TEXT,
    short_name TEXT NOT NULL,
    fully_qualified_name TEXT NOT NULL,
    parent_name TEXT,
    file_path TEXT NOT NULL,
    file_type TEXT NOT NULL,
    symbol_type TEXT NOT NULL,
    modifiers TEXT NOT NULL DEFAULT '[]', -- JSON array
    
    -- Locations
    line_start INTEGER NOT NULL,         
    line_end INTEGER NOT NULL,           
    char_start INTEGER NOT NULL,
    char_end INTEGER NOT NULL,

    ident_line_start INTEGER NOT NULL,         
    ident_line_end INTEGER NOT NULL,           
    ident_char_start INTEGER NOT NULL,
    ident_char_end INTEGER NOT NULL,
    
    -- Relationships
    extends_name TEXT,
    implements_names TEXT NOT NULL DEFAULT '[]', -- JSON array
    
    -- Metadata
    metadata TEXT NOT NULL DEFAULT '{}', -- JSON object SymbolMetadata
                    
    last_modified INTEGER NOT NULL
);

CREATE INDEX idx_fqn ON symbols(fully_qualified_name);
CREATE INDEX idx_short_name ON symbols(short_name);
CREATE INDEX idx_parent ON symbols(parent_name);
CREATE INDEX idx_position_lookup ON symbols(file_path, line_start, line_end);
CREATE INDEX idx_vcs_branch ON symbols(vcs_branch);
CREATE INDEX idx_extends ON symbols(extends_name);
CREATE INDEX idx_symbol_type ON symbols(symbol_type);
CREATE INDEX idx_file_path ON symbols(file_path);
