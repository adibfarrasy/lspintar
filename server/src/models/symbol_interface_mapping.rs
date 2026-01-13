use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct SymbolInterfaceMapping {
    pub id: Option<i64>,
    pub symbol_fqn: String,
    pub interface_short_name: String,
    pub interface_fqn: Option<String>,
}
