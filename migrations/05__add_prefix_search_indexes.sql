-- Composite indexes for prefix completion queries.
-- Covering (symbol_type, short_name) and (symbol_type, fully_qualified_name) lets
-- the planner use symbol_type as an equality filter and then scan the prefix range
-- on the second column without touching the main table rows.
CREATE INDEX IF NOT EXISTS idx_ext_type_short_name
    ON external_symbols(symbol_type, short_name);

CREATE INDEX IF NOT EXISTS idx_ext_type_fqn
    ON external_symbols(symbol_type, fully_qualified_name);

CREATE INDEX IF NOT EXISTS idx_type_short_name
    ON symbols(symbol_type, short_name);

CREATE INDEX IF NOT EXISTS idx_type_fqn
    ON symbols(symbol_type, fully_qualified_name);
