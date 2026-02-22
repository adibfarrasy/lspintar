use std::{fs, path::Path, process::Stdio, time::Duration};

use anyhow::{Context, anyhow};
use tempfile::tempdir;
use tower_lsp::lsp_types::{Position, Range, TextEdit};

use crate::{lsp_error, lsp_warn};

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// Only find direct import match
pub fn naive_resolve_fqn(name: &str, imports: &[String]) -> Option<String> {
    if let Some(import) = imports.iter().find(|i| i.split('.').last() == Some(name)) {
        return Some(import.clone());
    }

    None
}

pub fn decompile_class(
    class_name: &str,
    buffer: &[u8],
    decompiler_jar: &Path,
) -> anyhow::Result<String> {
    let input_dir = tempdir()
        .context("Failed to create temp input dir")?
        .path()
        .join("input");
    let output_dir = tempdir()
        .context("Failed to create temp output dir")?
        .path()
        .join("output");

    fs::create_dir_all(&input_dir)?;
    fs::create_dir_all(&output_dir)?;

    let class_file_name = format!("{}.class", class_name.replace('.', "/"));
    let class_file_path = input_dir.join(&class_file_name);

    if let Some(parent) = class_file_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&class_file_path, buffer)?;

    let mut command = std::process::Command::new("java");
    command.args(&[
        "-jar",
        decompiler_jar.to_string_lossy().as_ref(),
        class_file_path.to_string_lossy().as_ref(),
        "--outputdir",
        output_dir.to_string_lossy().as_ref(),
        "--caseinsensitivefs",
        "true",
    ]);
    let output = execute_with_timeout(command).context("Failed to execute Java decompiler")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Decompilation failed: {}", stderr));
    }

    let java_file_name = format!("{}.java", class_name.replace('.', "/"));
    let java_file_path = output_dir.join(&java_file_name);

    if !java_file_path.exists() {
        return Err(anyhow!(
            "Decompiled file not found: {}",
            java_file_path.display()
        ));
    }

    let decompiled_source =
        fs::read_to_string(&java_file_path).context("Failed to read decompiled source file")?;

    Ok(decompiled_source)
}

const DECOMPILATION_TIMEOUT_SECS: u64 = 5;

pub fn execute_with_timeout(
    mut command: std::process::Command,
) -> anyhow::Result<std::process::Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn decompiler process")?;

    let timeout = Duration::from_secs(DECOMPILATION_TIMEOUT_SECS);
    let start_time = std::time::Instant::now();

    // Use a simple polling approach to check for completion or timeout
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                let output = child
                    .wait_with_output()
                    .context("Failed to collect decompiler output")?;
                return Ok(output);
            }
            Ok(None) => {
                if start_time.elapsed() > timeout {
                    let pid = child.id();
                    lsp_error!(
                        "Decompilation timeout reached ({}s), terminating process {}",
                        DECOMPILATION_TIMEOUT_SECS,
                        pid
                    );

                    // Try to kill the process
                    if let Err(e) = child.kill() {
                        lsp_warn!("Failed to kill decompilation process {}: {}", pid, e);
                    }
                    let _ = child.wait(); // Clean up zombie process

                    return Err(anyhow!(
                        "Decompilation timed out after {} seconds. Process was terminated to prevent CPU exhaustion.",
                        DECOMPILATION_TIMEOUT_SECS
                    ));
                }

                // Sleep briefly before checking again
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(anyhow!("Error waiting for decompiler process: {}", e)
                    .context("Command execution failed"));
            }
        }
    }
}

