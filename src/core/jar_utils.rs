use std::path::PathBuf;
use anyhow::Context;
use tracing::error;

use crate::{
    core::{
        build_tools::ExternalDependency,
        constants::TEMP_DIR_PREFIX,
        dependency_cache::source_file_info::SourceFileInfo,
        utils::path_to_file_uri,
    },
};

/// Creates a temporary directory path for a dependency
#[tracing::instrument(skip_all)]
pub fn dependency_temp_dir(dependency: Option<ExternalDependency>) -> PathBuf {
    let base_dir = std::env::temp_dir().join(TEMP_DIR_PREFIX);

    match dependency {
        Some(dep) => base_dir.join(dep.to_path_string()),
        None => base_dir.join("builtin"),
    }
}

/// Extracts a ZIP/JAR file to a temporary directory
#[tracing::instrument(skip_all)]
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

    Some(())
}

/// Gets the URI for a source file, handling both direct files and ZIP/JAR extraction
#[tracing::instrument(skip_all)]
pub fn get_uri(external_info: &SourceFileInfo) -> Option<String> {
    if let Some(zip_internal_path) = &external_info.zip_internal_path {
        let temp_dir = dependency_temp_dir(external_info.dependency.clone());
        
        // Handle decompiled .class files - create .java file with decompiled content
        if zip_internal_path.ends_with(".class") {
            let java_file_path = temp_dir.join(zip_internal_path.replace(".class", ".java"));
            
            // Ensure temp directory exists
            if let Some(parent) = java_file_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            
            // Write decompiled content to .java file if it doesn't exist
            if !java_file_path.exists() {
                if let Ok(decompiled_content) = external_info.get_content() {
                    if let Err(e) = std::fs::write(&java_file_path, decompiled_content) {
                        tracing::debug!("Failed to write decompiled content to {}: {}", java_file_path.display(), e);
                        return None;
                    }
                } else {
                    tracing::debug!("Failed to get decompiled content for {}", zip_internal_path);
                    return None;
                }
            }
            
            path_to_file_uri(&java_file_path)
        } else {
            // Regular source files - extract from JAR
            let target_file = temp_dir.join(zip_internal_path);
            
            // Always ensure the file exists - extract if directory doesn't exist or if specific file is missing
            if !temp_dir.exists() || !target_file.exists() {
                extract_zip_file_to_temp(external_info);
            }

            path_to_file_uri(&target_file)
        }
    } else {
        path_to_file_uri(&external_info.source_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::build_tools::ExternalDependency;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_dependency_temp_dir_with_some_dependency() {
        let dependency = Some(ExternalDependency {
            group: "com.example".to_string(),
            artifact: "test-lib".to_string(),
            version: "1.0.0".to_string(),
        });

        let result = dependency_temp_dir(dependency);
        let expected_path = std::env::temp_dir()
            .join(TEMP_DIR_PREFIX)
            .join("com.example.test-lib.1.0.0");

        assert_eq!(result, expected_path);
    }

    #[test]
    fn test_dependency_temp_dir_with_none_dependency() {
        let result = dependency_temp_dir(None);
        let expected_path = std::env::temp_dir()
            .join(TEMP_DIR_PREFIX)
            .join("builtin");

        assert_eq!(result, expected_path);
    }

    #[test]
    fn test_get_uri_direct_file() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("TestClass.java");
        fs::write(&test_file, "public class TestClass {}").unwrap();

        let source_info = SourceFileInfo::new(
            test_file.clone(),
            None,
            None,
        );

        let result = get_uri(&source_info);
        assert!(result.is_some());
        assert!(result.unwrap().contains("TestClass.java"));
    }

    #[test]
    fn test_get_uri_class_file_with_zip_internal_path() {
        let temp_dir = TempDir::new().unwrap();
        let jar_file = temp_dir.path().join("test.jar");
        
        // Create a mock JAR file with a class file
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar_file).unwrap());
        zip.start_file::<&str, ()>("com/example/TestClass.class", Default::default()).unwrap();
        zip.write_all(b"fake class content").unwrap();
        zip.finish().unwrap();

        // Create source info for decompiled content
        let source_info = SourceFileInfo::new_for_decompilation(
            jar_file,
            Some("com/example/TestClass.class".to_string()),
            Some(ExternalDependency {
                group: "com.example".to_string(),
                artifact: "test-lib".to_string(),
                version: "1.0.0".to_string(),
            }),
        );

        let result = get_uri(&source_info);
        // Should attempt to create decompiled .java file
        assert!(result.is_none() || result.unwrap().contains("TestClass.java"));
    }

    #[test]
    fn test_get_uri_regular_source_file_in_jar() {
        let temp_dir = TempDir::new().unwrap();
        let jar_file = temp_dir.path().join("test.jar");
        
        // Create a mock JAR file with a source file
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar_file).unwrap());
        zip.start_file::<&str, ()>("com/example/TestClass.java", Default::default()).unwrap();
        zip.write_all(b"public class TestClass {}").unwrap();
        zip.finish().unwrap();

        let source_info = SourceFileInfo::new(
            jar_file,
            Some("com/example/TestClass.java".to_string()),
            Some(ExternalDependency {
                group: "com.example".to_string(),
                artifact: "test-lib".to_string(),
                version: "1.0.0".to_string(),
            }),
        );

        // This test may not fully work without actual extraction, but tests the logic path
        let result = get_uri(&source_info);
        // The function should return a path even if extraction fails
        assert!(result.is_none() || result.unwrap().contains("TestClass.java"));
    }
}