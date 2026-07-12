use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
};

use crate::build_tools::{BuildToolHandler, SubprojectClasspath, pair_jars_with_sources};

pub struct MavenHandler;

/// `mvnw` if the project ships a wrapper, else whatever `mvn` is on PATH.
fn maven_cmd(root: &Path) -> &'static str {
    if root.join("mvnw").exists() {
        "./mvnw"
    } else {
        "mvn"
    }
}

/// Runs `dependency:build-classpath`, optionally scoped to one module (`-pl`),
/// and returns the jars it wrote to the output file.
fn build_classpath(root: &Path, module: Option<&str>) -> Result<Vec<PathBuf>> {
    let out_file = tempfile::NamedTempFile::new()?;
    let out_path = out_file.path().to_owned();

    let mut cmd = Command::new(maven_cmd(root));
    cmd.current_dir(root).args([
        "-q",
        &format!("-Dmdep.outputFile={}", out_path.display()),
    ]);
    match module {
        // `package` first (with `-am`) so sibling reactor modules this one
        // depends on are built and resolvable without being in ~/.m2 yet.
        Some(module) => {
            cmd.args(["-pl", module, "-am", "-DskipTests", "package", "dependency:build-classpath"]);
        }
        None => {
            cmd.arg("dependency:build-classpath");
        }
    }

    let output = cmd.output().context("Failed to execute mvn")?;
    if !output.status.success() {
        anyhow::bail!("Maven failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    // A module with no dependencies leaves the file empty.
    let classpath = std::fs::read_to_string(&out_path).unwrap_or_default();
    Ok(classpath
        .split([':', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// Sibling `*-sources.jar` of a dependency jar, if Maven already downloaded it.
fn sibling_source_jar(jar: &Path) -> Option<PathBuf> {
    let stem = jar.file_stem()?.to_str()?;
    let source = jar.with_file_name(format!("{stem}-sources.jar"));
    source.exists().then_some(source)
}

impl BuildToolHandler for MavenHandler {
    fn is_project(&self, root: &Path) -> bool {
        root.join("pom.xml").exists()
    }

    fn get_dependency_paths(&self, root: &Path) -> Result<Vec<(Option<PathBuf>, Option<PathBuf>)>> {
        // Pulls *-sources.jar into the local repo, next to each dependency jar.
        // Best-effort: dependencies without published sources make this fail,
        // and the bytecode jars are still worth returning.
        let _ = Command::new(maven_cmd(root))
            .current_dir(root)
            .args(["-q", "dependency:sources"])
            .output();

        let jars = build_classpath(root, None)?;
        let sources = jars.iter().filter_map(|jar| sibling_source_jar(jar));
        let all: HashSet<PathBuf> = jars.iter().cloned().chain(sources).collect();

        Ok(pair_jars_with_sources(all))
    }

    fn get_jdk_dependency_path(&self, root: &Path) -> Result<Option<PathBuf>> {
        let java_home = match std::env::var_os("JAVA_HOME") {
            Some(home) => PathBuf::from(home),
            None => {
                let output = Command::new(maven_cmd(root))
                    .current_dir(root)
                    .arg("-version")
                    .output()
                    .context("Failed to execute mvn")?;

                let stdout = String::from_utf8_lossy(&output.stdout);
                match stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("Java version:"))
                    .and_then(|line| line.split_once("runtime:"))
                    .map(|(_, home)| PathBuf::from(home.trim()))
                {
                    Some(home) => home,
                    None => return Ok(None),
                }
            }
        };

        // Java 9+ keeps sources at lib/src.zip; Java 8 at the JDK root.
        Ok([java_home.join("lib/src.zip"), java_home.join("src.zip")]
            .into_iter()
            .find(|p| p.exists()))
    }

    fn is_build_file(&self, path: &Path) -> bool {
        path.file_name().and_then(|n| n.to_str()) == Some("pom.xml")
    }

    fn get_subproject_classpath(&self, root: &Path) -> Result<Vec<SubprojectClasspath>> {
        let modules = parse_modules(&std::fs::read_to_string(root.join("pom.xml"))?);
        if modules.is_empty() {
            return Ok(vec![]);
        }

        Ok(modules
            .iter()
            .filter_map(|module| {
                let module_dir = root.join(module);
                let source_dirs: Vec<PathBuf> = ["java", "kotlin", "groovy"]
                    .iter()
                    .map(|lang| module_dir.join("src/main").join(lang))
                    .filter(|d| d.exists())
                    .collect();

                if source_dirs.is_empty() {
                    return None;
                }

                Some(SubprojectClasspath {
                    source_dirs,
                    jar_paths: build_classpath(root, Some(module)).ok()?,
                })
            })
            .collect())
    }
}

/// Extracts `<module>` entries from an aggregator pom.
///
/// ponytail: flat `<module>` lists only — nested aggregators and
/// property-interpolated module paths need a real XML parser.
fn parse_modules(pom: &str) -> Vec<String> {
    pom.split("<module>")
        .skip(1)
        .filter_map(|rest| rest.split_once("</module>"))
        .map(|(module, _)| module.trim().to_string())
        .filter(|module| !module.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_module_list() {
        let pom = r#"
            <project>
              <modules>
                <module>core</module>
                <module>app</module>
              </modules>
            </project>
        "#;
        assert_eq!(parse_modules(pom), vec!["core", "app"]);
    }

    #[test]
    fn no_modules_in_single_project_pom() {
        assert!(parse_modules("<project><artifactId>x</artifactId></project>").is_empty());
    }
}
