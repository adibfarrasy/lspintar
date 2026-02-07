use sqlx::SqlitePool;

use crate::models::{external_symbol::ExternalSymbol, symbol::Symbol};

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
        if symbols.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        let file_path = &symbols[0].file_path;
        let branch = &symbols[0].vcs_branch;
        sqlx::query("DELETE FROM symbols WHERE file_path = ? AND vcs_branch = ?")
            .bind(file_path)
            .bind(branch)
            .execute(&mut *tx)
            .await?;

        for s in symbols {
            sqlx::query(
                "INSERT INTO symbols (vcs_branch, short_name, package_name, fully_qualified_name, parent_name, 
                file_path, file_type, symbol_type, modifiers, line_start, line_end, 
                char_start, char_end, ident_line_start, ident_line_end, ident_char_start,
                ident_char_end, metadata, last_modified)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(vcs_branch, file_path, fully_qualified_name, metadata) DO UPDATE SET
                    vcs_branch = excluded.vcs_branch,
                    short_name = excluded.short_name,
                    package_name = excluded.package_name,
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
                    metadata = excluded.metadata,
                    last_modified = excluded.last_modified",
            )
            .bind(&s.vcs_branch)
            .bind(&s.short_name)
            .bind(&s.package_name)
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
            .bind(&s.metadata)
            .bind(s.last_modified)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_symbol_by_fqn_and_branch(
        &self,
        fqn: &str,
        vcs_branch: &str,
    ) -> Result<Option<Symbol>, sqlx::Error> {
        tracing::info!("find_symbol_by_fqn_and_branch");
        sqlx::query_as::<_, Symbol>(
            "SELECT * FROM symbols WHERE fully_qualified_name = ? AND vcs_branch = ?",
        )
        .bind(fqn)
        .bind(vcs_branch)
        .fetch_optional(&self.pool)
        .await
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_symbols_by_fqn_and_branch(
        &self,
        fqn: &str,
        vcs_branch: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        tracing::info!("find_symbols_by_fqn_and_branch");
        sqlx::query_as::<_, Symbol>(
            "SELECT * FROM symbols WHERE fully_qualified_name = ? AND vcs_branch = ?",
        )
        .bind(fqn)
        .bind(vcs_branch)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn insert_symbol_super_mappings(
        &self,
        mappings: Vec<(&str, &str, Option<&str>)>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        for (symbol_fqn, super_short_name, super_fqn) in mappings {
            sqlx::query(
                "INSERT INTO symbol_super_mapping (symbol_fqn, super_short_name, super_fqn) 
             VALUES (?, ?, ?)",
            )
            .bind(symbol_fqn)
            .bind(super_short_name)
            .bind(super_fqn)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn find_super_impls_by_fqn_and_branch(
        &self,
        super_fqn: &str,
        branch: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        let symbols = sqlx::query_as::<_, Symbol>(
            "SELECT s.id, s.vcs_branch, s.short_name, s.package_name, 
                s.fully_qualified_name, s.parent_name, s.file_path, 
                s.file_type, s.symbol_type, s.modifiers, s.line_start, 
                s.line_end, s.char_start, s.char_end, s.ident_line_start,
                s.ident_line_end, s.ident_char_start, s.ident_char_end,
                s.metadata, s.last_modified
                FROM symbols s
                INNER JOIN symbol_super_mapping ssm 
                    ON s.fully_qualified_name = ssm.symbol_fqn
                WHERE ssm.super_fqn = ? 
                AND s.vcs_branch = ?",
        )
        .bind(super_fqn)
        .bind(branch)
        .fetch_all(&self.pool)
        .await?;

        Ok(symbols)
    }

    pub async fn find_super_impls_by_short_name_and_branch(
        &self,
        super_short_name: &str,
        branch: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        let symbols = sqlx::query_as::<_, Symbol>(
            "SELECT s.id, s.vcs_branch, s.short_name, s.package_name, 
                s.fully_qualified_name, s.parent_name, s.file_path, 
                s.file_type, s.symbol_type, s.modifiers, s.line_start, 
                s.line_end, s.char_start, s.char_end, s.ident_line_start,
                s.ident_line_end, s.ident_char_start, s.ident_char_end,
                s.metadata, s.last_modified
                FROM symbols s
                INNER JOIN symbol_super_mapping ssm 
                    ON s.fully_qualified_name = ssm.symbol_fqn
                WHERE ssm.super_short_name = ? 
                AND s.vcs_branch = ?",
        )
        .bind(super_short_name)
        .bind(branch)
        .fetch_all(&self.pool)
        .await?;

        Ok(symbols)
    }

    pub async fn find_supers_by_symbol_fqn_and_branch(
        &self,
        symbol_fqn: &str,
        branch: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        let symbols = sqlx::query_as::<_, Symbol>(
            "SELECT s.id, s.vcs_branch, s.short_name, s.package_name, 
                s.fully_qualified_name, s.parent_name, s.file_path, 
                s.file_type, s.symbol_type, s.modifiers, s.line_start, 
                s.line_end, s.char_start, s.char_end, s.ident_line_start,
                s.ident_line_end, s.ident_char_start, s.ident_char_end,
                s.metadata, s.last_modified
                FROM symbols s
                INNER JOIN symbol_super_mapping ssm 
                    ON s.fully_qualified_name = ssm.super_fqn
                WHERE ssm.symbol_fqn = ? 
                AND s.vcs_branch = ?",
        )
        .bind(symbol_fqn)
        .bind(branch)
        .fetch_all(&self.pool)
        .await?;

        Ok(symbols)
    }

    pub async fn insert_external_symbols(
        &self,
        symbols: &[ExternalSymbol],
    ) -> Result<(), sqlx::Error> {
        if symbols.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        let jar_path = &symbols[0].jar_path;
        let source_file_path = &symbols[0].source_file_path;

        sqlx::query("DELETE FROM external_symbols WHERE jar_path = ? AND source_file_path = ?")
            .bind(jar_path)
            .bind(source_file_path)
            .execute(&mut *tx)
            .await?;

        for s in symbols {
            sqlx::query(
            "INSERT INTO external_symbols (jar_path, source_file_path, short_name, package_name, 
            fully_qualified_name, parent_name, symbol_type, modifiers, line_start, line_end, 
            char_start, char_end, ident_line_start, ident_line_end, ident_char_start,
            ident_char_end, needs_decompilation, metadata, last_modified)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(jar_path, source_file_path, fully_qualified_name) DO UPDATE SET
                short_name = excluded.short_name,
                package_name = excluded.package_name,
                parent_name = excluded.parent_name,
                symbol_type = excluded.symbol_type,
                modifiers = excluded.modifiers,
                line_start = excluded.line_start,
                line_end = excluded.line_end,
                char_start = excluded.char_start,
                char_end = excluded.char_end,
                ident_line_start = excluded.ident_line_start,
                ident_line_end = excluded.ident_line_end,
                ident_char_start = excluded.ident_char_start,
                ident_char_end = excluded.ident_char_end,
                needs_decompilation = excluded.needs_decompilation,
                metadata = excluded.metadata,
                last_modified = excluded.last_modified",
        )
        .bind(&s.jar_path)
        .bind(&s.source_file_path)
        .bind(&s.short_name)
        .bind(&s.package_name)
        .bind(&s.fully_qualified_name)
        .bind(&s.parent_name)
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
        .bind(s.needs_decompilation)
        .bind(&s.metadata)
        .bind(s.last_modified)
        .execute(&mut *tx)
        .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn find_external_symbol_by_fqn(
        &self,
        fqn: &str,
    ) -> Result<Option<ExternalSymbol>, sqlx::Error> {
        sqlx::query_as::<_, ExternalSymbol>(
            "SELECT * FROM external_symbols WHERE fully_qualified_name = ? LIMIT 1",
        )
        .bind(fqn)
        .fetch_optional(&self.pool)
        .await
    }
}
