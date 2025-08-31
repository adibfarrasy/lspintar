use std::collections::HashMap;
use tower_lsp::lsp_types::{
    Position, Range, TextDocumentContentChangeEvent, VersionedTextDocumentIdentifier,
};
use tree_sitter::Tree;

use crate::languages::LanguageRegistry;

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: String,
    pub content: String,
    pub version: i32,
}

impl Document {
    pub fn new(uri: String, content: String, version: i32, _language_id: String) -> Self {
        Self {
            uri,
            content,
            version,
        }
    }

    pub fn apply_changes(&mut self, changes: Vec<TextDocumentContentChangeEvent>) {
        for change in changes {
            if let Some(range) = change.range {
                // Incremental change
                self.apply_range_change(range, &change.text);
            } else {
                // Full document replacement
                self.content = change.text;
            }
        }
    }

    fn apply_range_change(&mut self, range: Range, new_text: &str) {
        let start_offset = self.position_to_offset(range.start);
        let end_offset = self.position_to_offset(range.end);

        let mut content = self.content.chars().collect::<Vec<_>>();
        content.splice(start_offset..end_offset, new_text.chars());
        self.content = content.into_iter().collect();
    }

    fn position_to_offset(&self, position: Position) -> usize {
        let mut offset = 0;
        let mut current_line = 0;
        let mut current_char = 0;

        for ch in self.content.chars() {
            if current_line == position.line && current_char == position.character {
                break;
            }

            if ch == '\n' {
                current_line += 1;
                current_char = 0;
            } else {
                current_char += 1;
            }
            offset += ch.len_utf8();
        }

        offset
    }

}

pub struct DocumentManager {
    documents: HashMap<String, Document>,
    trees: HashMap<String, Tree>,
}

impl DocumentManager {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            trees: HashMap::new(),
        }
    }

    pub fn insert(&mut self, document: Document) {
        self.documents.insert(document.uri.clone(), document);
    }

    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }

    pub fn remove(&mut self, uri: &str) -> Option<Document> {
        self.documents.remove(uri)
    }

    pub fn update_content(
        &mut self,
        identifier: VersionedTextDocumentIdentifier,
        changes: Vec<TextDocumentContentChangeEvent>,
        language_registry: &LanguageRegistry,
    ) -> Option<&Document> {
        let uri = identifier.uri.to_string();

        let content = {
            if let Some(document) = self.documents.get_mut(&uri) {
                document.version = identifier.version;
                document.apply_changes(changes);
                document.content.clone()
            } else {
                return None;
            }
        };

        self.reparse_and_cache_tree(&uri, &content, language_registry);

        self.documents.get(&uri)
    }

    pub fn reparse_and_cache_tree(
        &mut self,
        uri: &str,
        content: &str,
        language_registry: &LanguageRegistry,
    ) {
        if let Some(language_support) = language_registry.detect_language(uri) {
            let mut parser = language_support.create_parser();
            if let Some(tree) = parser.parse(content, None) {
                self.trees.insert(uri.to_string(), tree);
            } else {
                // Remove cached tree if parsing failed
                self.trees.remove(uri);
            }
        }
    }

    pub fn get_tree(&self, uri: &str) -> Option<&Tree> {
        self.trees.get(uri)
    }
}
