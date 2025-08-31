use anyhow::{anyhow, Context, Result};
use dashmap::{DashMap, DashSet};
use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::debug;

use crate::core::{
    build_tools::ExternalDependency,
    dependency_cache::project_deps::{IndexingStatus, ProjectMetadata},
};

use super::dependency_cache::source_file_info::SourceFileInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitState {
    pub head_commit: String,
    pub branch: String,
    pub dependencies_hash: String,
}


pub struct PersistenceLayer {
    conn: Mutex<Connection>,
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
            conn: Mutex::new(conn),
            project_path: project_path.canonicalize().unwrap_or(project_path),
        };

        // Initialize database tables
        persistence.create_tables()?;
        Ok(persistence)
    }

    /// Create all necessary tables and indexes
    fn create_tables(&self) -> SqliteResult<()> {
        let conn = self.conn.lock().unwrap();
        // Git state table
        conn.execute(
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
        conn.execute(
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
        conn.execute(
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
        conn.execute(
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
        conn.execute(
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
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_symbol_file_path ON symbol_index(file_path)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_indexed_at ON symbol_index(indexed_at)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_builtin_indexed_at ON builtin_infos(indexed_at)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_inheritance_indexed_at ON inheritance_index(indexed_at)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_external_indexed_at ON project_external_infos(indexed_at)",
            [],
        )?;

        Ok(())
    }

    /// Check if current git state differs from cached state
    /// Called from: LSP initialize (startup validation)
    pub fn is_git_state_stale(&self) -> Result<bool> {
        let current_state = self.get_current_git_state()?;
        let conn = self.conn.lock().unwrap();

        let stored_state = conn.prepare(
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
            Ok(stored) => Ok(stored.head_commit != current_state.head_commit
                || stored.branch != current_state.branch
                || stored.dependencies_hash != current_state.dependencies_hash),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(true), // No stored state
            Err(e) => Err(e.into()),
        }
    }

    /// Update stored git state after successful indexing
    /// Called from: After indexing completes, workspace/didChangeWatchedFiles on .git changes
    pub fn update_git_state(&self, git_state: GitState) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        self.conn.lock().unwrap().execute(
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



    /// Store symbol index to database
    /// Called from: After indexing project files, textDocument/didSave (incremental)
    pub fn store_symbol_index(&self, map: &DashMap<(PathBuf, String), PathBuf>) -> Result<()> {
        tracing::debug!("store_symbol_index: storing {} symbols", map.len());
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Delete existing entries for this project
        tx.execute(
            "DELETE FROM symbol_index WHERE project_path = ?",
            params![self.project_path.to_string_lossy()],
        )?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        {
            // Prepare insert statement in its own scope
            let mut stmt = tx.prepare(
                "INSERT INTO symbol_index 
                 (project_path, fully_qualified_name, file_path, mtime, size, indexed_at) 
                 VALUES (?, ?, ?, ?, ?, ?)",
            )?;

            for entry in map.iter() {
                let ((project_path, fqn), file_path) = (entry.key(), entry.value());

                // Store all project-related entries (allow subdirectories)
                // Skip only if the project_path is not under our workspace
                if !project_path.starts_with(&self.project_path) {
                    tracing::debug!("store_symbol_index: skipping symbol '{}' - project_path {:?} doesn't start with workspace {:?}", fqn, project_path, self.project_path);
                    continue;
                }
                

                // Get file metadata
                let (mtime, size) = match fs::metadata(file_path) {
                    Ok(metadata) => {
                        let mtime =
                            metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs() as i64;
                        let size = metadata.len() as i64;
                        (mtime, size)
                    }
                    Err(_) => (0, 0), // File might not exist
                };

                stmt.execute(params![
                    project_path.to_string_lossy(), // Use the symbol's project_path, not workspace path
                    fqn,
                    file_path.to_string_lossy(),
                    mtime,
                    size,
                    now
                ])?;
            }
        } // stmt is dropped here

        tx.commit()?;
        Ok(())
    }


    /// Store builtin infos to database
    /// Called from: After scanning JAVA_HOME/GROOVY_HOME, when builtins change
    pub fn store_builtin_infos(&self, map: &DashMap<String, SourceFileInfo>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

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
                        let mtime =
                            metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs() as i64;
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
    pub fn load_inheritance_index(
        &self,
    ) -> Result<DashMap<(PathBuf, String), Vec<(PathBuf, usize, usize)>>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT project_path, type_name, locations FROM inheritance_index WHERE project_path LIKE ?"
        )?;

        let workspace_pattern = format!("{}%", self.project_path.to_string_lossy());
        let rows = stmt.query_map(params![workspace_pattern], |row| {
            let project_path: String = row.get(0)?;
            let type_name: String = row.get(1)?;
            let locations_blob: Vec<u8> = row.get(2)?;
            Ok((PathBuf::from(project_path), type_name, locations_blob))
        })?;

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
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Delete existing entries for this project
        tx.execute(
            "DELETE FROM inheritance_index WHERE project_path = ?",
            params![self.project_path.to_string_lossy()],
        )?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        {
            // Prepare insert statement in its own scope
            let mut stmt = tx.prepare(
                "INSERT INTO inheritance_index 
                 (project_path, type_name, locations, indexed_at) 
                 VALUES (?, ?, ?, ?)",
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


    /// Store project external infos to database
    /// Called from: After gradle dependency resolution, workspace/didChangeWatchedFiles on build.gradle
    pub fn store_project_external_infos(
        &self,
        map: &DashMap<(PathBuf, String), SourceFileInfo>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Delete existing entries for this project
        tx.execute(
            "DELETE FROM project_external_infos WHERE project_path = ?",
            params![self.project_path.to_string_lossy()],
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
                        let mtime =
                            metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs() as i64;
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




    /// Get current git state for comparison
    fn get_current_git_state(&self) -> Result<GitState> {
        // Get HEAD commit
        let head_output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.project_path)
            .output()
            .context("Failed to execute git rev-parse HEAD")?;

        let head_commit = String::from_utf8_lossy(&head_output.stdout)
            .trim()
            .to_string();

        // Get current branch
        let branch_output = Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&self.project_path)
            .output()
            .context("Failed to execute git symbolic-ref")?;

        let branch = if branch_output.status.success() {
            String::from_utf8_lossy(&branch_output.stdout)
                .trim()
                .to_string()
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
        let canonical_path = project_path
            .canonicalize()
            .unwrap_or_else(|_| project_path.to_path_buf());

        let mut hasher = Sha256::new();
        hasher.update(canonical_path.to_string_lossy().as_bytes());
        let result = hasher.finalize();

        // Take first 16 chars of hex representation
        format!("{:x}", result)[..16].to_string()
    }


    /// Store project metadata to database
    pub fn store_project_metadata(
        &self,
        project_metadata: &DashMap<PathBuf, ProjectMetadata>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Create table if it doesn't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS project_metadata (
                project_path TEXT PRIMARY KEY,
                inter_project_deps TEXT,
                external_dep_names TEXT,
                indexing_status TEXT
            )",
            [],
        )?;

        // Clear existing data
        conn.execute("DELETE FROM project_metadata", [])?;

        // Store project metadata
        let mut stmt = conn.prepare(
            "INSERT INTO project_metadata (project_path, inter_project_deps, external_dep_names, indexing_status) VALUES (?, ?, ?, ?)"
        )?;

        for entry in project_metadata.iter() {
            let (project_path, metadata) = (entry.key(), entry.value());

            // Serialize inter_project_deps
            let inter_deps: Vec<String> = metadata
                .inter_project_deps
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let inter_deps_json = serde_json::to_string(&inter_deps)?;

            // Serialize external_dep_names
            let external_deps: Vec<String> = metadata
                .external_dep_names
                .iter()
                .map(|s| s.clone())
                .collect();
            let external_deps_json = serde_json::to_string(&external_deps)?;

            let indexing_status = format!("{:?}", metadata.indexing_status);

            stmt.execute(params![
                project_path.to_string_lossy(),
                inter_deps_json,
                external_deps_json,
                indexing_status
            ])?;
        }

        Ok(())
    }

    /// Load project metadata from database
    pub fn load_project_metadata(&self) -> Result<DashMap<PathBuf, ProjectMetadata>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT project_path, inter_project_deps, external_dep_names, indexing_status FROM project_metadata"
        )?;

        let project_metadata = DashMap::new();
        let rows = stmt.query_map([], |row| {
            let project_path_str: String = row.get(0)?;
            let inter_deps_json: String = row.get(1)?;
            let external_deps_json: String = row.get(2)?;
            let indexing_status_str: String = row.get(3)?;

            Ok((
                project_path_str,
                inter_deps_json,
                external_deps_json,
                indexing_status_str,
            ))
        })?;

        for row_result in rows {
            let (project_path_str, inter_deps_json, external_deps_json, indexing_status_str) =
                row_result?;
            let project_path = PathBuf::from(project_path_str);

            // Deserialize inter_project_deps
            let inter_deps_vec: Vec<String> =
                serde_json::from_str(&inter_deps_json).unwrap_or_default();
            let inter_project_deps = Arc::new(DashSet::new());
            for dep in inter_deps_vec {
                inter_project_deps.insert(PathBuf::from(dep));
            }

            // Deserialize external_dep_names
            let external_deps_vec: Vec<String> =
                serde_json::from_str(&external_deps_json).unwrap_or_default();
            let external_dep_names = Arc::new(DashSet::new());
            for dep in external_deps_vec {
                external_dep_names.insert(dep);
            }

            // Parse indexing status
            let indexing_status = match indexing_status_str.as_str() {
                "InProgress" => IndexingStatus::InProgress,
                "Completed" => IndexingStatus::Completed,
                _ => IndexingStatus::InProgress,
            };

            let metadata = ProjectMetadata {
                inter_project_deps,
                external_dep_names,
                indexing_status,
            };

            project_metadata.insert(project_path, metadata);
        }

        Ok(project_metadata)
    }

    /// Lazy lookup for a single symbol from database
    /// Called from: go-to-definition requests when symbol not in memory cache
    pub fn lookup_symbol(&self, project_root: &PathBuf, fqn: &str) -> Result<Option<PathBuf>> {

        let conn = self.conn.lock().unwrap();

        // First try project-specific lookup (current behavior)
        let mut stmt = conn.prepare(
            "SELECT file_path FROM symbol_index WHERE project_path = ? AND fully_qualified_name = ?"
        )?;

        let result = stmt.query_row(params![project_root.to_string_lossy(), fqn], |row| {
            let file_path: String = row.get(0)?;
            Ok(PathBuf::from(file_path))
        });

        // If found in current project, return it
        match result {
            Ok(file_path) => {
                return Ok(Some(file_path));
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Continue to workspace search below
            }
            Err(e) => return Err(e.into()),
        }

        let workspace_root = self.project_path.clone();
        let workspace_pattern = format!("{}%", workspace_root.to_string_lossy());


        let mut stmt = conn.prepare(
            "SELECT file_path FROM symbol_index WHERE project_path LIKE ? AND fully_qualified_name = ?"
        )?;

        let result = stmt.query_row(params![workspace_pattern, fqn], |row| {
            let file_path: String = row.get(0)?;
            Ok(PathBuf::from(file_path))
        });

        match result {
            Ok(file_path) => {
                Ok(Some(file_path))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Symbol not found in workspace
                {
                    let mut debug_stmt = conn.prepare("SELECT fully_qualified_name, project_path FROM symbol_index WHERE fully_qualified_name LIKE ? LIMIT 5")?;
                    let pattern = format!("%{}%", fqn.split('.').last().unwrap_or(fqn));
                    debug!(
                        "DEBUG: Searching for similar symbols with pattern: {}",
                        pattern
                    );

                    let rows = debug_stmt.query_map(params![pattern], |row| {
                        let fqn_db: String = row.get(0)?;
                        let project_path_db: String = row.get(1)?;
                        Ok((fqn_db, project_path_db))
                    })?;

                    for row in rows {
                        if let Ok((fqn_db, project_path_db)) = row {
                            debug!(
                                "DEBUG: Similar symbol found: '{}' in project '{}'",
                                fqn_db, project_path_db
                            );
                        }
                    }
                }
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Called from: go-to-definition requests when builtin not in memory cache
    pub fn lookup_builtin_info(&self, class_name: &str) -> Result<Option<SourceFileInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT source_path, zip_internal_path, dependency_info FROM builtin_infos WHERE class_name = ?"
        )?;

        let result = stmt.query_row(params![class_name], |row| {
            let source_path: String = row.get(0)?;
            let zip_internal_path: Option<String> = row.get(1)?;
            let dependency_info_json: Option<String> = row.get(2)?;
            Ok((source_path, zip_internal_path, dependency_info_json))
        });

        match result {
            Ok((source_path, zip_internal_path, dependency_info_json)) => {
                let dependency = if let Some(json) = dependency_info_json {
                    deserialize_external_dependency(&json).ok().flatten()
                } else {
                    None
                };

                let source_info =
                    SourceFileInfo::new(PathBuf::from(source_path), zip_internal_path, dependency);

                Ok(Some(source_info))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Lazy lookup for project external info from database
    /// Called from: go-to-definition requests when external dependency not in memory cache
    pub fn lookup_project_external_info(
        &self,
        project_root: &PathBuf,
        type_name: &str,
    ) -> Result<Option<SourceFileInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT source_path, zip_internal_path, dependency_info FROM project_external_infos WHERE project_path = ? AND type_name = ?"
        )?;

        let result = stmt.query_row(params![project_root.to_string_lossy(), type_name], |row| {
            let source_path: String = row.get(0)?;
            let zip_internal_path: Option<String> = row.get(1)?;
            let dependency_info_json: Option<String> = row.get(2)?;
            Ok((source_path, zip_internal_path, dependency_info_json))
        });

        match result {
            Ok((source_path, zip_internal_path, dependency_info_json)) => {
                let dependency = if let Some(json) = dependency_info_json {
                    deserialize_external_dependency(&json).ok().flatten()
                } else {
                    None
                };

                let source_info =
                    SourceFileInfo::new(PathBuf::from(source_path), zip_internal_path, dependency);

                Ok(Some(source_info))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Bulk store all cached data  
    /// Called from: After complete project indexing, LSP shutdown
    pub fn store_all_caches(
        &self,
        symbol_index: &DashMap<(PathBuf, String), PathBuf>,
        builtin_infos: &DashMap<String, SourceFileInfo>,
        inheritance_index: &DashMap<(PathBuf, String), Vec<(PathBuf, usize, usize)>>,
        project_external_infos: &DashMap<(PathBuf, String), SourceFileInfo>,
        project_metadata: &DashMap<PathBuf, ProjectMetadata>,
    ) -> Result<()> {
        // Store each cache separately to handle partial failures better
        if let Err(e) = self.store_symbol_index(symbol_index) {
            tracing::error!("Failed to store symbol index: {}", e);
        }

        let _ = self.store_builtin_infos(builtin_infos);

        let _ = self.store_inheritance_index(inheritance_index);

        let _ = self.store_project_external_infos(project_external_infos);

        let _ = self.store_project_metadata(project_metadata);

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
