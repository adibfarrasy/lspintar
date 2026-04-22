//! `textDocument/rename` implementation.
//!
//! Rename reuses the same resolver as goto-definition, in reverse: given a
//! declaration resolved at the cursor, every reference whose
//! `resolve_symbol_at_position` lands on that declaration is renamed.  Scope
//! and shadowing are therefore respected by construction.

use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};

use lsp_core::language_support::LanguageSupport;
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::{
        OneOf, Position, Range, RenameParams, TextDocumentIdentifier, TextDocumentPositionParams,
        TextEdit, Url, WorkspaceEdit,
    },
};

use crate::{enums::ResolvedSymbol, models::symbol::Symbol, server::Backend};

impl Backend {
    /// Entry point for `textDocument/rename`.  Returns `Ok(None)` when the
    /// rename is a silent no-op (e.g. target resolves into an external JAR).
    /// Returns `Err` when the new name is invalid for the target language, or
    /// when the cursor is not on a renameable symbol.
    pub async fn rename_impl(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let tdpp = TextDocumentPositionParams {
            text_document: params.text_document_position.text_document.clone(),
            position: params.text_document_position.position,
        };
        let new_name = params.new_name;

        let primary = match self.resolve_symbol_at_position(&tdpp).await {
            Ok(mut syms) if !syms.is_empty() => syms.remove(0),
            _ => {
                // Fall back: the cursor may be on the symbol's own declaration
                // ident range, where goto-definition has nothing to resolve.
                // Try indexed-symbol lookup first (class/field/function
                // declarations), then local-declaration lookup (parameters,
                // for-each bindings, catch clauses) where the current
                // file's tree can answer directly.
                if let Some(sym) = self.find_declaration_at(&tdpp).await? {
                    ResolvedSymbol::Project(sym)
                } else if let Some(local) = self.local_at(&tdpp).await {
                    local
                } else {
                    return Err(Error::invalid_params("nothing to rename at cursor"));
                }
            }
        };

        // Validate the new name against the *language of the target symbol*,
        // falling back to the source file's language.
        let lang = self.language_for_target(&primary, &tdpp)?;
        if !lang.is_valid_identifier(&new_name) {
            return Err(Error::invalid_params(format!(
                "'{new_name}' is not a valid identifier for the target language",
            )));
        }

        match primary {
            ResolvedSymbol::External(_) => Ok(None),
            ResolvedSymbol::Local { uri, position, .. } => {
                self.rename_local(uri, position, &new_name).await
            }
            ResolvedSymbol::Project(sym) => self.rename_project_symbol(sym, &new_name).await,
        }
    }

    /// When the cursor sits on a local/parameter/catch/for-each binding's
    /// declaration identifier, treat it as a local rename seeded at that
    /// position.  Uses per-language `find_local_references` — if the language
    /// returns `Some(_)`, the position is a valid local-declaration site.
    async fn local_at(&self, tdpp: &TextDocumentPositionParams) -> Option<ResolvedSymbol> {
        let path = tdpp.text_document.uri.to_file_path().ok()?;
        let ext = path.extension().and_then(|e| e.to_str())?;
        let lang = self.languages.get(ext)?;
        let (tree, content) = lang.parse(&path)?;
        // Ensure the cursor is on an identifier with matching name.
        let node =
            lsp_core::ts_helper::get_node_at_position(&tree, &content, &tdpp.position)?;
        if node.kind() != "identifier" && node.kind() != "simple_identifier" {
            return None;
        }
        let name = node.utf8_text(content.as_bytes()).ok()?.to_string();
        let refs = lang.find_local_references(&tree, &content, &tdpp.position)?;
        if refs.is_empty() {
            return None;
        }
        Some(ResolvedSymbol::Local {
            name,
            var_type: None,
            uri: tdpp.text_document.uri.clone(),
            position: tdpp.position,
        })
    }

