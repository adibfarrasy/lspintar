use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

use crate::models::{external_symbol::ExternalSymbol, symbol::Symbol};

fn capitalize_prefix(prefix: &str) -> String {
    let lower = prefix.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
    }
}

#[derive(Debug)]
pub struct Repository {
    pool: SqlitePool,
}

impl Repository {
    pub async fn new(path: &str) -> Result<Self, sqlx::Error> {
        let url = if path.starts_with("file:") || path == ":memory:" {
            format!("sqlite:{}", path)
        } else {
            format!("sqlite:{}?mode=rwc", path)
        };

        let pool = SqlitePoolOptions::new()
            .max_connections(num_cpus::get() as u32)
            .connect(&url)
            .await?;

        // WAL mode: readers never block on writers, so autocomplete queries
        // run concurrently with the initial indexing writes instead of queuing behind them.
        // synchronous=NORMAL is safe with WAL and avoids the per-commit fsync overhead.
        // busy_timeout gives queries a grace period before returning SQLITE_BUSY.
        sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await?;
        sqlx::query("PRAGMA synchronous=NORMAL").execute(&pool).await?;
        sqlx::query("PRAGMA busy_timeout=5000").execute(&pool).await?;
        // Enables index use for LIKE prefix queries. Queries use lower(prefix) for FQNs
        // and capitalize(prefix) for short names to preserve case-insensitive matching.
        sqlx::query("PRAGMA case_sensitive_like=ON").execute(&pool).await?;

        sqlx::migrate!("../migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn insert_symbols(&self, symbols: &[Symbol]) -> Result<(), sqlx::Error> {
        if symbols.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        let file_path = &symbols[0].file_path;
        sqlx::query("DELETE FROM symbols WHERE file_path = ?")
            .bind(file_path)
            .execute(&mut *tx)
            .await?;

        for s in symbols {
            sqlx::query(
                "INSERT INTO symbols (short_name, package_name, fully_qualified_name, parent_name, 
                file_path, file_type, symbol_type, modifiers, line_start, line_end, 
                char_start, char_end, ident_line_start, ident_line_end, ident_char_start,
                ident_char_end, metadata, last_modified)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(file_path, fully_qualified_name, metadata) DO UPDATE SET
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
    pub async fn find_symbol_by_fqn(&self, fqn: &str) -> Result<Option<Symbol>, sqlx::Error> {
        tracing::info!("find_symbol_by_fqn");
        sqlx::query_as::<_, Symbol>("SELECT * FROM symbols WHERE fully_qualified_name = ?")
            .bind(fqn)
            .fetch_optional(&self.pool)
            .await
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_symbols_by_parent_name(
        &self,
        parent_fqn: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        tracing::info!("find_symbols_by_parent_name");
        sqlx::query_as::<_, Symbol>("SELECT * FROM symbols WHERE parent_name = ?")
            .bind(parent_fqn)
            .fetch_all(&self.pool)
            .await
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_symbols_by_prefix(&self, prefix: &str) -> Result<Vec<Symbol>, sqlx::Error> {
        tracing::info!("find_symbols_by_prefix");
        let fqn_pat = format!("{}%", prefix.to_lowercase());
        let short_pat = format!("{}%", capitalize_prefix(prefix));
        let mut by_fqn = sqlx::query_as::<_, Symbol>(
            "SELECT * FROM symbols WHERE fully_qualified_name LIKE ? AND symbol_type NOT IN ('Function', 'Field') LIMIT 100",
        )
        .bind(&fqn_pat)
        .fetch_all(&self.pool)
        .await?;

        let seen: std::collections::HashSet<String> =
            by_fqn.iter().map(|s| s.fully_qualified_name.clone()).collect();

        let by_short = sqlx::query_as::<_, Symbol>(
            "SELECT * FROM symbols WHERE short_name LIKE ? AND symbol_type NOT IN ('Function', 'Field') LIMIT 100",
        )
        .bind(&short_pat)
        .fetch_all(&self.pool)
        .await?;

        by_fqn.extend(by_short.into_iter().filter(|s| !seen.contains(&s.fully_qualified_name)));
        by_fqn.truncate(200);
        Ok(by_fqn)
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_symbols_by_fqn(&self, fqn: &str) -> Result<Vec<Symbol>, sqlx::Error> {
        tracing::info!("find_symbols_by_fqn");
        sqlx::query_as::<_, Symbol>("SELECT * FROM symbols WHERE fully_qualified_name = ?")
            .bind(fqn)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn insert_symbol_super_mappings(
        &self,
        mappings: Vec<(&str, &str, Option<&str>)>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        for (symbol_fqn, _, _) in &mappings {
            sqlx::query("DELETE FROM symbol_super_mapping WHERE symbol_fqn = ?")
                .bind(symbol_fqn)
                .execute(&mut *tx)
                .await?;
        }

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

    pub async fn find_super_impls_by_fqn(
        &self,
        super_fqn: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        let symbols = sqlx::query_as::<_, Symbol>(
            "SELECT s.id, s.short_name, s.package_name, 
                s.fully_qualified_name, s.parent_name, s.file_path, 
                s.file_type, s.symbol_type, s.modifiers, s.line_start, 
                s.line_end, s.char_start, s.char_end, s.ident_line_start,
                s.ident_line_end, s.ident_char_start, s.ident_char_end,
                s.metadata, s.last_modified
                FROM symbols s
                INNER JOIN symbol_super_mapping ssm 
                    ON s.fully_qualified_name = ssm.symbol_fqn
                WHERE ssm.super_fqn = ?",
        )
        .bind(super_fqn)
        .fetch_all(&self.pool)
        .await?;

        Ok(symbols)
    }

    pub async fn find_super_impls_by_short_name(
        &self,
        super_short_name: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        let symbols = sqlx::query_as::<_, Symbol>(
            "SELECT s.id, s.short_name, s.package_name, 
                s.fully_qualified_name, s.parent_name, s.file_path, 
                s.file_type, s.symbol_type, s.modifiers, s.line_start, 
                s.line_end, s.char_start, s.char_end, s.ident_line_start,
                s.ident_line_end, s.ident_char_start, s.ident_char_end,
                s.metadata, s.last_modified
                FROM symbols s
                INNER JOIN symbol_super_mapping ssm 
                    ON s.fully_qualified_name = ssm.symbol_fqn
                WHERE ssm.super_short_name = ?",
        )
        .bind(super_short_name)
        .fetch_all(&self.pool)
        .await?;

        Ok(symbols)
    }

    pub async fn find_supers_by_symbol_fqn(
        &self,
        symbol_fqn: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        let symbols = sqlx::query_as::<_, Symbol>(
            "SELECT s.id, s.short_name, s.package_name, 
                s.fully_qualified_name, s.parent_name, s.file_path, 
                s.file_type, s.symbol_type, s.modifiers, s.line_start, 
                s.line_end, s.char_start, s.char_end, s.ident_line_start,
                s.ident_line_end, s.ident_char_start, s.ident_char_end,
                s.metadata, s.last_modified
                FROM symbols s
                INNER JOIN symbol_super_mapping ssm 
                    ON s.fully_qualified_name = ssm.super_fqn
                WHERE ssm.symbol_fqn = ?",
        )
        .bind(symbol_fqn)
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

        for s in symbols {
            sqlx::query(
            "INSERT INTO external_symbols (jar_path, source_file_path, alt_jar_path, short_name, package_name, 
            fully_qualified_name, parent_name, symbol_type, modifiers, line_start, line_end, 
            char_start, char_end, ident_line_start, ident_line_end, ident_char_start,
            ident_char_end, needs_decompilation, metadata, last_modified, file_type)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(jar_path, source_file_path, fully_qualified_name, metadata) DO UPDATE SET
                alt_jar_path = excluded.alt_jar_path,
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
                last_modified = excluded.last_modified,
                file_type = excluded.file_type",
        )
        .bind(&s.jar_path)
        .bind(&s.source_file_path)
        .bind(&s.alt_jar_path)
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
        .bind(&s.file_type)
        .execute(&mut *tx)
        .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_external_symbol_by_fqn(
        &self,
        fqn: &str,
    ) -> Result<Option<ExternalSymbol>, sqlx::Error> {
        let result = sqlx::query_as::<_, ExternalSymbol>(
            "SELECT * FROM external_symbols WHERE fully_qualified_name = ? ORDER BY needs_decompilation ASC LIMIT 1",
        )
        .bind(fqn)
        .fetch_optional(&self.pool)
        .await;

        tracing::info!("find_external_symbol_by_fqn result: {:?}", result);
        result
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_external_symbols_by_parent_name(
        &self,
        parent_fqn: &str,
    ) -> Result<Vec<ExternalSymbol>, sqlx::Error> {
        tracing::info!("find_external_symbols_by_parent_name");
        sqlx::query_as::<_, ExternalSymbol>("SELECT * FROM external_symbols WHERE parent_name = ?")
            .bind(parent_fqn)
            .fetch_all(&self.pool)
            .await
    }

    /// Like `find_external_symbols_by_parent_name` but restricted to symbols from the given JARs.
    /// Falls back to the unfiltered query when `jar_paths` is empty.
    #[tracing::instrument(skip(self, jar_paths))]
    pub async fn find_external_symbols_by_parent_name_and_jars(
        &self,
        parent_fqn: &str,
        jar_paths: &[String],
    ) -> Result<Vec<ExternalSymbol>, sqlx::Error> {
        let all = self.find_external_symbols_by_parent_name(parent_fqn).await?;
        if jar_paths.is_empty() {
            return Ok(all);
        }
        Ok(all
            .into_iter()
            .filter(|s| jar_paths.contains(&s.jar_path))
            .collect())
    }

    #[tracing::instrument(skip(self))]
    pub async fn find_external_symbols_by_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<ExternalSymbol>, sqlx::Error> {
        tracing::info!("find_external_symbols_by_prefix");
        let fqn_pat = format!("{}%", prefix.to_lowercase());
        let short_pat = format!("{}%", capitalize_prefix(prefix));
        let mut by_fqn = sqlx::query_as::<_, ExternalSymbol>(
            "SELECT * FROM external_symbols WHERE fully_qualified_name LIKE ? AND symbol_type NOT IN ('Function', 'Field') LIMIT 100",
        )
        .bind(&fqn_pat)
        .fetch_all(&self.pool)
        .await?;

        let seen: std::collections::HashSet<String> =
            by_fqn.iter().map(|s| s.fully_qualified_name.clone()).collect();

        let by_short = sqlx::query_as::<_, ExternalSymbol>(
            "SELECT * FROM external_symbols WHERE short_name LIKE ? AND symbol_type NOT IN ('Function', 'Field') LIMIT 100",
        )
        .bind(&short_pat)
        .fetch_all(&self.pool)
        .await?;

        by_fqn.extend(by_short.into_iter().filter(|s| !seen.contains(&s.fully_qualified_name)));
        by_fqn.truncate(200);
        Ok(by_fqn)
    }

    /// Like `find_external_symbols_by_prefix` but restricted to symbols from the given JARs.
    /// Falls back to the unfiltered query when `jar_paths` is empty.
    #[tracing::instrument(skip(self, jar_paths))]
    pub async fn find_external_symbols_by_prefix_and_jars(
        &self,
        prefix: &str,
        jar_paths: &[String],
    ) -> Result<Vec<ExternalSymbol>, sqlx::Error> {
        let all = self.find_external_symbols_by_prefix(prefix).await?;
        if jar_paths.is_empty() {
            return Ok(all);
        }
        Ok(all
            .into_iter()
            .filter(|s| jar_paths.contains(&s.jar_path))
            .collect())
    }

    pub async fn delete_symbols_for_file(&self, file_path: &str) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "DELETE FROM symbol_super_mapping WHERE symbol_fqn IN 
        (SELECT fully_qualified_name FROM symbols WHERE file_path = ?)",
        )
        .bind(file_path)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM symbols WHERE file_path = ?")
            .bind(file_path)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_external_symbols_for_jar(&self, jar_path: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM external_symbols WHERE jar_path = ?")
            .bind(jar_path)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Returns all symbols indexed for a single source file.  Used by the
    /// rename handler to identify the declaration at the cursor when
    /// `resolve_symbol_at_position` cannot resolve a declaration site.
    pub async fn find_symbols_by_file_path(
        &self,
        file_path: &str,
    ) -> Result<Vec<Symbol>, sqlx::Error> {
        sqlx::query_as::<_, Symbol>("SELECT * FROM symbols WHERE file_path = ?")
            .bind(file_path)
            .fetch_all(&self.pool)
            .await
    }

    /// Returns the distinct file paths of all indexed project symbols.
    /// Used by the references handler to know which source files to search.
    pub async fn find_all_source_file_paths(&self) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT file_path FROM symbols ORDER BY file_path")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(p,)| p).collect())
    }

    pub async fn clear_all(&self) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM symbol_super_mapping")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM symbols").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM external_symbols")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
