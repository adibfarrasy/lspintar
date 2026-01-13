CREATE TABLE symbol_interface_mapping (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    symbol_fqn TEXT NOT NULL,
    interface_short_name TEXT NOT NULL,
    interface_fqn TEXT
);

CREATE INDEX idx_interface_fqn ON symbol_interface_mapping(interface_fqn);
CREATE INDEX idx_interface_short_name ON symbol_interface_mapping(interface_short_name);
CREATE INDEX idx_symbol_fqn ON symbol_interface_mapping(symbol_fqn);
