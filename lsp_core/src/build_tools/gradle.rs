use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
};

use crate::build_tools::{BuildToolHandler, SubprojectClasspath, pair_jars_with_sources};

pub struct GradleHandler;

impl BuildToolHandler for GradleHandler {
    fn is_project(&self, root: &Path) -> bool {
        root.join("build.gradle").exists()
            || root.join("build.gradle.kts").exists()
            || root.join("settings.gradle").exists()
            || root.join("settings.gradle.kts").exists()
    }

    fn get_dependency_paths(&self, root: &Path) -> Result<Vec<(Option<PathBuf>, Option<PathBuf>)>> {
        let init_script = r#"
        allprojects {
            afterEvaluate {
                if (['java', 'groovy', 'kotlin', 'org.jetbrains.kotlin.jvm']
                    .any { plugins.hasPlugin(it) }) {
                    task lspClasspath {
                        doLast {
                            def allJars = (configurations.compileClasspath.files + configurations.runtimeClasspath.files).unique()
                            allJars.each {
                                println it.absolutePath
                            }
                        }
                    }
                    
                    task lspSources {
                        doLast {
                            def allArtifacts = (configurations.compileClasspath.resolvedConfiguration.resolvedArtifacts + 
                                configurations.runtimeClasspath.resolvedConfiguration.resolvedArtifacts).unique()

                            allArtifacts.each { artifact ->
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

        // Per-process unique path — see get_subproject_classpath for why.
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_init = std::env::temp_dir().join(format!(
            "lsp-gradle-init-{}-{}.gradle",
            std::process::id(),
            unique,
        ));
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

        let jars: HashSet<PathBuf> = String::from_utf8(output.stdout)?
            .lines()
            .map(|line| PathBuf::from(line.trim()))
            .collect();

        Ok(pair_jars_with_sources(jars))
    }

    fn get_jdk_dependency_path(&self, root: &Path) -> Result<Option<PathBuf>> {
        let init_script = r#"
        allprojects {
            task lspJdkSources {
                doLast {
                    def javaHome = org.gradle.internal.jvm.Jvm.current().javaHome
                    // Java 9+ location
                    def libSrcZip = new File(javaHome, 'lib/src.zip')
                    if (libSrcZip.exists()) {
                        println libSrcZip.absolutePath
                        return
                    }
                    
                    // Java 8 location
                    def srcZip = new File(javaHome, 'src.zip')
                    if (srcZip.exists()) {
                        println srcZip.absolutePath
                    }
                }
            }
        }
        "#;

        // Per-process unique path — see comment on get_subproject_classpath
        // for the rationale (parallel invocations clobbering one shared file).
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_init = std::env::temp_dir().join(format!(
            "lsp-jdk-init-{}-{}.gradle",
            std::process::id(),
            unique,
        ));
        std::fs::write(&temp_init, init_script)?;

        let gradle_cmd = if root.join("gradlew").exists() {
            "./gradlew"
        } else {
            "gradle"
        };

        let output = Command::new(gradle_cmd)
            .current_dir(root)
            .args(["-I", temp_init.to_str().unwrap(), "lspJdkSources", "-q"])
            .output()
            .context("Failed to execute gradle")?;

        if !output.status.success() {
            anyhow::bail!("Gradle failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        let src_zip = String::from_utf8(output.stdout)?
            .lines()
            .next()
            .map(|line| PathBuf::from(line.trim()))
            .filter(|p| p.exists());

        Ok(src_zip)
    }

    fn is_build_file(&self, path: &Path) -> bool {
        matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts")
        )
    }

    fn get_subproject_classpath(&self, root: &Path) -> Result<Vec<SubprojectClasspath>> {
        let init_script = r#"
        allprojects {
            afterEvaluate {
                if (['java', 'groovy', 'kotlin', 'org.jetbrains.kotlin.jvm']
                    .any { plugins.hasPlugin(it) }) {
                    task lspSubprojectClasspath {
                        doLast {
                            def sourceDirs = sourceSets.findAll { it.name == 'main' }
                                .collect { it.allSource.srcDirs }
                                .flatten()
                                .findAll { it.exists() }
                                *.absolutePath
                            def jars = (configurations.compileClasspath.files
                                + configurations.runtimeClasspath.files)
                                .unique()
                                *.absolutePath
                            println groovy.json.JsonOutput.toJson([sourceDirs: sourceDirs, jarPaths: jars])
                        }
                    }
                }
            }
        }
        "#;

        // Use a per-process unique path so concurrent invocations (parallel
        // cargo-test binaries, multiple lspintar instances on one machine,
        // etc.) don't clobber each other's init script.  A single shared
        // `/tmp/lsp-gradle-subproject-init.gradle` previously raced on write
        // and produced intermittent empty classpaths — the root cause of the
        // `completion_prefix_with_import` flake.
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_init = std::env::temp_dir().join(format!(
            "lsp-gradle-subproject-init-{}-{}.gradle",
            std::process::id(),
            unique,
        ));
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
                "lspSubprojectClasspath",
                "-q",
            ])
            .output()
            .context("Failed to execute gradle")?;

        if !output.status.success() {
            anyhow::bail!("Gradle failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        let entries = String::from_utf8(output.stdout)?
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                #[derive(serde::Deserialize)]
                struct Raw {
                    #[serde(rename = "sourceDirs")]
                    source_dirs: Vec<String>,
                    #[serde(rename = "jarPaths")]
                    jar_paths: Vec<String>,
                }
                serde_json::from_str::<Raw>(line).ok().map(|r| SubprojectClasspath {
                    source_dirs: r.source_dirs.into_iter().map(PathBuf::from).collect(),
                    jar_paths: r.jar_paths.into_iter().map(PathBuf::from).collect(),
                })
            })
            .collect();

        Ok(entries)
    }
}
