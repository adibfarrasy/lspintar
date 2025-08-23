use std::path::PathBuf;
use anyhow::Context;
use tracing::{debug, error};

use crate::{
    core::{
        build_tools::ExternalDependency,
        constants::TEMP_DIR_PREFIX,
        dependency_cache::source_file_info::SourceFileInfo,
        utils::path_to_file_uri,
    },
};

/// Creates a temporary directory path for a dependency
pub fn dependency_temp_dir(dependency: Option<ExternalDependency>) -> PathBuf {
    let base_dir = std::env::temp_dir().join(TEMP_DIR_PREFIX);

    match dependency {
        Some(dep) => base_dir.join(dep.to_path_string()),
        None => base_dir.join("builtin"),
    }
}

/// Extracts a ZIP/JAR file to a temporary directory
pub fn extract_zip_file_to_temp(source_info: &SourceFileInfo) -> Option<()> {
    let temp_dir = dependency_temp_dir(source_info.dependency.clone());

    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        error!("Failed to create temp directory: {}", e);
        return None;
    }

    let zip_file = std::fs::File::open(&source_info.source_path)
        .context(format!(
            "failed to open zip file {:?}",
            source_info.source_path
        ))
        .ok()?;

    let mut archive = zip::ZipArchive::new(zip_file)
        .context("failed to read zip archive")
        .ok()?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .context(format!("failed to get file at index {}", i))
            .ok()?;

        if file.is_dir() {
            continue;
        }

        let file_path = temp_dir.join(file.name());

        if let Some(parent) = file_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!(
                    "Failed to create directory structure for {:?}: {}",
                    parent, e
                );
                continue;
            }
        }

        let mut output = std::fs::File::create(&file_path)
            .context(format!("failed to create file {:?}", file_path))
            .ok()?;

        if let Err(e) = std::io::copy(&mut file, &mut output) {
            error!("Failed to extract file {}: {}", file.name(), e);
            continue;
        }
    }

    debug!("Extracted dependency to: {:?}", temp_dir);
    Some(())
}

/// Gets the URI for a source file, handling both direct files and ZIP/JAR extraction
pub fn get_uri(external_info: &SourceFileInfo) -> Option<String> {
    if let Some(zip_internal_path) = &external_info.zip_internal_path {
        let temp_dir = dependency_temp_dir(external_info.dependency.clone());
        let target_file = temp_dir.join(zip_internal_path);
        
        // Always ensure the file exists - extract if directory doesn't exist or if specific file is missing
        if !temp_dir.exists() || !target_file.exists() {
            extract_zip_file_to_temp(external_info);
        }

        path_to_file_uri(&target_file)
    } else {
        path_to_file_uri(&external_info.source_path)
    }
}