use anyhow::{anyhow, Context, Result};
use std::{
    fs,
    path::PathBuf,
    process::Command,
};
use tempfile::TempDir;
use tracing::{debug, warn};

/// Java decompiler integration for converting .class files to .java source
pub struct JavaDecompiler {
    decompiler_jar_path: Option<PathBuf>,
    temp_dir: TempDir,
}

impl JavaDecompiler {
    pub fn new() -> Result<Self> {
        let temp_dir = tempfile::tempdir()
            .context("Failed to create temporary directory for decompilation")?;
        
        Ok(Self {
            decompiler_jar_path: find_decompiler_jar(),
            temp_dir,
        })
    }

    /// Decompile a single .class file to .java source code
    pub fn decompile_class(&self, class_name: &str, class_bytes: &[u8]) -> Result<String> {
        if self.decompiler_jar_path.is_none() {
            return Err(anyhow!("Java decompiler JAR not found. Please install FernFlower or CFR decompiler."));
        }

        let decompiler_jar = self.decompiler_jar_path.as_ref().unwrap();
        
        // Create temporary input and output directories
        let input_dir = self.temp_dir.path().join("input");
        let output_dir = self.temp_dir.path().join("output");
        
        fs::create_dir_all(&input_dir)?;
        fs::create_dir_all(&output_dir)?;

        // Write class file to temporary location
        let class_file_name = format!("{}.class", class_name.replace('.', "/"));
        let class_file_path = input_dir.join(&class_file_name);
        
        // Ensure parent directories exist
        if let Some(parent) = class_file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        fs::write(&class_file_path, class_bytes)?;

        debug!("Decompiling {} using Java decompiler", class_name);

        // Run CFR decompiler (simpler than FernFlower)
        let output = Command::new("java")
            .args(&[
                "-jar",
                decompiler_jar.to_string_lossy().as_ref(),
                class_file_path.to_string_lossy().as_ref(),
                "--outputdir",
                output_dir.to_string_lossy().as_ref(),
                "--caseinsensitivefs", "true",
            ])
            .output()
            .context("Failed to execute Java decompiler")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Java decompilation failed for {}: {}", class_name, stderr);
            return Err(anyhow!("Decompilation failed: {}", stderr));
        }

        // Read the decompiled .java file
        let java_file_name = format!("{}.java", class_name.replace('.', "/"));
        let java_file_path = output_dir.join(&java_file_name);
        
        if !java_file_path.exists() {
            return Err(anyhow!("Decompiled file not found: {}", java_file_path.display()));
        }

        let decompiled_source = fs::read_to_string(&java_file_path)
            .context("Failed to read decompiled source file")?;

        debug!("Successfully decompiled {} ({} bytes)", class_name, decompiled_source.len());
        Ok(decompiled_source)
    }

    /// Decompile multiple .class files to .java source code in a single CFR invocation
    pub fn decompile_classes_batch(&self, classes: &[(String, Vec<u8>)]) -> Result<Vec<(String, String)>> {
        if self.decompiler_jar_path.is_none() {
            return Err(anyhow!("Java decompiler JAR not found. Please install FernFlower or CFR decompiler."));
        }

        if classes.is_empty() {
            return Ok(Vec::new());
        }

        let decompiler_jar = self.decompiler_jar_path.as_ref().unwrap();
        
        // Create temporary input and output directories
        let batch_id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        let input_dir = self.temp_dir.path().join(format!("batch_input_{}", batch_id));
        let output_dir = self.temp_dir.path().join(format!("batch_output_{}", batch_id));
        
        fs::create_dir_all(&input_dir)?;
        fs::create_dir_all(&output_dir)?;

        // Write all class files to temporary location
        for (class_name, class_bytes) in classes {
            let class_file_name = format!("{}.class", class_name.replace('.', "/"));
            let class_file_path = input_dir.join(&class_file_name);
            
            // Ensure parent directories exist
            if let Some(parent) = class_file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            
            fs::write(&class_file_path, class_bytes)?;
        }

        debug!("Batch decompiling {} classes using CFR", classes.len());

        // Run CFR decompiler on the entire input directory
        let output = Command::new("java")
            .args(&[
                "-jar",
                decompiler_jar.to_string_lossy().as_ref(),
                input_dir.to_string_lossy().as_ref(),
                "--outputdir",
                output_dir.to_string_lossy().as_ref(),
                "--caseinsensitivefs", "true",
            ])
            .output()
            .context("Failed to execute Java decompiler")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Batch decompilation failed: {}", stderr);
            return Err(anyhow!("Batch decompilation failed: {}", stderr));
        }

        // Read all decompiled .java files
        let mut results = Vec::new();
        for (class_name, _) in classes {
            let java_file_name = format!("{}.java", class_name.replace('.', "/"));
            let java_file_path = output_dir.join(&java_file_name);
            
            if java_file_path.exists() {
                match fs::read_to_string(&java_file_path) {
                    Ok(decompiled_source) => {
                        debug!("Successfully batch decompiled {} ({} bytes)", class_name, decompiled_source.len());
                        results.push((class_name.clone(), decompiled_source));
                    }
                    Err(e) => {
                        warn!("Failed to read decompiled file for {}: {}", class_name, e);
                    }
                }
            } else {
                warn!("Decompiled file not found for {}: {}", class_name, java_file_path.display());
            }
        }

        debug!("Batch decompilation completed: {}/{} classes successful", results.len(), classes.len());
        Ok(results)
    }
}

/// Find Java decompiler JAR in common locations
fn find_decompiler_jar() -> Option<PathBuf> {
    let mut possible_paths = vec![
        // Common installation locations
        PathBuf::from("/usr/share/java/cfr.jar"),
        PathBuf::from("/usr/share/java/fernflower.jar"),
        PathBuf::from("/opt/cfr/cfr.jar"),
        PathBuf::from("/opt/fernflower/fernflower.jar"),
        
        // Current directory
        PathBuf::from("cfr.jar"),
        PathBuf::from("fernflower.jar"),
        PathBuf::from("./cfr.jar"),
        PathBuf::from("./fernflower.jar"),
    ];

    // Add user home locations if home directory exists
    if let Some(home) = dirs::home_dir() {
        possible_paths.push(home.join(".local/share/decompiler/cfr.jar"));
        possible_paths.push(home.join(".local/share/fernflower/fernflower.jar"));
        possible_paths.push(home.join("bin/cfr.jar"));
        possible_paths.push(home.join("bin/fernflower.jar"));
    }

    for path in possible_paths.into_iter() {
        if path.exists() {
            debug!("Found Java decompiler JAR at: {}", path.display());
            return Some(path);
        }
    }

    warn!("Java decompiler JAR not found. Please install CFR or FernFlower decompiler.");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_decompiler_jar() {
        // This test will only pass if a decompiler is actually installed
        let jar_path = find_decompiler_jar();
        if jar_path.is_some() {
            println!("Java decompiler found at: {:?}", jar_path);
        } else {
            println!("Java decompiler not found (this is expected in CI)");
        }
    }
}