    /// Scan indexed symbols for the one whose ident range contains the
    /// cursor position in the target file.  Used when the cursor sits on a
    /// declaration's own name and `resolve_symbol_at_position` cannot resolve
    /// it as a usage.
    async fn find_declaration_at(
        &self,
        tdpp: &TextDocumentPositionParams,
    ) -> Result<Option<Symbol>> {
        let Some(repo) = self.repo.get() else {
            return Ok(None);
        };
        let path = match tdpp.text_document.uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return Ok(None),
        };
        let path_str = path.to_string_lossy().to_string();
        let symbols = repo
            .find_symbols_by_file_path(&path_str)
            .await
            .unwrap_or_default();
        let pos = tdpp.position;
        for s in symbols {
            if (s.ident_line_start as u32) <= pos.line
                && pos.line <= (s.ident_line_end as u32)
                && (s.ident_char_start as u32) <= pos.character
                && pos.character <= (s.ident_char_end as u32)
            {
                return Ok(Some(s));
            }
        }
        Ok(None)
    }

    fn language_for_target(
        &self,
        symbol: &ResolvedSymbol,
        fallback: &TextDocumentPositionParams,
    ) -> Result<Arc<dyn LanguageSupport + Send + Sync>> {
        let lookup_key = match symbol {
            ResolvedSymbol::Project(s) => file_type_to_extension(&s.file_type),
            ResolvedSymbol::External(s) => file_type_to_extension(&s.file_type),
            ResolvedSymbol::Local { .. } => {
                let path = PathBuf::from_str(fallback.text_document.uri.path())
                    .map_err(|_| Error::invalid_params("bad uri"))?;
                path.extension()
                    .and_then(|e| e.to_str())
                    .ok_or_else(|| Error::invalid_params("no extension"))?
                    .to_string()
            }
        };
        self.languages
            .get(&lookup_key)
            .cloned()
            .ok_or_else(|| Error::invalid_params("unsupported language"))
    }

    // ----------------------------------------------------------------------
    // Project symbol dispatch
    // ----------------------------------------------------------------------

    async fn rename_project_symbol(
        &self,
        target: Symbol,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        match target.symbol_type.as_str() {
            "Class" | "Interface" | "Enum" | "Annotation" => {
                self.rename_type_like(target, new_name).await
            }
            "Function" => self.rename_function_group(target, new_name).await,
            "Field" => self.rename_field_with_accessors(target, new_name).await,
            _ => Err(Error::invalid_params(format!(
                "symbol type '{}' is not renameable",
                target.symbol_type
            ))),
        }
    }

    // ----------------------------------------------------------------------
    // Class / interface / enum / annotation rename
    // ----------------------------------------------------------------------

    async fn rename_type_like(
        &self,
        target: Symbol,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        let short_name = target.short_name.clone();
        let target_fqns: Vec<String> = vec![target.fully_qualified_name.clone()];

        // Include the constructor FQNs (members under this class whose short
        // name equals the class short_name) so that `new Foo(..)` call sites
        // that happen to resolve to a constructor symbol still count.
        let mut extra_fqns: Vec<String> = Vec::new();
        if let Some(repo) = self.repo.get() {
            if let Ok(children) = repo
                .find_symbols_by_parent_name(&target.fully_qualified_name)
                .await
            {
                for c in children {
                    if c.short_name == short_name {
                        extra_fqns.push(c.fully_qualified_name);
                    }
                }
            }
        }

        let mut all_fqns = target_fqns;
        all_fqns.extend(extra_fqns);

        // Always edit the declaration itself first.
        let mut edits_per_file: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        push_decl_edit(&mut edits_per_file, &target, new_name)?;

        self.collect_identity_aware_refs(
            &short_name,
            &all_fqns,
            new_name,
            &[target.file_path.as_str()],
            &mut edits_per_file,
        )
        .await?;

        Ok(Some(workspace_edit_from(edits_per_file)))
    }

    // ----------------------------------------------------------------------
    // Function rename with signature-matched hierarchy walk
    // ----------------------------------------------------------------------

    async fn rename_function_group(
        &self,
        target: Symbol,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        let peers = self.signature_matched_hierarchy(&target).await;
        let short_name = target.short_name.clone();
        let peer_fqns: Vec<String> = peers
            .iter()
            .map(|s| s.fully_qualified_name.clone())
            .collect();
        let decl_file_paths: Vec<&str> = peers.iter().map(|s| s.file_path.as_str()).collect();

        let mut edits_per_file: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for peer in &peers {
            push_decl_edit(&mut edits_per_file, peer, new_name)?;
        }

        self.collect_identity_aware_refs(
            &short_name,
            &peer_fqns,
            new_name,
            &decl_file_paths,
            &mut edits_per_file,
        )
        .await?;

        Ok(Some(workspace_edit_from(edits_per_file)))
    }

    /// Walk the inheritance graph around `seed`, collecting functions that
    /// share its short name, parameter arity and parameter type list.
    async fn signature_matched_hierarchy(&self, seed: &Symbol) -> Vec<Symbol> {
        let Some(repo) = self.repo.get() else {
            return vec![seed.clone()];
        };

        // Collect the set of containing types reachable from the seed's
        // declaring class — walking up to supers and down to implementers.
        let Some(containing_fqn) = seed.parent_name.clone() else {
            return vec![seed.clone()];
        };

        let mut types_to_visit = vec![containing_fqn.clone()];
        let mut visited_types: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut hierarchy_types: Vec<String> = Vec::new();

        while let Some(t) = types_to_visit.pop() {
            if !visited_types.insert(t.clone()) {
                continue;
            }
            hierarchy_types.push(t.clone());

            if let Ok(sups) = repo.find_supers_by_symbol_fqn(&t).await {
                for s in sups {
                    types_to_visit.push(s.fully_qualified_name);
                }
            }
            if let Ok(subs) = repo.find_super_impls_by_fqn(&t).await {
                for s in subs {
                    types_to_visit.push(s.fully_qualified_name);
                }
            }
        }

        // Short-name fallback: when the super FQN could not be resolved at
        // index time (generic parent types, Kotlin `class X : Y<T>`), the
        // forward SuperMapping lookup misses.  Include short-name matches as
        // a best-effort fill-in.
        if let Some(short_name) = hierarchy_types
            .iter()
            .find_map(|fqn| fqn.rsplit('.').next().map(String::from))
        {
            if let Ok(subs) = repo.find_super_impls_by_short_name(&short_name).await {
                for s in subs {
                    if !visited_types.contains(&s.fully_qualified_name) {
                        visited_types.insert(s.fully_qualified_name.clone());
                        hierarchy_types.push(s.fully_qualified_name);
                    }
                }
            }
        }

        let mut peers: Vec<Symbol> = Vec::new();
        for t in &hierarchy_types {
            if let Ok(children) = repo.find_symbols_by_parent_name(t).await {
                for c in children {
                    if c.symbol_type == "Function"
                        && c.short_name == seed.short_name
                        && signatures_match(seed, &c)
                    {
                        peers.push(c);
                    }
                }
            }
        }

        if peers.is_empty() {
            peers.push(seed.clone());
        }
        peers
    }

    // ----------------------------------------------------------------------
    // Field rename with property-accessor coupling
    // ----------------------------------------------------------------------

    async fn rename_field_with_accessors(
        &self,
        target: Symbol,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        let short_name = target.short_name.clone();
        let mut fqns: Vec<String> = vec![target.fully_qualified_name.clone()];

        // Locate accessor siblings (Groovy `getFoo`/`setFoo`/`isFoo`, or
        // Kotlin JVM-visible accessors).  Only collected for JVM languages
        // that surface them as Function symbols under the same parent.
        let mut accessor_syms: Vec<(Symbol, AccessorKind)> = Vec::new();
        if let Some(parent_fqn) = target.parent_name.clone() {
            if let Some(repo) = self.repo.get() {
                if let Ok(siblings) = repo.find_symbols_by_parent_name(&parent_fqn).await {
                    for s in siblings {
                        if s.symbol_type != "Function" {
                            continue;
                        }
                        if let Some(kind) = accessor_kind_for(&s.short_name, &short_name) {
                            accessor_syms.push((s, kind));
                        }
                    }
                }
            }
        }

        for (s, _) in &accessor_syms {
            fqns.push(s.fully_qualified_name.clone());
        }

        let mut edits_per_file: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        push_decl_edit(&mut edits_per_file, &target, new_name)?;
        for (s, kind) in &accessor_syms {
            let new_accessor = rename_accessor_text(kind, new_name);
            push_decl_edit(&mut edits_per_file, s, &new_accessor)?;
        }

        // Identity-aware reference sweep, using the field's short name as
        // filter, covering accessor call sites separately.
        let mut decl_paths: Vec<&str> = vec![target.file_path.as_str()];
        for (s, _) in &accessor_syms {
            decl_paths.push(s.file_path.as_str());
        }

        self.collect_identity_aware_refs(
            &short_name,
            &fqns,
            new_name,
            &decl_paths,
            &mut edits_per_file,
        )
        .await?;

        // Accessor call sites have a *different* identifier text than the
        // field — sweep for each accessor by its own short name.
        for (s, kind) in &accessor_syms {
            let new_accessor = rename_accessor_text(kind, new_name);
            self.collect_identity_aware_refs(
                &s.short_name,
                std::slice::from_ref(&s.fully_qualified_name),
                &new_accessor,
                &decl_paths,
                &mut edits_per_file,
            )
            .await?;
        }

        Ok(Some(workspace_edit_from(edits_per_file)))
    }

    // ----------------------------------------------------------------------
    // Local variable rename (single-file, scope-aware)
    // ----------------------------------------------------------------------

    async fn rename_local(
        &self,
        uri: Url,
        decl_position: Position,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        let path = PathBuf::from_str(uri.path())
            .map_err(|_| Error::invalid_params("bad uri".to_string()))?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| Error::invalid_params("no extension"))?;
        let lang = self
            .languages
            .get(ext)
            .ok_or_else(|| Error::invalid_params("unsupported language"))?;

        let (tree, content) = lang
            .parse(&path)
            .ok_or_else(|| Error::invalid_params("parse failed"))?;

        let ranges = lang
            .find_local_references(&tree, &content, &decl_position)
            .ok_or_else(|| Error::invalid_params("no local declaration found"))?;
        if ranges.is_empty() {
            return Ok(None);
        }

        let edits: Vec<TextEdit> = ranges
            .into_iter()
            .map(|range| TextEdit {
                range,
                new_text: new_name.to_string(),
            })
            .collect();

        let mut edits_per_file: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        edits_per_file.insert(uri, edits);
        Ok(Some(workspace_edit_from(edits_per_file)))
    }

    // ----------------------------------------------------------------------
    // Identity-aware reference sweep
    // ----------------------------------------------------------------------

    /// For every project source file, find text occurrences of `short_name`,
    /// verify each resolves to one of `target_fqns`, and append an edit
    /// replacing the occurrence with `new_text`.
    ///
    /// `decl_file_paths` lists source files whose declaration identifier
    /// ranges are *already* in `edits_per_file`; occurrences at those ranges
    /// are suppressed to avoid duplicates.
    async fn collect_identity_aware_refs(
        &self,
        short_name: &str,
        target_fqns: &[String],
        new_text: &str,
        decl_file_paths: &[&str],
        edits_per_file: &mut HashMap<Url, Vec<TextEdit>>,
    ) -> Result<()> {
        let Some(repo) = self.repo.get() else {
            return Ok(());
        };
        let file_paths = repo.find_all_source_file_paths().await.unwrap_or_default();

        for file_path in file_paths {
            let fp = PathBuf::from(&file_path);
            let ext = match fp.extension().and_then(|e| e.to_str()) {
                Some(e) => e.to_string(),
                None => continue,
            };
            let Some(file_lang) = self.languages.get(&ext) else {
                continue;
            };
            let content = match std::fs::read_to_string(&fp) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let Ok(uri) = Url::from_file_path(&fp) else {
                continue;
            };

            let candidates = word_boundary_occurrences(&content, short_name);
            if candidates.is_empty() {
                continue;
            }

            let tree = match file_lang.parse_str(&content) {
                Some((t, _)) => t,
                None => continue,
            };

            for (line, column, end_column) in candidates {
                let position = Position {
                    line: line as u32,
                    character: column as u32,
                };
                // Skip the declaration sites we've already edited.
                if decl_file_paths.contains(&file_path.as_str())
                    && declaration_already_covered(
                        edits_per_file.get(&uri),
                        position,
                        end_column as u32,
                    )
                {
                    continue;
                }
                if position_in_comment_or_string(&tree, line, column) {
                    continue;
                }

                // Identity check: does this occurrence resolve to any of the
                // target FQNs?
                let tdpp = TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    position,
                };
                let resolved = match self.resolve_symbol_at_position(&tdpp).await {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let matches = resolved.iter().any(|r| match r {
                    ResolvedSymbol::Project(s) => {
                        target_fqns.iter().any(|f| f == &s.fully_qualified_name)
                    }
                    _ => false,
                });
                if !matches {
                    continue;
                }

                let range = Range {
                    start: position,
                    end: Position {
                        line: line as u32,
                        character: end_column as u32,
                    },
                };
                edits_per_file.entry(uri.clone()).or_default().push(TextEdit {
                    range,
                    new_text: new_text.to_string(),
                });
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------
// Helpers — accessor coupling
// --------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum AccessorKind {
    Getter, // getFoo
    Setter, // setFoo
    IsGetter, // isFoo (boolean properties)
}

/// When `accessor_name` is a getter/setter/is-getter for `field_name`, return
/// which.  Capitalisation follows the JavaBeans convention.
fn accessor_kind_for(accessor_name: &str, field_name: &str) -> Option<AccessorKind> {
    let cap = capitalize_first(field_name);
    if accessor_name == format!("get{}", cap) {
        Some(AccessorKind::Getter)
    } else if accessor_name == format!("set{}", cap) {
        Some(AccessorKind::Setter)
    } else if accessor_name == format!("is{}", cap) {
        Some(AccessorKind::IsGetter)
    } else {
        None
    }
}

fn rename_accessor_text(kind: &AccessorKind, new_field: &str) -> String {
    let cap = capitalize_first(new_field);
    match kind {
        AccessorKind::Getter => format!("get{}", cap),
        AccessorKind::Setter => format!("set{}", cap),
        AccessorKind::IsGetter => format!("is{}", cap),
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

// --------------------------------------------------------------------------
// Helpers — text scanning and comment/string filtering
// --------------------------------------------------------------------------

/// Returns (line_idx, start_col, end_col) for each ASCII-word-boundary
/// occurrence of `needle` in `content`.  Columns are 0-based UTF-8 byte
/// offsets within the line.
fn word_boundary_occurrences(content: &str, needle: &str) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    for (line_idx, line) in content.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while let Some(pos) = line[i..].find(needle) {
            let abs = i + pos;
            let before_ok = abs == 0 || !is_ident(bytes[abs - 1]);
            let after = abs + needle.len();
            let after_ok = after >= bytes.len() || !is_ident(bytes[after]);
            if before_ok && after_ok {
                out.push((line_idx, abs, after));
            }
            i = abs + 1;
            if i >= bytes.len() {
                break;
            }
        }
    }
    out
}

/// Returns true when the byte offset `(line, col)` falls inside a comment or
/// string literal node in `tree`.
fn position_in_comment_or_string(tree: &tree_sitter::Tree, line: usize, col: usize) -> bool {
    let pt = tree_sitter::Point { row: line, column: col };
    let Some(mut node) = tree.root_node().descendant_for_point_range(pt, pt) else {
        return false;
    };
    loop {
        let kind = node.kind();
        if kind.contains("comment")
            || kind.contains("string_literal")
            || kind.contains("string_content")
            || kind == "line_string_literal"
            || kind == "multi_line_string_literal"
            || kind == "character_literal"
        {
            return true;
        }
        match node.parent() {
            Some(p) => node = p,
            None => return false,
        }
    }
}

fn declaration_already_covered(
    edits: Option<&Vec<TextEdit>>,
    start: Position,
    end_char: u32,
) -> bool {
    let Some(edits) = edits else { return false };
    edits.iter().any(|e| {
        e.range.start.line == start.line
            && e.range.start.character == start.character
            && e.range.end.line == start.line
            && e.range.end.character == end_char
    })
}

// --------------------------------------------------------------------------
// Helpers — signature equality
// --------------------------------------------------------------------------

/// Two functions share a signature when they have the same parameter arity
/// and the same parameter types in order (varargs `T...` and array `T[]`
/// are treated as equivalent).
fn signatures_match(a: &Symbol, b: &Symbol) -> bool {
    let pa = a.metadata.parameters.as_ref();
    let pb = b.metadata.parameters.as_ref();
    match (pa, pb) {
        (Some(pa), Some(pb)) => {
            if pa.len() != pb.len() {
                return false;
            }
            pa.iter()
                .zip(pb.iter())
                .all(|(x, y)| normalise_type(x.type_name.as_deref()) == normalise_type(y.type_name.as_deref()))
        }
        (None, None) => true,
        _ => false,
    }
}

fn normalise_type(t: Option<&str>) -> Option<String> {
    t.map(|s| {
        let trimmed = s.trim();
        // Treat `T...` (Java varargs) and `T[]` as the same.
        if let Some(prefix) = trimmed.strip_suffix("...") {
            return format!("{}[]", prefix.trim_end());
        }
        trimmed.to_string()
    })
}

// --------------------------------------------------------------------------
// Helpers — workspace-edit construction
// --------------------------------------------------------------------------

fn push_decl_edit(
    edits_per_file: &mut HashMap<Url, Vec<TextEdit>>,
    sym: &Symbol,
    new_name: &str,
) -> Result<()> {
    let uri = Url::from_file_path(&sym.file_path)
        .map_err(|_| Error::invalid_params(format!("bad file path: {}", sym.file_path)))?;
    let range = Range {
        start: Position {
            line: sym.ident_line_start as u32,
            character: sym.ident_char_start as u32,
        },
        end: Position {
            line: sym.ident_line_end as u32,
            character: sym.ident_char_end as u32,
        },
    };
    edits_per_file.entry(uri).or_default().push(TextEdit {
        range,
        new_text: new_name.to_string(),
    });
    Ok(())
}

/// The indexer stores `file_type` as the language name ("java", "groovy",
/// "kotlin"); the `languages` map is keyed by file extension.  Translate.
fn file_type_to_extension(file_type: &str) -> String {
    match file_type {
        "kotlin" => "kt".to_string(),
        other => other.to_string(),
    }
}

fn workspace_edit_from(edits_per_file: HashMap<Url, Vec<TextEdit>>) -> WorkspaceEdit {
    // Dedupe edits per file: same (start, end) multiple times.
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for (uri, mut list) in edits_per_file {
        list.sort_by(|a, b| {
            a.range
                .start
                .line
                .cmp(&b.range.start.line)
                .then(a.range.start.character.cmp(&b.range.start.character))
                .then(a.range.end.line.cmp(&b.range.end.line))
                .then(a.range.end.character.cmp(&b.range.end.character))
        });
        list.dedup_by(|a, b| a.range == b.range && a.new_text == b.new_text);
        changes.insert(uri, list);
    }
    WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }
}

// Silence the unused-import warning on OneOf when rename_provider uses
// RenameProviderCapability::Simple instead.
#[allow(dead_code)]
fn _unused_onef(_: OneOf<bool, ()>) {}
