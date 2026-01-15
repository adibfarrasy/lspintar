CREATE TABLE symbol_super_mapping (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    symbol_fqn TEXT NOT NULL,
    super_short_name TEXT NOT NULL,
    super_fqn TEXT
);

CREATE INDEX idx_interface_fqn ON symbol_super_mapping(super_fqn);
CREATE INDEX idx_interface_short_name ON symbol_super_mapping(super_short_name);
CREATE INDEX idx_symbol_fqn ON symbol_super_mapping(symbol_fqn);
