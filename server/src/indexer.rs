use classfile_parser::{
    ClassAccessFlags, class_parser, constant_info::ConstantInfo, field_info::FieldAccessFlags,
    method_info::MethodAccessFlags,
};
use java::JAVA_IMPLICIT_IMPORTS;
use lsp_core::{
    language_support::LanguageSupport, node_types::NodeType, util::naive_resolve_fqn,
    vcs::VcsHandler,
};
use std::{collections::HashMap, fs::File, io::Read, path::Path, sync::Arc};
use zip::ZipArchive;

use crate::{
    models::{
        external_symbol::ExternalSymbol,
        symbol::{Symbol, SymbolMetadata, SymbolParameter},
        symbol_super_mapping::SymbolSuperMapping,
    },
    repo::Repository,
};

use anyhow::{Context, Result, anyhow};
use sqlx::types::Json;
use tree_sitter::{Node, Tree};
use walkdir::WalkDir;

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Indexer {
    languages: HashMap<String, Arc<dyn LanguageSupport>>,
    repo: Arc<Repository>,
    vcs: Arc<dyn VcsHandler>,
}

impl Indexer {
    pub fn new(repo: Arc<Repository>, vcs: Arc<dyn VcsHandler>) -> Self {
        Self {
            languages: HashMap::new(),
            repo,
            vcs,
        }
    }

    pub fn register_language(&mut self, ext: &str, lang: Arc<dyn LanguageSupport>) {
        self.languages.insert(ext.to_string(), lang.clone());
    }

