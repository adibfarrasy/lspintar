use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::core::build_tools::ExternalDependency;

use super::dependency_cache::source_file_info::SourceFileInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitState {
    pub head_commit: String,
    pub branch: String,
    pub dependencies_hash: String,
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub mtime: u64,
    pub size: u64,
}

pub struct PersistenceLayer {
    conn: Connection,
    project_path: PathBuf,
}

impl PersistenceLayer {
    /// Initialize database connection and create tables
    /// Called from: LSP initialize request
    pub fn new(project_path: PathBuf) -> Result<Self> {
        // Create cache directory
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| anyhow!("Could not find cache directory"))?
            .join("lspintar")
            .join(Self::get_project_hash(&project_path));
        
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("Failed to create cache directory: {:?}", cache_dir))?;
        
        let db_path = cache_dir.join("index.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database: {:?}", db_path))?;
        
        // Set SQLite pragmas for performance
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "temp_store", "memory")?;
        conn.pragma_update(None, "mmap_size", "268435456")?; // 256MB
        
        let persistence = Self {
            conn,
            project_path: project_path.canonicalize()
                .unwrap_or(project_path),
        };
        
        persistence.create_tables()?;
        Ok(persistence)
    }

    /// Create all necessary tables and indexes
    fn create_tables(&self) -> SqliteResult<()> {
        // Git state table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS git_state (
                project_path TEXT PRIMARY KEY,
                head_commit TEXT NOT NULL,
                branch TEXT NOT NULL,
                dependencies_hash TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        )?;
        
        // Symbol index table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS symbol_index (
                project_path TEXT NOT NULL,
                fully_qualified_name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL,
                indexed_at INTEGER NOT NULL,
                PRIMARY KEY (project_path, fully_qualified_name)
            )",
            [],
        )?;
        
        // Builtin infos table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS builtin_infos (
                class_name TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                zip_internal_path TEXT,
                dependency_info TEXT,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL,
                indexed_at INTEGER NOT NULL
            )",
            [],
        )?;
        
        // Inheritance index table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS inheritance_index (
                project_path TEXT NOT NULL,
                type_name TEXT NOT NULL,
                locations BLOB NOT NULL,
                indexed_at INTEGER NOT NULL,
                PRIMARY KEY (project_path, type_name)
            )",
            [],
        )?;
        
        // Project external infos table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS project_external_infos (
                project_path TEXT NOT NULL,
                type_name TEXT NOT NULL,
                source_path TEXT NOT NULL,
                zip_internal_path TEXT,
                dependency_info TEXT,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL,
                indexed_at INTEGER NOT NULL,
                PRIMARY KEY (project_path, type_name)
            )",
            [],
        )?;
        
        // Create indexes for performance
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_symbol_file_path ON symbol_index(file_path)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_indexed_at ON symbol_index(indexed_at)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_builtin_indexed_at ON builtin_infos(indexed_at)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_inheritance_indexed_at ON inheritance_index(indexed_at)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_external_indexed_at ON project_external_infos(indexed_at)",
            [],
        )?;
        
        Ok(())
    }

    /// Check if current git state differs from cached state
    /// Called from: LSP initialize (startup validation)
    pub fn is_git_state_stale(&self) -> Result<bool> {
        let current_state = self.get_current_git_state()?;
        
        let stored_state = self.conn.prepare(
            "SELECT head_commit, branch, dependencies_hash FROM git_state WHERE project_path = ?"
        )?.query_row(
            params![self.project_path.to_string_lossy()],
            |row| {
                Ok(GitState {
                    head_commit: row.get(0)?,
                    branch: row.get(1)?,
                    dependencies_hash: row.get(2)?,
                })
            }
        );
        
        match stored_state {
            Ok(stored) => {
                Ok(stored.head_commit != current_state.head_commit
                    || stored.branch != current_state.branch
                    || stored.dependencies_hash != current_state.dependencies_hash)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(true), // No stored state
            Err(e) => Err(e.into()),
        }
    }

    /// Update stored git state after successful indexing
    /// Called from: After indexing completes, workspace/didChangeWatchedFiles on .git changes
    pub fn update_git_state(&self, git_state: GitState) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        
        self.conn.execute(
            "INSERT OR REPLACE INTO git_state 
             (project_path, head_commit, branch, dependencies_hash, updated_at) 
             VALUES (?, ?, ?, ?, ?)",
            params![
                self.project_path.to_string_lossy(),
                git_state.head_commit,
                git_state.branch,
                git_state.dependencies_hash,
                now as i64
            ],
        )?;
        
        Ok(())
    }

    /// Check if file metadata has changed since last index
    /// Called from: textDocument/didOpen, before cache lookups
    pub fn is_file_stale(&self, file_path: &Path) -> Result<bool> {
        let metadata = fs::metadata(file_path);
        if metadata.is_err() {
            return Ok(true); // File doesn't exist, consider stale
        }
        let metadata = metadata?;
        
        let current_mtime = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)?
            .as_secs();
        let current_size = metadata.len();
        
        // Check symbol_index table first
        let stored_result = self.conn.prepare(
            "SELECT mtime, size FROM symbol_index WHERE file_path = ? AND project_path = ?"
        )?.query_row(
            params![file_path.to_string_lossy(), self.project_path.to_string_lossy()],
            |row| {
                let stored_mtime: i64 = row.get(0)?;
                let stored_size: i64 = row.get(1)?;
                Ok((stored_mtime as u64, stored_size as u64))
            }
        );
        
        match stored_result {
            Ok((stored_mtime, stored_size)) => {
                Ok(stored_mtime != current_mtime || stored_size != current_size)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(true), // Not in cache
            Err(e) => Err(e.into()),
        }
    }

    /// Load symbol index: (project_root, fqn) -> file_path
    /// Called from: LSP initialize (bulk load), workspace/symbol requests
    pub fn load_symbol_index(&self) -> Result<DashMap<(PathBuf, String), PathBuf>> {
        let mut stmt = self.conn.prepare(
            "SELECT project_path, fully_qualified_name, file_path FROM symbol_index WHERE project_path LIKE ?"
        )?;
        
        let workspace_pattern = format!("{}%", self.project_path.to_string_lossy());
        let rows = stmt.query_map(
            params![workspace_pattern],
            |row| {
                let project_path: String = row.get(0)?;
                let fqn: String = row.get(1)?;
                let file_path: String = row.get(2)?;
                Ok((PathBuf::from(project_path), fqn, PathBuf::from(file_path)))
            }
        )?;
        
        let map = DashMap::new();
        for row in rows {
            let (project_path, fqn, file_path) = row?;
            let key = (project_path, fqn);
            map.insert(key, file_path);
        }
        
        Ok(map)
    }

    /// Store symbol index to database
    /// Called from: After indexing project files, textDocument/didSave (incremental)
    pub fn store_symbol_index(&self, map: &DashMap<(PathBuf, String), PathBuf>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        
        // Delete existing entries for this project
        tx.execute(
            "DELETE FROM symbol_index WHERE project_path = ?",
            params![self.project_path.to_string_lossy()]
        )?;
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        
        {
            // Prepare insert statement in its own scope
            let mut stmt = tx.prepare(
                "INSERT INTO symbol_index 
                 (project_path, fully_qualified_name, file_path, mtime, size, indexed_at) 
                 VALUES (?, ?, ?, ?, ?, ?)"
            )?;
            
            
            for entry in map.iter() {
                let ((project_path, fqn), file_path) = (entry.key(), entry.value());
                
                // Store all project-related entries (allow subdirectories)
                // Skip only if the project_path is not under our workspace
                if !project_path.starts_with(&self.project_path) {
                    skipped_count += 1;
                    continue;
                }
                
                // Get file metadata
                let (mtime, size) = match fs::metadata(file_path) {
                    Ok(metadata) => {
                        let mtime = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs() as i64;
                        let size = metadata.len() as i64;
                        (mtime, size)
                    }
                    Err(_) => (0, 0), // File might not exist
                };
                
                stmt.execute(params![
                    project_path.to_string_lossy(),  // Use the symbol's project_path, not workspace path
                    fqn,
                    file_path.to_string_lossy(),
                    mtime,
                    size,
                    now
                ])?;
                saved_count += 1;
            }
            
        } // stmt is dropped here
        
        tx.commit()?;
        Ok(())
    }

    /// Load builtin class infos: class_name -> SourceFileInfo
    /// Called from: LSP initialize (bulk load)
    pub fn load_builtin_infos(&self) -> Result<DashMap<String, SourceFileInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT class_name, source_path, zip_internal_path, dependency_info 
             FROM builtin_infos"
        )?;
        
        let rows = stmt.query_map([], |row| {
            let class_name: String = row.get(0)?;
            let source_path: String = row.get(1)?;
            let zip_internal_path: Option<String> = row.get(2)?;
            let dependency_info_json: Option<String> = row.get(3)?;
            Ok((class_name, source_path, zip_internal_path, dependency_info_json))
        })?;
        
        let map = DashMap::new();
        for row in rows {
            let (class_name, source_path, zip_internal_path, dependency_info_json) = row?;
            
            let dependency = if let Some(json) = dependency_info_json {
                deserialize_external_dependency(&json).ok().flatten()
            } else {
                None
            };
            
            let source_info = SourceFileInfo::new(
                PathBuf::from(source_path),
                zip_internal_path,
                dependency,
            );
            
            map.insert(class_name, source_info);
        }
        
        Ok(map)
    }

    /// Store builtin infos to database
    /// Called from: After scanning JAVA_HOME/GROOVY_HOME, when builtins change
    pub fn store_builtin_infos(&self, map: &DashMap<String, SourceFileInfo>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        
        // Delete all existing builtin_infos
        tx.execute("DELETE FROM builtin_infos", [])?;
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        
        {
            // Prepare insert statement in its own scope
            let mut stmt = tx.prepare(
                "INSERT INTO builtin_infos 
                 (class_name, source_path, zip_internal_path, dependency_info, mtime, size, indexed_at) 
                 VALUES (?, ?, ?, ?, ?, ?, ?)"
            )?;
            
            for entry in map.iter() {
                let (class_name, source_info) = (entry.key(), entry.value());
                
                // Get file metadata
                let (mtime, size) = match fs::metadata(&source_info.source_path) {
                    Ok(metadata) => {
                        let mtime = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs() as i64;
                        let size = metadata.len() as i64;
                        (mtime, size)
                    }
                    Err(_) => (0, 0), // ZIP files or missing files
                };
                
                // Serialize dependency info
                let dependency_json = serialize_external_dependency(&source_info.dependency)?;
                
                stmt.execute(params![
                    class_name,
                    source_info.source_path.to_string_lossy(),
                    source_info.zip_internal_path,
                    dependency_json,
                    mtime,
                    size,
                    now
                ])?;
            }
        } // stmt is dropped here
        
        tx.commit()?;
        Ok(())
    }

    /// Load inheritance index: (project_root, type_name) -> Vec<(file, line, col)>
    /// Called from: LSP initialize, textDocument/references for inheritance chains
    pub fn load_inheritance_index(&self) -> Result<DashMap<(PathBuf, String), Vec<(PathBuf, usize, usize)>>> {
        let mut stmt = self.conn.prepare(
            "SELECT project_path, type_name, locations FROM inheritance_index WHERE project_path LIKE ?"
        )?;
        
        let workspace_pattern = format!("{}%", self.project_path.to_string_lossy());
        let rows = stmt.query_map(
            params![workspace_pattern],
            |row| {
                let project_path: String = row.get(0)?;
                let type_name: String = row.get(1)?;
                let locations_blob: Vec<u8> = row.get(2)?;
                Ok((PathBuf::from(project_path), type_name, locations_blob))
            }
        )?;
        
        let map = DashMap::new();
        for row in rows {
            let (project_path, type_name, locations_blob) = row?;
            if let Ok(locations) = deserialize_locations(&locations_blob) {
                let key = (project_path, type_name);
                map.insert(key, locations);
            }
        }
        
        Ok(map)
    }

    /// Store inheritance index to database
    /// Called from: After analyzing class hierarchies, textDocument/didSave (incremental)
    pub fn store_inheritance_index(
        &self,
        map: &DashMap<(PathBuf, String), Vec<(PathBuf, usize, usize)>>,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        
        // Delete existing entries for this project
        tx.execute(
            "DELETE FROM inheritance_index WHERE project_path = ?",
            params![self.project_path.to_string_lossy()]
        )?;
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        
        {
            // Prepare insert statement in its own scope
            let mut stmt = tx.prepare(
                "INSERT INTO inheritance_index 
                 (project_path, type_name, locations, indexed_at) 
                 VALUES (?, ?, ?, ?)"
            )?;
            
            for entry in map.iter() {
                let ((project_path, type_name), locations) = (entry.key(), entry.value());
                
                // Store all project-related entries (allow subdirectories)
                if !project_path.starts_with(&self.project_path) {
                    continue;
                }
                
                let locations_blob = serialize_locations(locations)?;
                
                stmt.execute(params![
                    project_path.to_string_lossy(),
                    type_name,
                    locations_blob,
                    now
                ])?;
            }
        } // stmt is dropped here
        
        tx.commit()?;
        Ok(())
    }

    /// Load project external infos: (project_root, type_name) -> SourceFileInfo
    /// Called from: LSP initialize, workspace/symbol for external dependencies
    pub fn load_project_external_infos(&self) -> Result<DashMap<(PathBuf, String), SourceFileInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT project_path, type_name, source_path, zip_internal_path, dependency_info 
             FROM project_external_infos WHERE project_path LIKE ?"
        )?;
        
        let workspace_pattern = format!("{}%", self.project_path.to_string_lossy());
        let rows = stmt.query_map(
            params![workspace_pattern],
            |row| {
                let project_path: String = row.get(0)?;
                let type_name: String = row.get(1)?;
                let source_path: String = row.get(2)?;
                let zip_internal_path: Option<String> = row.get(3)?;
                let dependency_info_json: Option<String> = row.get(4)?;
                Ok((PathBuf::from(project_path), type_name, source_path, zip_internal_path, dependency_info_json))
            }
        )?;
        
        let map = DashMap::new();
        for row in rows {
            let (project_path, type_name, source_path, zip_internal_path, dependency_info_json) = row?;
            
            let dependency = if let Some(json) = dependency_info_json {
                deserialize_external_dependency(&json).ok().flatten()
            } else {
                None
            };
            
            let source_info = SourceFileInfo::new(
                PathBuf::from(source_path),
                zip_internal_path,
                dependency,
            );
            
            let key = (project_path, type_name);
            map.insert(key, source_info);
        }
        
        Ok(map)
    }

    /// Store project external infos to database
    /// Called from: After gradle dependency resolution, workspace/didChangeWatchedFiles on build.gradle
    pub fn store_project_external_infos(
        &self,
        map: &DashMap<(PathBuf, String), SourceFileInfo>,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        
        // Delete existing entries for this project
        tx.execute(
            "DELETE FROM project_external_infos WHERE project_path = ?",
            params![self.project_path.to_string_lossy()]
        )?;
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        
        {
            // Prepare insert statement in its own scope
            let mut stmt = tx.prepare(
                "INSERT INTO project_external_infos 
                 (project_path, type_name, source_path, zip_internal_path, dependency_info, mtime, size, indexed_at) 
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )?;
            
            for entry in map.iter() {
                let ((project_path, type_name), source_info) = (entry.key(), entry.value());
                
                // Store all project-related entries (allow subdirectories)
                if !project_path.starts_with(&self.project_path) {
                    continue;
                }
                
                // Get file metadata
                let (mtime, size) = match fs::metadata(&source_info.source_path) {
                    Ok(metadata) => {
                        let mtime = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs() as i64;
                        let size = metadata.len() as i64;
                        (mtime, size)
                    }
                    Err(_) => (0, 0), // ZIP files or missing files
                };
                
                // Serialize dependency info
                let dependency_json = serialize_external_dependency(&source_info.dependency)?;
                
                stmt.execute(params![
                    project_path.to_string_lossy(),
                    type_name,
                    source_info.source_path.to_string_lossy(),
                    source_info.zip_internal_path,
                    dependency_json,
                    mtime,
                    size,
                    now
                ])?;
            }
        } // stmt is dropped here
        
        tx.commit()?;
        Ok(())
    }

    /// Invalidate cache entries for specific files
    /// Called from: textDocument/didChange, workspace/didChangeWatchedFiles
    pub fn invalidate_files(&self, file_paths: &[PathBuf]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        
        for file_path in file_paths {
            let file_path_str = file_path.to_string_lossy();
            
            // Delete from symbol_index
            tx.execute(
                "DELETE FROM symbol_index WHERE file_path = ? AND project_path = ?",
                params![file_path_str, self.project_path.to_string_lossy()]
            )?;
            
            // Delete from project_external_infos
            tx.execute(
                "DELETE FROM project_external_infos WHERE source_path = ? AND project_path = ?",
                params![file_path_str, self.project_path.to_string_lossy()]
            )?;
            
            // For inheritance_index, we need to check if file_path exists in the locations blob
            // This is more complex, so for now we'll just mark the cache as potentially stale
        }
        
        tx.commit()?;
        Ok(())
    }

    /// Clean up entries for files that no longer exist
    /// Called from: Periodic maintenance, after git checkout/pull
    pub fn cleanup_missing_files(&self, existing_files: &[PathBuf]) -> Result<()> {
        let existing_set: std::collections::HashSet<PathBuf> = existing_files.iter().cloned().collect();
        let tx = self.conn.unchecked_transaction()?;
        
        // Get all file paths from symbol_index
        let mut stmt = tx.prepare(
            "SELECT DISTINCT file_path FROM symbol_index WHERE project_path = ?"
        )?;
        
        let rows = stmt.query_map(
            params![self.project_path.to_string_lossy()],
            |row| {
                let path: String = row.get(0)?;
                Ok(PathBuf::from(path))
            }
        )?;
        
        let mut file_paths = Vec::new();
        for row in rows {
            file_paths.push(row?);
        }
        drop(stmt); // Explicitly drop the statement
        
        // Delete entries for missing files
        for file_path in file_paths {
            if !existing_set.contains(&file_path) && !file_path.exists() {
                tx.execute(
                    "DELETE FROM symbol_index WHERE file_path = ? AND project_path = ?",
                    params![file_path.to_string_lossy(), self.project_path.to_string_lossy()]
                )?;
                
                tx.execute(
                    "DELETE FROM project_external_infos WHERE source_path = ? AND project_path = ?",
                    params![file_path.to_string_lossy(), self.project_path.to_string_lossy()]
                )?;
            }
        }
        
        tx.commit()?;
        
        // VACUUM to reclaim space
        self.conn.execute("VACUUM", [])?;
        
        Ok(())
    }

    /// Check current database size and enforce limits
    /// Called from: LSP shutdown, periodic maintenance
    pub fn enforce_size_limit(&self, max_size_mb: u64) -> Result<()> {
        // Get database file size by looking at the database file
        let db_path = self.conn.path().ok_or_else(|| anyhow!("Cannot get database path"))?;
        let db_size = fs::metadata(db_path)?.len();
        let max_size_bytes = max_size_mb * 1024 * 1024;
        
        if db_size > max_size_bytes {
            
            let tx = self.conn.unchecked_transaction()?;
            
            // Delete oldest entries from each table (keep most recent 70%)
            let tables_and_conditions = [
                ("symbol_index", "project_path = ?"),
                ("builtin_infos", "1 = 1"), // No project filtering for builtins
                ("inheritance_index", "project_path = ?"),
                ("project_external_infos", "project_path = ?"),
            ];
            
            for (table, condition) in &tables_and_conditions {
                let delete_query = format!(
                    "DELETE FROM {} WHERE {} AND indexed_at < (
                        SELECT indexed_at FROM {} WHERE {} 
                        ORDER BY indexed_at DESC 
                        LIMIT 1 OFFSET (SELECT COUNT(*) * 7 / 10 FROM {} WHERE {})
                    )",
                    table, condition, table, condition, table, condition
                );
                
                if condition.contains("project_path") {
                    tx.execute(&delete_query, params![self.project_path.to_string_lossy()])?;
                } else {
                    tx.execute(&delete_query, [])?;
                }
            }
            
            tx.commit()?;
            self.conn.execute("VACUUM", [])?;
        }
        
        Ok(())
    }

    /// Get current git state for comparison
    fn get_current_git_state(&self) -> Result<GitState> {
        // Get HEAD commit
        let head_output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.project_path)
            .output()
            .context("Failed to execute git rev-parse HEAD")?;
        
        let head_commit = String::from_utf8_lossy(&head_output.stdout).trim().to_string();
        
        // Get current branch
        let branch_output = Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&self.project_path)
            .output()
            .context("Failed to execute git symbolic-ref")?;
        
        let branch = if branch_output.status.success() {
            String::from_utf8_lossy(&branch_output.stdout).trim().to_string()
        } else {
            // Detached HEAD state
            "HEAD".to_string()
        };
        
        // Hash dependency files
        let dependencies_hash = self.hash_dependency_files()?;
        
        Ok(GitState {
            head_commit,
            branch,
            dependencies_hash,
        })
    }

    /// Generate unique project identifier for cache directory
    fn get_project_hash(project_path: &Path) -> String {
        let canonical_path = project_path.canonicalize()
            .unwrap_or_else(|_| project_path.to_path_buf());
        
        let mut hasher = Sha256::new();
        hasher.update(canonical_path.to_string_lossy().as_bytes());
        let result = hasher.finalize();
        
        // Take first 16 chars of hex representation
        format!("{:x}", result)[..16].to_string()
    }

    /// Bulk load all cached data for project startup
    /// Called from: LSP initialize after git state validation
    pub fn load_all_caches(
        &self,
    ) -> Result<(
        DashMap<(PathBuf, String), PathBuf>,                      // symbol_index
        DashMap<String, SourceFileInfo>,                          // builtin_infos
        DashMap<(PathBuf, String), Vec<(PathBuf, usize, usize)>>, // inheritance_index
        DashMap<(PathBuf, String), SourceFileInfo>,               // project_external_infos
    )> {
        let symbol_index = self.load_symbol_index().unwrap_or_else(|_| DashMap::new());
        
        let builtin_infos = self.load_builtin_infos().unwrap_or_else(|_| DashMap::new());
        
        let inheritance_index = self.load_inheritance_index().unwrap_or_else(|_| DashMap::new());
        
        let project_external_infos = self.load_project_external_infos().unwrap_or_else(|_| DashMap::new());
        
        Ok((symbol_index, builtin_infos, inheritance_index, project_external_infos))
    }

    /// Bulk store all cached data  
    /// Called from: After complete project indexing, LSP shutdown
    pub fn store_all_caches(
        &self,
        symbol_index: &DashMap<(PathBuf, String), PathBuf>,
        builtin_infos: &DashMap<String, SourceFileInfo>,
        inheritance_index: &DashMap<(PathBuf, String), Vec<(PathBuf, usize, usize)>>,
        project_external_infos: &DashMap<(PathBuf, String), SourceFileInfo>,
    ) -> Result<()> {
        // Store each cache separately to handle partial failures better
        let _ = self.store_symbol_index(symbol_index);
        
        let _ = self.store_builtin_infos(builtin_infos);
        
        let _ = self.store_inheritance_index(inheritance_index);
        
        let _ = self.store_project_external_infos(project_external_infos);
        
        // Update git state to mark cache as current
        if let Ok(git_state) = self.get_current_git_state() {
            let _ = self.update_git_state(git_state);
        }
        
        Ok(())
    }
    
    /// Hash dependency-related files to detect changes
    fn hash_dependency_files(&self) -> Result<String> {
        let mut hasher = Sha256::new();
        
        // Hash common dependency files
        let dependency_files = [
            "build.gradle",
            "build.gradle.kts", 
            "settings.gradle",
            "settings.gradle.kts",
            "Cargo.toml",
            "Cargo.lock",
            "pom.xml",
            "package.json",
        ];
        
        for file_name in &dependency_files {
            let file_path = self.project_path.join(file_name);
            if file_path.exists() {
                if let Ok(content) = fs::read_to_string(&file_path) {
                    hasher.update(file_name.as_bytes());
                    hasher.update(content.as_bytes());
                }
            }
        }
        
        let result = hasher.finalize();
        Ok(format!("{:x}", result)[..16].to_string())
    }
}

// Helper functions for serialization
fn serialize_locations(locations: &Vec<(PathBuf, usize, usize)>) -> Result<Vec<u8>> {
    bincode::serialize(locations).map_err(|e| anyhow!("Failed to serialize locations: {}", e))
}

fn deserialize_locations(data: &[u8]) -> Result<Vec<(PathBuf, usize, usize)>> {
    bincode::deserialize(data).map_err(|e| anyhow!("Failed to deserialize locations: {}", e))
}

fn serialize_external_dependency(dep: &Option<ExternalDependency>) -> Result<String> {
    match dep {
        Some(dep) => serde_json::to_string(dep)
            .map_err(|e| anyhow!("Failed to serialize ExternalDependency: {}", e)),
        None => Ok("null".to_string()),
    }
}

fn deserialize_external_dependency(json: &str) -> Result<Option<ExternalDependency>> {
    if json == "null" || json.is_empty() {
        return Ok(None);
    }
    
    let dep: ExternalDependency = serde_json::from_str(json)
        .map_err(|e| anyhow!("Failed to deserialize ExternalDependency: {}", e))?;
    Ok(Some(dep))
}
