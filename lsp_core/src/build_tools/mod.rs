pub mod gradle;
pub mod maven;
pub mod no_build_tool;

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::build_tools::{gradle::GradleHandler, maven::MavenHandler, no_build_tool::NoBuildTool};

#[derive(Debug, Clone, PartialEq)]
pub enum BuildTool {
    Gradle,
    Maven,
}

/// Maps a single sub-project's source roots to the JARs on its compile/runtime classpath.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubprojectClasspath {
    pub source_dirs: Vec<PathBuf>,
    pub jar_paths: Vec<PathBuf>,
}

impl SubprojectClasspath {
    /// Returns true if `file` lives under one of this sub-project's source roots.
    pub fn contains_file(&self, file: &Path) -> bool {
        self.source_dirs.iter().any(|d| file.starts_with(d))
    }
}

/// Dead network mirrors, VPN-gated Artifactory hosts, or an interactive prompt
/// with no TTY attached can make `mvn`/`gradle` hang forever. Bound every
/// build-tool invocation so indexing falls back to local-source-only instead.
pub const BUILD_TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs `cmd`, killing it and erroring out if it hasn't finished within `timeout`.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Result<Output> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn command")?;

    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Command timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    Ok(Output {
        status,
        stdout: stdout_thread.join().unwrap_or_default(),
        stderr: stderr_thread.join().unwrap_or_default(),
    })
}

pub fn get_build_tool(root: &Path) -> Arc<dyn BuildToolHandler + Send + Sync> {
    let providers: Vec<Arc<dyn BuildToolHandler>> =
        vec![Arc::new(GradleHandler), Arc::new(MavenHandler)];
    providers
        .into_iter()
        .find(|p| p.is_project(root))
        .unwrap_or_else(|| Arc::new(NoBuildTool))
}

/// Splits jar paths into bytecode/source jars and pairs them by base name,
/// e.g. `guava-33.0.jar` <-> `guava-33.0-sources.jar`.
pub fn pair_jars_with_sources(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<(Option<PathBuf>, Option<PathBuf>)> {
    let (source_jars, bytecode_jars): (Vec<PathBuf>, Vec<PathBuf>) = paths
        .into_iter()
        .filter(|p| p.exists())
        .partition(|p| p.to_string_lossy().ends_with("-sources.jar"));

    let base_name = |path: &Path, strip: bool| -> Option<String> {
        path.file_stem().and_then(|s| s.to_str()).map(|name| {
            if strip {
                name.trim_end_matches("-sources").to_string()
            } else {
                name.to_string()
            }
        })
    };

    let source_map: HashMap<String, PathBuf> = source_jars
        .into_iter()
        .filter_map(|path| base_name(&path, true).map(|n| (n, path)))
        .collect();

    let bytecode_map: HashMap<String, PathBuf> = bytecode_jars
        .into_iter()
        .filter_map(|path| base_name(&path, false).map(|n| (n, path)))
        .collect();

    let mut pairs: Vec<(Option<PathBuf>, Option<PathBuf>)> = source_map
        .iter()
        .map(|(name, src)| (bytecode_map.get(name).cloned(), Some(src.clone())))
        .collect();

    // bytecode-only jars (no source)
    for (name, byte) in &bytecode_map {
        if !source_map.contains_key(name) {
            pairs.push((Some(byte.clone()), None));
        }
    }

    pairs
}

pub trait BuildToolHandler: Send + Sync {
    fn is_project(&self, root: &Path) -> bool;
    fn get_dependency_paths(&self, root: &Path) -> Result<Vec<(Option<PathBuf>, Option<PathBuf>)>>;
    fn get_jdk_dependency_path(&self, root: &Path) -> Result<Option<PathBuf>>;
    fn is_build_file(&self, path: &Path) -> bool;
    /// Returns the per-sub-project source-root → classpath JAR mapping.
    /// Returns an empty vec for single-project setups or when not applicable.
    fn get_subproject_classpath(&self, root: &Path) -> Result<Vec<SubprojectClasspath>>;
}

#[cfg(test)]
mod timeout_tests {
    use super::*;

    #[test]
    fn kills_command_that_exceeds_timeout() {
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let err = run_with_timeout(&mut cmd, Duration::from_millis(200)).unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn returns_output_of_fast_command() {
        let mut cmd = Command::new("echo");
        cmd.arg("hi");
        let output = run_with_timeout(&mut cmd, Duration::from_secs(5)).unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hi");
    }
}
