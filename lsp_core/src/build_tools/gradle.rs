use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
};

use crate::build_tools::BuildToolHandler;

pub struct GradleHandler;

impl BuildToolHandler for GradleHandler {
    fn is_project(&self, root: &Path) -> bool {
        root.join("build.gradle").exists()
            || root.join("build.gradle.kts").exists()
            || root.join("settings.gradle").exists()
            || root.join("settings.gradle.kts").exists()
    }

    fn get_dependency_paths(&self, root: &Path) -> Result<Vec<(PathBuf, Option<PathBuf>)>> {
        let init_script = r#"
        allprojects {
            afterEvaluate {
                if (plugins.hasPlugin('java') || plugins.hasPlugin('groovy')) {
                    task lspClasspath {
                        doLast {
                            configurations.compileClasspath.files.each {
                                println it.absolutePath
                            }
                        }
                    }
                    
                    task lspSources {
                        doLast {
                            configurations.compileClasspath.resolvedConfiguration.resolvedArtifacts.each { artifact ->
                                def id = artifact.moduleVersion.id
                                try {
                                    def dep = dependencies.create("${id.group}:${id.name}:${id.version}:sources")
                                    def sourceConfig = configurations.detachedConfiguration(dep)
                                    sourceConfig.files.each { sourceJar ->
                                        println sourceJar.absolutePath
                                    }
                                } catch (Exception e) {
                                    // Source not available, skip
                                }
                            }
                        }
                    }
                }
            }
        }
        "#;

        let temp_init = std::env::temp_dir().join("lsp-gradle-init.gradle");
        std::fs::write(&temp_init, init_script)?;

        let gradle_cmd = if root.join("gradlew").exists() {
            "./gradlew"
        } else {
            "gradle"
        };
        let output = Command::new(gradle_cmd)
            .current_dir(root)
            .args([
                "-I",
                temp_init.to_str().unwrap(),
                "lspClasspath",
                "lspSources",
                "-q",
            ])
            .output()
            .context("Failed to execute gradle")?;

        if !output.status.success() {
            anyhow::bail!("Gradle failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        let (source_jars, bytecode_jars): (Vec<PathBuf>, Vec<PathBuf>) =
            String::from_utf8(output.stdout)?
                .lines()
                .map(|line| PathBuf::from(line.trim()))
                .collect::<HashSet<_>>()
                .into_iter()
                .filter(|p| p.exists())
                .partition(|p| p.to_string_lossy().contains("-sources.jar"));

        if bytecode_jars.is_empty() || source_jars.is_empty() {
            return Ok(vec![]);
        }

        let source_map: HashMap<String, PathBuf> = source_jars
            .into_iter()
            .filter_map(|path| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|name| name.trim_end_matches("-sources").to_string())
                    .map(|base_name| (base_name, path))
            })
            .collect();

        Ok(bytecode_jars
            .into_iter()
            .map(|bytecode| {
                let source = bytecode
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|bytecode_name| source_map.get(bytecode_name).cloned());

                (bytecode, source)
            })
            .collect())
    }
}
