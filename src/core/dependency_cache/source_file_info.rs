use anyhow::{Context, Result};
use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    sync::{Arc, RwLock},
};
use zip::ZipArchive;
use tracing::debug;

use tree_sitter::Tree;

use crate::core::{
    build_tools::ExternalDependency,
    constants::{GROOVY_PARSER, JAVA_PARSER},
};

#[derive(Debug, Clone)]
pub struct SourceFileInfo {
    pub source_path: PathBuf,
    pub zip_internal_path: Option<String>,
    pub dependency: Option<ExternalDependency>,
    inner: Arc<RwLock<SourceFileInfoInner>>,
}

#[derive(Debug, Default)]
struct SourceFileInfoInner {
    tree: Option<Tree>,
    content: Option<String>,
}

impl SourceFileInfo {
    pub fn new(
        source_path: PathBuf,
        zip_internal_path: Option<String>,
        dependency: Option<ExternalDependency>,
    ) -> Self {
        Self {
            source_path,
            zip_internal_path,
            dependency,
            inner: Arc::new(RwLock::new(SourceFileInfoInner::default())),
        }
    }


    pub fn new_for_decompilation(
        source_path: PathBuf,
        zip_internal_path: Option<String>,
        dependency: Option<ExternalDependency>,
    ) -> Self {
        Self {
            source_path,
            zip_internal_path,
            dependency,
            inner: Arc::new(RwLock::new(SourceFileInfoInner::default())),
        }
    }

    pub fn get_content(&self) -> Result<String> {
        {
            let inner = self.inner.read().unwrap();
            if let Some(ref content) = inner.content {
                return Ok(content.clone());
            }
        }

        // Try loading from source first
        if let Ok(content) = self.load_content() {
            self.inner.write().unwrap().content = Some(content.clone());
            return Ok(content);
        }

        // If loading failed and we have a .class file, try decompilation
        if let Some(zip_path) = &self.zip_internal_path {
            if zip_path.ends_with(".class") {
                if let Ok(content) = self.decompile_on_demand() {
                    self.inner.write().unwrap().content = Some(content.clone());
                    return Ok(content);
                }
            }
        }

        Err(anyhow::anyhow!("Failed to load or decompile content"))
    }

    pub fn get_tree(&self) -> Result<Tree> {
        {
            let inner = self.inner.read().unwrap();
            if let Some(ref tree) = inner.tree {
                return Ok(tree.clone());
            }
        }

        let content = self.get_content()?;
        let tree = self.parse_content(&content)?;
        self.inner.write().unwrap().tree = Some(tree.clone());
        Ok(tree)
    }


    /// Decompile a .class file on-demand when content is requested
    fn decompile_on_demand(&self) -> Result<String> {
        use crate::core::decompiler::JavaDecompiler;
        use std::io::Read;
        use zip::ZipArchive;
        use std::io::Cursor;

        let zip_path = self.zip_internal_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No internal path for decompilation"))?;

        if !zip_path.ends_with(".class") {
            return Err(anyhow::anyhow!("Not a .class file: {}", zip_path));
        }

        debug!("On-demand decompiling: {} from {}", zip_path, self.source_path.display());

        // Read the .class file from the JAR
        let jar_data = std::fs::read(&self.source_path)?;
        let mut archive = ZipArchive::new(Cursor::new(jar_data))?;
        let mut class_file = archive.by_name(zip_path)?;
        let mut class_bytes = Vec::new();
        class_file.read_to_end(&mut class_bytes)?;

        // Extract class name from path
        let class_name = zip_path.strip_suffix(".class")
            .ok_or_else(|| anyhow::anyhow!("Invalid class file path"))?
            .replace('/', ".");

        // Decompile using JavaDecompiler
        let decompiler = JavaDecompiler::new()?;
        let decompiled_content = decompiler.decompile_class(&class_name, &class_bytes)?;

        debug!("Successfully decompiled {} on-demand ({} bytes)", class_name, decompiled_content.len());
        Ok(decompiled_content)
    }

    fn load_content(&self) -> Result<String> {
        if let Some(zip_path) = &self.zip_internal_path {
            let zip_data = fs::read(&self.source_path)?;
            let mut archive = ZipArchive::new(Cursor::new(zip_data))?;
            let mut file = archive.by_name(zip_path)?;
            let mut content = String::new();
            file.read_to_string(&mut content)?;
            Ok(content)
        } else {
            fs::read_to_string(&self.source_path)
                .with_context(|| format!("Failed to read: {:?}", self.source_path))
        }
    }

    fn parse_content(&self, content: &str) -> Result<Tree> {
        let language = if self.source_path.extension().and_then(|s| s.to_str()) == Some("groovy")
            || self
                .zip_internal_path
                .as_ref()
                .map(|p| p.ends_with(".groovy"))
                .unwrap_or(false)
        {
            GROOVY_PARSER.get_or_init(|| tree_sitter_groovy::language())
        } else if self.source_path.extension().and_then(|s| s.to_str()) == Some("kt")
            || self
                .zip_internal_path
                .as_ref()
                .map(|p| p.ends_with(".kt"))
                .unwrap_or(false)
        {
            use crate::core::constants::KOTLIN_PARSER;
            KOTLIN_PARSER.get_or_init(|| tree_sitter_kotlin::language())
        } else {
            JAVA_PARSER.get_or_init(|| tree_sitter_java::LANGUAGE.into())
        };

        let mut parser = tree_sitter::Parser::new();
        parser.set_language(language)?;
        parser
            .parse(content, None)
            .with_context(|| format!("Failed to parse: {:?}", self.source_path))
    }
}

impl ExternalDependency {
    pub fn to_path_string(&self) -> String {
        return format!("{}.{}.{}", &self.group, &self.artifact, &self.version);
    }
}
