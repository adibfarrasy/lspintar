use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct SymbolSuperMapping {
    pub id: Option<i64>,
    pub symbol_fqn: String,
    pub super_short_name: String,
    pub super_fqn: Option<String>,
}
