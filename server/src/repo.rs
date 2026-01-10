use sqlx::SqlitePool;
use tracing::debug;

use crate::models::symbol::Symbol;

#[derive(Debug)]
pub struct Repository {
    pool: SqlitePool,
}

impl Repository {
    pub async fn new(path: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect(&format!("sqlite:{}", path)).await?;
        sqlx::migrate!("../migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn insert_symbols(&self, symbols: &[Symbol]) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        for s in symbols {
            sqlx::query(
                "INSERT INTO symbols (vcs_branch, short_name, fully_qualified_name, parent_name, 
                file_path, file_type, symbol_type, modifiers, line_start, line_end, 
                char_start, char_end, ident_line_start, ident_line_end, ident_char_start,
                ident_char_end, extends_name, implements_names, metadata, last_modified)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(vcs_branch, file_path, fully_qualified_name) DO UPDATE SET
                    vcs_branch = excluded.vcs_branch,
                    short_name = excluded.short_name,
                    fully_qualified_name = excluded.fully_qualified_name,
                    parent_name = excluded.parent_name,
                    file_type = excluded.file_type,
                    symbol_type = excluded.symbol_type,
                    modifiers = excluded.modifiers,
                    line_end = excluded.line_end,
                    char_end = excluded.char_end,
                    ident_line_start = excluded.ident_line_start,
                    ident_line_end = excluded.ident_line_end,
                    ident_char_start = excluded.ident_char_start,
                    ident_char_end = excluded.ident_char_end,
                    extends_name = excluded.extends_name,
                    implements_names = excluded.implements_names,
                    metadata = excluded.metadata,
                    last_modified = excluded.last_modified",
            )
            .bind(&s.vcs_branch)
            .bind(&s.short_name)
            .bind(&s.fully_qualified_name)
            .bind(&s.parent_name)
            .bind(&s.file_path)
            .bind(&s.file_type)
            .bind(&s.symbol_type)
            .bind(&s.modifiers)
            .bind(s.line_start)
            .bind(s.line_end)
            .bind(s.char_start)
            .bind(s.char_end)
            .bind(s.ident_line_start)
            .bind(s.ident_line_end)
            .bind(s.ident_char_start)
            .bind(s.ident_char_end)
            .bind(&s.extends_name)
            .bind(&s.implements_names)
            .bind(&s.metadata)
            .bind(s.last_modified)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_symbol_by_fqn(&self, fqn: &str) -> Result<Option<Symbol>, sqlx::Error> {
        debug!("find_symbol_by_fqn");
        sqlx::query_as::<_, Symbol>("SELECT * FROM symbols WHERE fully_qualified_name = ?")
            .bind(fqn)
            .fetch_optional(&self.pool)
            .await
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_symbol_by_fqn_and_branch(
        &self,
        fqn: &str,
        vcs_branch: &str,
    ) -> Result<Option<Symbol>, sqlx::Error> {
        debug!("find_symbol_by_fqn_and_branch");
        sqlx::query_as::<_, Symbol>(
            "SELECT * FROM symbols WHERE fully_qualified_name = ? AND vcs_branch = ?",
        )
        .bind(fqn)
        .bind(vcs_branch)
        .fetch_optional(&self.pool)
        .await
    }
}