/// Strip comment signifiers from documentation text
/// Removes /*, *, */, // while preserving multi-line format
#[tracing::instrument(skip_all)]
pub fn strip_comment_signifiers(docs: &str) -> String {
    let mut lines: Vec<String> = docs
        .lines()
        .map(|line| {
            let trimmed = line.trim();

            // Remove /* at start of line
            let without_start = if trimmed.starts_with("/**") {
                trimmed.strip_prefix("/**").unwrap_or(trimmed).trim()
            } else if trimmed.starts_with("/*") {
                trimmed.strip_prefix("/*").unwrap_or(trimmed).trim()
            } else {
                trimmed
            };

            // Remove */ at end of line
            let without_end = if without_start.ends_with("*/") {
                without_start
                    .strip_suffix("*/")
                    .unwrap_or(without_start)
                    .trim()
            } else {
                without_start
            };

            // Remove leading * or // with more aggressive matching
            let without_prefix = if without_end.starts_with("* ") {
                without_end.strip_prefix("* ").unwrap_or(without_end)
            } else if without_end == "*" {
                // Handle standalone asterisks
                ""
            } else if without_end.starts_with("*") && without_end.len() > 1 {
                // Handle * immediately followed by content
                &without_end[1..]
            } else if without_end.starts_with("// ") {
                without_end.strip_prefix("// ").unwrap_or(without_end)
            } else if without_end.starts_with("//") {
                without_end.strip_prefix("//").unwrap_or(without_end)
            } else {
                without_end
            };

            without_prefix.trim().to_string()
        })
        .collect();

    // Remove empty lines at start and end
    while lines.first().map_or(false, |line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().map_or(false, |line| line.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

pub fn extract_receiver(line: &str, char_pos: usize) -> Option<&str> {
    let before = &line[..char_pos];
    let dot_pos = before.rfind('.')?;
    Some(extract_prefix(before, dot_pos))
}

pub fn extract_prefix(line: &str, char_pos: usize) -> &str {
    let before = &line[..char_pos];
    let start = before
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    &before[start..]
}

pub fn get_import_text_edit(
    content: &str,
    fqn: &str,
    package_name: &str,
    parent_fqn: &str,
) -> TextEdit {
    let last_import_line = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with("import "))
        .last()
        .map(|(i, _)| i as u32);

    let insert_line = match last_import_line {
        Some(i) => i + 1,
        None => {
            // no imports in file, fall back to after package declaration
            content
                .lines()
                .enumerate()
                .find(|(_, line)| line.starts_with("package "))
                .map(|(i, _)| i as u32 + 1)
                .unwrap_or(0)
        }
    };

    let range = Range {
        start: Position {
            line: insert_line,
            character: 0,
        },
        end: Position {
            line: insert_line,
            character: 0,
        },
    };

    let autoimport_text = if package_name == parent_fqn {
        fqn
    } else {
        &parent_fqn.replace("#", ".")
    };

    TextEdit {
        range,
        new_text: format!("import {}\n", autoimport_text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_import_text_edit() {
        let cases = vec![
            (
                "package com.example\n\nimport com.example.Foo\nimport com.example.Bar\n\nclass Baz {}",
                "com.example.Qux",
                "com.example",
                "com.example",
                "import com.example.Qux\n",
                4,
            ),
            (
                "class Baz {}",
                "com.example.Foo",
                "com.example",
                "com.example",
                "import com.example.Foo\n",
                0,
            ),
            (
                "package com.example\n\nimport com.example.Foo\n\nclass Baz {}",
                "com.example.Foo#Bar",
                "com.example",
                "com.example.Foo",
                "import com.example.Foo\n",
                3,
            ),
            (
                "package com.example\n\nimport com.example.Foo\n\nclass Baz {}",
                "com.example.Foo#Bar#baz",
                "com.example",
                "com.example.Foo#Bar",
                "import com.example.Foo.Bar\n",
                3,
            ),
        ];

        for (content, fqn, package_name, parent_fqn, expected_text, expected_line) in cases {
            let edit = get_import_text_edit(content, fqn, package_name, parent_fqn);
            assert_eq!(
                edit.range.start.line, expected_line,
                "failed for fqn: {}",
                fqn
            );
            assert_eq!(edit.range.start.character, 0);
            assert_eq!(edit.range.end, edit.range.start);
            assert_eq!(edit.new_text, expected_text, "failed for fqn: {}", fqn);
        }
    }
}