    pub async fn index_workspace(&self, path: &Path) -> Result<()> {
        for entry in WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| !is_excluded(e))
        {
            let entry = entry?;
            if entry.file_type().is_file() {
                self.index_file(&entry.path()).await?;
            }
        }
        Ok(())
    }

    pub async fn index_file(&self, path: &Path) -> Result<()> {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if self.languages.contains_key(ext) {
                let lang = self
                    .languages
                    .get(ext)
                    .ok_or_else(|| anyhow!("failed to get language implementation"))?;
                let parsed = lang
                    .parse(&path)
                    .ok_or_else(|| anyhow!("failed to parse file: {}", path.display()))?;

                let data =
                    self.get_symbols_from_tree(&parsed.0, lang.as_ref(), &path, &parsed.1, false)?;

                self.repo.insert_symbols(&data.0).await?;

                if !data.1.is_empty() {
                    let mappings = data
                        .1
                        .iter()
                        .map(|mapping| {
                            (
                                &*mapping.symbol_fqn,
                                &*mapping.super_short_name,
                                mapping.super_fqn.as_deref(),
                            )
                        })
                        .collect();

                    self.repo.insert_symbol_super_mappings(mappings).await?;
                }
            }
        }
        Ok(())
    }

    fn get_symbols_from_tree(
        &self,
        tree: &Tree,
        lang: &dyn LanguageSupport,
        path: &Path,
        content: &str,
        is_external: bool,
    ) -> Result<(Vec<Symbol>, Vec<SymbolSuperMapping>)> {
        let mut symbols = Vec::new();
        let mut symbol_super_mappings = Vec::new();
        let Some(package_name) = lang.get_package_name(tree, content) else {
            return Ok((symbols, symbol_super_mappings));
        };

        let imports = lang.get_imports(tree, content);

        self.dfs(
            tree.root_node(),
            lang,
            &package_name,
            false,
            &mut symbols,
            path,
            content,
            &package_name,
            &mut symbol_super_mappings,
            &imports,
            is_external,
        )?;

        Ok((symbols, symbol_super_mappings))
    }

    fn dfs(
        &self,
        root: Node,
        lang: &dyn LanguageSupport,
        initial_parent: &str,
        initial_is_type_parent: bool,
        symbols: &mut Vec<Symbol>,
        path: &Path,
        content: &str,
        package_name: &str,
        symbol_super_mappings: &mut Vec<SymbolSuperMapping>,
        imports: &[String],
        is_external: bool,
    ) -> Result<()> {
        let mut stack = vec![(root, initial_parent.to_string(), initial_is_type_parent)];

        while let Some((node, parent_name, is_type_parent)) = stack.pop() {
            let (new_parent, new_is_type_parent) = if lang.should_index(&node, content) {
                let node_type = lang.get_type(&node);
                let modifiers = lang.get_modifiers(&node, content);

                if is_external && !modifiers.contains(&"public".to_string()) {
                    (parent_name.clone(), is_type_parent)
                } else {
                    let short_name = lang
                        .get_short_name(&node, content)
                        .context("Failed to get short name")?;
                    let sep = if is_type_parent { "#" } else { "." };
                    let fqn = format!("{}{}{}", parent_name, sep, short_name);
                    let range = lang.get_range(&node).context("Failed to get range")?;
                    let ident_range = lang
                        .get_ident_range(&node)
                        .context("Failed to get ident range")?;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .context("Failed to get duration")?
                        .as_secs();

                    let implements = lang.get_implements(&node, content);

                    if let Some(superclass_short_name) = lang.get_extends(&node, content) {
                        let superclass_fqn = naive_resolve_fqn(&superclass_short_name, imports);
                        symbol_super_mappings.push(SymbolSuperMapping {
                            id: None,
                            symbol_fqn: fqn.clone(),
                            super_short_name: superclass_short_name,
                            super_fqn: superclass_fqn,
                        });
                    }

                    for interface_short_name in implements {
                        let interface_fqn = naive_resolve_fqn(&interface_short_name, imports);
                        symbol_super_mappings.push(SymbolSuperMapping {
                            id: None,
                            symbol_fqn: fqn.clone(),
                            super_short_name: interface_short_name,
                            super_fqn: interface_fqn,
                        });
                    }

                    let documentation = lang.get_documentation(&node, content);
                    let annotations = lang.get_annotations(&node, content);

                    let mut metadata = SymbolMetadata {
                        annotations: Some(annotations),
                        parameters: None,
                        documentation: documentation,
                        return_type: None,
                    };

                    match node_type {
                        Some(NodeType::Function) => {
                            let symbol_params = lang
                                .get_parameters(&node, content)
                                .context("failed to get function params")?
                                .into_iter()
                                .map(|(name, type_name, default_value)| SymbolParameter {
                                    name,
                                    type_name,
                                    default_value,
                                })
                                .collect();
                            metadata.parameters = Some(symbol_params);
                            metadata.return_type = lang.get_return(&node, content);
                        }
                        Some(NodeType::Field) => {
                            let ret = lang
                                .get_return(&node, content)
                                .context("failed to get function return type")?;
                            metadata.return_type = Some(ret);
                        }
                        _ => (),
                    };

                    symbols.push(Symbol {
                        id: None,
                        vcs_branch: self.vcs.get_current_branch().ok().unwrap(),
                        short_name: short_name,
                        package_name: package_name.to_string(),
                        fully_qualified_name: fqn.clone(),
                        parent_name: Some(parent_name.to_string()),
                        file_path: path.to_string_lossy().to_string(),
                        file_type: lang.get_language().to_string(),
                        symbol_type: node_type.clone().expect("unknown node type").to_string(),
                        modifiers: Json::from(modifiers),
                        line_start: range.start.line as i64,
                        line_end: range.end.line as i64,
                        char_start: range.start.character as i64,
                        char_end: range.end.character as i64,
                        ident_line_start: ident_range.start.line as i64,
                        ident_line_end: ident_range.end.line as i64,
                        ident_char_start: ident_range.start.character as i64,
                        ident_char_end: ident_range.end.character as i64,
                        metadata: Json::from(metadata),
                        last_modified: now as i64,
                    });

                    let is_next_type = matches!(
                        node_type,
                        Some(NodeType::Class | NodeType::Interface | NodeType::Enum)
                    );

                    (fqn, is_next_type)
                }
            } else {
                (parent_name.clone(), is_type_parent)
            };

            // Push children in reverse to maintain left-to-right traversal
            let children: Vec<_> = node.children(&mut node.walk()).collect();
            for child in children.into_iter().rev() {
                stack.push((child, new_parent.clone(), new_is_type_parent));
            }
        }

        Ok(())
    }

    pub async fn index_jar(&self, jar_path: &Path) -> Result<()> {
        let file = File::open(jar_path)?;
        let mut archive = ZipArchive::new(file)?;

        for i in 0..archive.len() {
            let entry_name = {
                let entry = archive.by_index(i)?;
                entry.name().to_string()
            };

            if entry_name.ends_with("module-info.class") {
                continue;
            }

            let path = Path::new(&entry_name).extension().and_then(|s| s.to_str());

            let mut entry = archive.by_index(i)?;
            let mut buffer = Vec::new();
            entry.read_to_end(&mut buffer)?;

            if matches!(path, Some("class")) {
                let symbols = self.extract_class_metadata(&buffer, &entry_name, jar_path)?;
                self.repo.insert_external_symbols(&symbols).await?;
            } else if matches!(path, Some("java" | "groovy" | "kt")) {
                self.index_jar_source_content(
                    Some(String::from_utf8(buffer)?),
                    &entry_name,
                    jar_path,
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn index_jar_source_content(
        &self,
        content: Option<String>,
        entry_name: &str,
        jar_path: &Path,
    ) -> Result<()> {
        // If this is JDK source and not in allow list, skip
        if jar_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == "src.zip")
            .unwrap_or(false)
        {
            if !JAVA_IMPLICIT_IMPORTS
                .iter()
                .any(|prefix| entry_name.contains(&prefix.replace(".", "/").trim_end_matches("*")))
            {
                return Ok(());
            }
        }

        let ext = Path::new(entry_name)
            .extension()
            .and_then(|e| e.to_str())
            .ok_or(anyhow!("No extension"))?;

        if let Some(lang) = self.languages.get(ext) {
            let parsed = lang
                .parse_str(&content.unwrap_or_default())
                .ok_or(anyhow!("Cannot parse content"))?;

            let (external_symbols, mappings) = self.get_external_symbols_from_tree(
                &parsed.0,
                lang.as_ref(),
                jar_path,
                entry_name,
                &parsed.1,
            )?;

            self.repo.insert_external_symbols(&external_symbols).await?;

            if !mappings.is_empty() {
                let mapping_refs = mappings
                    .iter()
                    .map(|m| (&*m.symbol_fqn, &*m.super_short_name, m.super_fqn.as_deref()))
                    .collect();
                self.repo.insert_symbol_super_mappings(mapping_refs).await?;
            }
        }

        Ok(())
    }

    fn get_external_symbols_from_tree(
        &self,
        tree: &Tree,
        lang: &dyn LanguageSupport,
        jar_path: &Path,
        source_file_path: &str,
        content: &str,
    ) -> Result<(Vec<ExternalSymbol>, Vec<SymbolSuperMapping>)> {
        let (symbols, mappings) =
            self.get_symbols_from_tree(tree, lang, jar_path, content, true)?;

        let external_symbols = symbols
            .into_iter()
            .map(|s| ExternalSymbol {
                id: None,
                jar_path: jar_path.to_string_lossy().to_string(),
                source_file_path: source_file_path.to_string(),
                short_name: s.short_name,
                fully_qualified_name: s.fully_qualified_name,
                package_name: s.package_name,
                parent_name: s.parent_name,
                symbol_type: s.symbol_type,
                modifiers: s.modifiers,
                line_start: s.line_start,
                line_end: s.line_end,
                char_start: s.char_start,
                char_end: s.char_end,
                ident_line_start: s.ident_line_start,
                ident_line_end: s.ident_line_end,
                ident_char_start: s.ident_char_start,
                ident_char_end: s.ident_char_end,
                needs_decompilation: false,
                metadata: s.metadata,
                last_modified: s.last_modified,
                file_type: lang.get_language().to_string(),
            })
            .collect();

        Ok((external_symbols, mappings))
    }

    fn extract_class_metadata(
        &self,
        class_bytes: &[u8],
        entry_name: &str,
        jar_path: &Path,
    ) -> Result<Vec<ExternalSymbol>> {
        let class = class_parser(class_bytes)
            .map_err(|e| anyhow!("Failed to parse class: {:?}", e))?
            .1;

        let mut symbols = Vec::new();

        let class_name = get_class_name(&class.const_pool, class.this_class)?.replace('/', ".");
        let package_name = class_name
            .rfind('.')
            .map(|i| &class_name[..i])
            .unwrap_or("");
        let short_name = class_name
            .rfind('.')
            .map(|i| &class_name[i + 1..])
            .unwrap_or(&class_name);

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        // Class symbol
        symbols.push(ExternalSymbol {
            id: None,
            jar_path: jar_path.to_string_lossy().to_string(),
            source_file_path: entry_name.to_string(),
            short_name: short_name.to_string(),
            fully_qualified_name: class_name.clone(),
            package_name: package_name.to_string(),
            parent_name: Some(package_name.to_string()),
            symbol_type: if class.access_flags.contains(ClassAccessFlags::INTERFACE) {
                NodeType::Interface.to_string()
            } else if class.access_flags.contains(ClassAccessFlags::ENUM) {
                NodeType::Enum.to_string()
            } else {
                NodeType::Class.to_string()
            },
            modifiers: Json::from(class_access_to_modifiers(class.access_flags)),
            line_start: 0,
            line_end: 0,
            char_start: 0,
            char_end: 0,
            ident_line_start: 0,
            ident_line_end: 0,
            ident_char_start: 0,
            ident_char_end: 0,
            needs_decompilation: true,
            metadata: Json::from(SymbolMetadata {
                annotations: Some(vec![]),
                parameters: None,
                documentation: None,
                return_type: None,
            }),
            last_modified: now,
            file_type: "java".to_string(),
        });

        // Methods
        for method in &class.methods {
            let method_name = get_utf8(&class.const_pool, method.name_index)?;
            let descriptor = get_utf8(&class.const_pool, method.descriptor_index)?;
            let (params, return_type) = parse_method_descriptor(&descriptor);

            symbols.push(ExternalSymbol {
                id: None,
                jar_path: jar_path.to_string_lossy().to_string(),
                source_file_path: entry_name.to_string(),
                short_name: method_name.clone(),
                fully_qualified_name: format!("{}#{}", class_name, method_name),
                package_name: package_name.to_string(),
                parent_name: Some(class_name.clone()),
                symbol_type: NodeType::Function.to_string(),
                modifiers: Json::from(method_access_to_modifiers(method.access_flags)),
                line_start: 0,
                line_end: 0,
                char_start: 0,
                char_end: 0,
                ident_line_start: 0,
                ident_line_end: 0,
                ident_char_start: 0,
                ident_char_end: 0,
                needs_decompilation: true,
                metadata: Json::from(SymbolMetadata {
                    annotations: None,
                    parameters: Some(params),
                    documentation: None,
                    return_type: Some(return_type),
                }),
                last_modified: now,
                file_type: "java".to_string(),
            });
        }

        // Fields
        for field in &class.fields {
            let field_name = get_utf8(&class.const_pool, field.name_index)?;
            let descriptor = get_utf8(&class.const_pool, field.descriptor_index)?;
            let field_type = parse_field_descriptor(&descriptor);

            symbols.push(ExternalSymbol {
                id: None,
                jar_path: jar_path.to_string_lossy().to_string(),
                source_file_path: entry_name.to_string(),
                short_name: field_name.clone(),
                fully_qualified_name: format!("{}#{}", class_name, field_name),
                package_name: package_name.to_string(),
                parent_name: Some(class_name.clone()),
                symbol_type: NodeType::Field.to_string(),
                modifiers: Json::from(field_access_to_modifiers(field.access_flags)),
                line_start: 0,
                line_end: 0,
                char_start: 0,
                char_end: 0,
                ident_line_start: 0,
                ident_line_end: 0,
                ident_char_start: 0,
                ident_char_end: 0,
                needs_decompilation: true,
                metadata: Json::from(SymbolMetadata {
                    annotations: None,
                    parameters: None,
                    documentation: None,
                    return_type: Some(field_type),
                }),
                last_modified: now,
                file_type: "java".to_string(),
            });
        }

        Ok(symbols)
    }
}

fn get_utf8(pool: &[ConstantInfo], index: u16) -> Result<String> {
    match &pool[(index - 1) as usize] {
        ConstantInfo::Utf8(s) => Ok(s.utf8_string.clone()),
        _ => Err(anyhow!("Not a UTF8 constant")),
    }
}

fn get_class_name(pool: &[ConstantInfo], index: u16) -> Result<String> {
    match &pool[(index - 1) as usize] {
        ConstantInfo::Class(c) => get_utf8(pool, c.name_index),
        _ => Err(anyhow!("Not a Class constant")),
    }
}

fn class_access_to_modifiers(flags: ClassAccessFlags) -> Vec<String> {
    let mut mods = Vec::new();
    if flags.contains(ClassAccessFlags::PUBLIC) {
        mods.push("public".to_string());
    }
    if flags.contains(ClassAccessFlags::FINAL) {
        mods.push("final".to_string());
    }
    if flags.contains(ClassAccessFlags::ABSTRACT) {
        mods.push("abstract".to_string());
    }
    mods
}

fn method_access_to_modifiers(flags: MethodAccessFlags) -> Vec<String> {
    let mut mods = Vec::new();
    if flags.contains(MethodAccessFlags::PUBLIC) {
        mods.push("public".to_string());
    }
    if flags.contains(MethodAccessFlags::PRIVATE) {
        mods.push("private".to_string());
    }
    if flags.contains(MethodAccessFlags::PROTECTED) {
        mods.push("protected".to_string());
    }
    if flags.contains(MethodAccessFlags::STATIC) {
        mods.push("static".to_string());
    }
    if flags.contains(MethodAccessFlags::FINAL) {
        mods.push("final".to_string());
    }
    if flags.contains(MethodAccessFlags::ABSTRACT) {
        mods.push("abstract".to_string());
    }
    mods
}

fn field_access_to_modifiers(flags: FieldAccessFlags) -> Vec<String> {
    let mut mods = Vec::new();
    if flags.contains(FieldAccessFlags::PUBLIC) {
        mods.push("public".to_string());
    }
    if flags.contains(FieldAccessFlags::PRIVATE) {
        mods.push("private".to_string());
    }
    if flags.contains(FieldAccessFlags::PROTECTED) {
        mods.push("protected".to_string());
    }
    if flags.contains(FieldAccessFlags::STATIC) {
        mods.push("static".to_string());
    }
    if flags.contains(FieldAccessFlags::FINAL) {
        mods.push("final".to_string());
    }
    mods
}

fn is_excluded(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| matches!(s, "build" | "target" | ".gradle" | ".git" | "out" | "bin"))
        .unwrap_or(false)
}

fn parse_field_descriptor(descriptor: &str) -> String {
    match descriptor {
        "I" => "int".to_string(),
        "J" => "long".to_string(),
        "D" => "double".to_string(),
        "F" => "float".to_string(),
        "Z" => "boolean".to_string(),
        "B" => "byte".to_string(),
        "C" => "char".to_string(),
        "S" => "short".to_string(),
        "V" => "void".to_string(),
        s if s.starts_with('L') => s[1..].trim_end_matches(';').replace('/', "."),
        s if s.starts_with('[') => format!("{}[]", parse_field_descriptor(&s[1..])),
        s => s.to_string(),
    }
}

fn parse_method_descriptor(descriptor: &str) -> (Vec<SymbolParameter>, String) {
    let (params_str, return_str) = descriptor
        .strip_prefix('(')
        .and_then(|s| s.split_once(')'))
        .unwrap_or(("", descriptor));

    let params = parse_params(params_str)
        .into_iter()
        .enumerate()
        .map(|(i, type_name)| SymbolParameter {
            name: format!("arg{}", i),
            type_name: Some(type_name),
            default_value: None,
        })
        .collect();

    (params, parse_field_descriptor(return_str))
}

fn parse_params(params_str: &str) -> Vec<String> {
    let mut types = Vec::new();
    let mut chars = params_str.chars().peekable();
    while let Some(c) = chars.next() {
        let t = match c {
            'I' => "int".to_string(),
            'J' => "long".to_string(),
            'D' => "double".to_string(),
            'F' => "float".to_string(),
            'Z' => "boolean".to_string(),
            'B' => "byte".to_string(),
            'C' => "char".to_string(),
            'S' => "short".to_string(),
            'L' => {
                let class: String = chars.by_ref().take_while(|&c| c != ';').collect();
                class.replace('/', ".")
            }
            '[' => {
                let inner = parse_params(&chars.by_ref().collect::<String>());
                if let Some(first) = inner.into_iter().next() {
                    types.push(format!("{}[]", first));
                }
                break;
            }
            _ => c.to_string(),
        };
        types.push(t);
    }
    types
}
