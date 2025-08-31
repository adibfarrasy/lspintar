use std::{collections::HashSet, path::PathBuf};

use crate::{
    core::{
        constants::{EXTENSIONS, PROJECT_ROOT_MARKER, SOURCE_DIRS},
        utils::{create_parser_for_language, detect_language_from_path, find_project_root},
    },
    languages::{
        groovy::symbols::extract_groovy_symbols, 
        java::symbols::extract_java_symbols,
        kotlin::symbols::extract_kotlin_symbols,
    },
};
use super::source_file_info::SourceFileInfo;
use anyhow::{Context, Result};
use tokio::{fs, task::spawn_blocking};
use tracing::debug;
use tree_sitter::Tree;
use walkdir::WalkDir;

#[tracing::instrument(skip_all)]
pub fn find_workspace_root(dir: &PathBuf) -> Option<PathBuf> {
    // First, find the immediate project root for this directory
    let project_root = find_project_root(dir)?;
    
    // Check if parent directory is a proper multi-project workspace
    // (has workspace-level configuration like settings.gradle.kts)
    if let Some(parent) = project_root.parent() {
        // Check if parent has workspace-level markers
        let workspace_markers = ["settings.gradle", "settings.gradle.kts", ".git"];
        if workspace_markers.iter().any(|marker| parent.join(marker).exists()) {
            // Count project directories in this workspace
            let mut project_count = 0;
            
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // Check if this directory has project markers (but not workspace markers)
                        let has_project_marker = PROJECT_ROOT_MARKER.iter().any(|marker| path.join(marker).exists());
                        let has_workspace_marker = workspace_markers.iter().any(|marker| path.join(marker).exists());
                        
                        if has_project_marker && !has_workspace_marker {
                            project_count += 1;
                            if project_count > 1 {
                                // Found multiple projects in a proper workspace, parent is workspace root
                                return Some(parent.to_path_buf());
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Single project workspace or no proper multi-project structure - project root is workspace root
    Some(project_root)
}

#[tracing::instrument(skip_all)]
pub fn find_project_roots(dir: &PathBuf) -> Result<Vec<PathBuf>> {
    let workspace_dir = find_workspace_root(dir).context("Cannot find workspace root directory")?;

    let mut project_roots = HashSet::new();

    // For multi-project workspaces, also check subdirectories
    // (e.g., Gradle multi-module projects, monorepos)
    for entry in WalkDir::new(&workspace_dir)
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


pub async fn collect_source_files(
    project_root: &PathBuf,
    is_external_dependency: bool,
) -> Result<Vec<PathBuf>> {
    let mut source_files = Vec::new();

    if is_external_dependency {
        scan_directory_for_sources(&project_root, &mut source_files).await?;
    } else {
        for src_dir in &SOURCE_DIRS {
            let full_path = project_root.join(src_dir);
            if full_path.exists() {
                scan_directory_for_sources(&full_path, &mut source_files).await?;
            }
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

#[tracing::instrument(skip_all)]
pub async fn parse_source_files_parallel(
    source_files: Vec<PathBuf>,
) -> Result<Vec<ParsedSourceFile>> {
    let tasks: Vec<_> = source_files.into_iter().map(parse_single_file).collect();

    let results = futures::future::join_all(tasks).await;
    Ok(results
        .into_iter()
        .filter_map(|r| r.inspect_err(|r| debug!("error: {:#?}", r)).ok())
        .collect())
}

async fn parse_single_file(file_path: PathBuf) -> Result<ParsedSourceFile> {
    let content = fs::read_to_string(&file_path)
        .await
        .context(format!("Failed to read file: {:?}", file_path))?;

    spawn_blocking(move || {
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
    })
    .await?
}

#[tracing::instrument(skip_all)]
pub async fn extract_symbol_definitions(
    parsed_files: Vec<ParsedSourceFile>,
) -> Result<Vec<SymbolDefinition>> {
    let tasks: Vec<_> = parsed_files
        .into_iter()
        .map(|parsed_file| {
            tokio::task::spawn_blocking(move || extract_symbols_from_tree_by_language(&parsed_file))
        })
        .collect();

    let results = futures::future::join_all(tasks).await;

    let mut all_symbols = Vec::new();
    for result in results {
        if let Ok(Ok(symbols)) = result {
            all_symbols.extend(symbols);
        } else {
            debug!("error: {:#?}", result);
        }
    }

    Ok(all_symbols)
}

fn extract_symbols_from_tree_by_language(
    parsed_file: &ParsedSourceFile,
) -> Result<Vec<SymbolDefinition>> {
    let result = match parsed_file.language.as_str() {
        "groovy" => extract_groovy_symbols(parsed_file),
        "java" => extract_java_symbols(parsed_file),
        "kotlin" => extract_kotlin_symbols(parsed_file),
        _ => {
            // Unsupported language, skip
            debug!("Unsupported language: {}", parsed_file.language);
            Ok(vec![])
        }
    };
    
    if let Err(e) = &result {
        debug!("Failed to extract symbols from {}: {:?}", parsed_file.file_path.display(), e);
    }
    
    result
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
    pub fully_qualified_name: String,
    pub source_file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub extends: Option<String>,
    pub implements: Vec<String>,
}

/// Extract symbol definitions from a SourceFileInfo (for decompiled content)
pub fn extract_symbols_from_source_file_info(source_info: &SourceFileInfo) -> Result<Vec<SymbolDefinition>> {
    let content = source_info.get_content()?;
    let tree = source_info.get_tree()?;
    
    // Determine language from the source path or zip internal path
    let language = if let Some(zip_path) = &source_info.zip_internal_path {
        if zip_path.ends_with(".groovy") {
            "groovy"
        } else if zip_path.ends_with(".kt") {
            "kotlin"
        } else {
            "java" // Default to Java for decompiled content
        }
    } else if let Some(ext) = source_info.source_path.extension().and_then(|s| s.to_str()) {
        match ext {
            "groovy" => "groovy",
            "kt" | "kts" => "kotlin",
            _ => "java",
        }
    } else {
        "java" // Default to Java for decompiled content
    };

    let parsed_file = ParsedSourceFile {
        file_path: source_info.source_path.clone(),
        content,
        tree,
        language: language.to_string(),
    };

    extract_symbols_from_tree_by_language(&parsed_file)
}
