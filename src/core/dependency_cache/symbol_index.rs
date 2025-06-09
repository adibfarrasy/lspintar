use std::{collections::HashSet, path::PathBuf};

use crate::{
    core::{
        constants::{EXTENSIONS, PROJECT_ROOT_MARKER, SOURCE_DIRS},
        symbols::SymbolType,
        utils::{create_parser_for_language, detect_language_from_path, find_project_root},
    },
    languages::groovy::symbols::extract_groovy_symbols,
};
use anyhow::{anyhow, Context, Result};
use futures::stream::{self, StreamExt};
use tree_sitter::Tree;
use walkdir::WalkDir;

pub async fn find_project_roots() -> Result<Vec<PathBuf>> {
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;

    let mut project_roots = HashSet::new();

    let root_dir = try_find_workspace_root(&current_dir);

    // For multi-project workspaces, also check subdirectories
    // (e.g., Gradle multi-module projects, monorepos)
    for entry in WalkDir::new(&root_dir)
        .max_depth(3) // or any reasonable depth
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
    {
        let dir_path = entry.path();

        // Skip build output and hidden directories
        if let Some(name) = dir_path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || name == "build" || name == "target" {
                continue;
            }
        }

        // Check if this directory itself is a project root
        if PROJECT_ROOT_MARKER
            .iter()
            .any(|marker| dir_path.join(marker).exists())
        {
            project_roots.insert(dir_path.to_path_buf());
        }
    }

    Ok(project_roots.into_iter().collect())
}

fn try_find_workspace_root(starting_path: &PathBuf) -> PathBuf {
    let mut current_root = starting_path.clone();
    loop {
        match find_project_root(&current_root) {
            Some(parent_dir) => {
                current_root = parent_dir;
            }
            None => break,
        }
    }

    current_root
}

pub async fn collect_source_files(project_root: &PathBuf) -> Result<Vec<PathBuf>> {
    let mut source_files = Vec::new();

    for src_dir in &SOURCE_DIRS {
        let full_path = project_root.join(src_dir);
        if full_path.exists() {
            scan_directory_for_sources(&full_path, &mut source_files).await?;
        }
    }

    Ok(source_files)
}

pub async fn scan_directory_for_sources(
    dir: &PathBuf,
    source_files: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if EXTENSIONS.contains(&ext) {
                source_files.push(path.to_path_buf());
            }
        }
    }
    Ok(())
}

pub async fn parse_source_files_parallel(
    source_files: Vec<PathBuf>,
) -> Result<Vec<ParsedSourceFile>> {
    let max_concurrent: usize = num_cpus::get();

    let parsed_files = stream::iter(source_files)
        .map(|file_path| async move {
            tokio::task::spawn_blocking(move || parse_single_file(file_path))
                .await
                .unwrap_or_else(|_| Err(anyhow!("Task panicked during file parsing")))
        })
        .buffer_unordered(max_concurrent)
        .filter_map(|result| async move { result.ok() })
        .collect::<Vec<_>>()
        .await;

    Ok(parsed_files)
}

fn parse_single_file(file_path: PathBuf) -> Result<ParsedSourceFile> {
    let content = std::fs::read_to_string(&file_path)
        .context(format!("Failed to read file: {:?}", file_path))?;

    let language = detect_language_from_path(&file_path).context("Unsupported file type")?;

    let mut parser = create_parser_for_language(language).context("Failed to create parser")?;

    let tree = parser
        .parse(&content, None)
        .context("Failed to parse source file")?;

    Ok(ParsedSourceFile {
        file_path,
        content,
        tree,
        language: language.to_string(),
    })
}

pub async fn extract_symbol_definitions(
    parsed_files: Vec<ParsedSourceFile>,
) -> Result<Vec<SymbolDefinition>> {
    let mut all_symbols = Vec::new();

    for parsed_file in parsed_files {
        let symbols = tokio::task::spawn_blocking(move || {
            extract_symbols_from_tree_by_language(&parsed_file)
        })
        .await??;

        all_symbols.extend(symbols);
    }

    Ok(all_symbols)
}

fn extract_symbols_from_tree_by_language(
    parsed_file: &ParsedSourceFile,
) -> Result<Vec<SymbolDefinition>> {
    match parsed_file.language.as_str() {
        "groovy" => extract_groovy_symbols(parsed_file),
        "java" => {
            // TODO: Implement Java symbol extraction
            Ok(vec![])
        }
        "kotlin" => {
            // TODO: Implement Kotlin symbol extraction
            Ok(vec![])
        }
        _ => {
            // Unsupported language, skip
            Ok(vec![])
        }
    }
}

// Supporting types
#[derive(Debug, Clone)]
pub struct ParsedSourceFile {
    pub file_path: PathBuf,
    pub content: String,
    pub tree: Tree,
    pub language: String,
}

#[derive(Debug, Clone)]
pub struct SymbolDefinition {
    pub name: String,
    pub fully_qualified_name: String,
    pub symbol_type: SymbolType,
    pub source_file: PathBuf,
    pub line: usize,
    pub column: usize,
}
