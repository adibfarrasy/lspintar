use lsp_core::{
    language_support::{IdentResult, LanguageSupport, ParameterResult, ParseResult},
    languages::Language,
    node_types::NodeType,
    ts_helper::{self, get_node_at_position, node_contains_position},
};
use std::{fs, path::Path, sync::Mutex};

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, Point, Query, QueryCursor, QueryMatch, StreamingIterator, Tree};

use crate::constants::GROOVY_IMPLICIT_IMPORTS;

pub struct GroovySupport {
    parser: Mutex<Parser>,
}

const IDENT_QUERY: &str = r#"
    (expression_statement (identifier) @trivial_case)
    (method_invocation
        object: (_) @method_qualifier
        name: (identifier) @method_name)
    (method_invocation
        object: (this) @this_qualifier
        name: (identifier) @this_method_name)
    (field_access
        object: (_) @field_qualifier
        field: (identifier) @field_name)
    (argument_list (identifier) @arg_name)
    (variable_declarator (identifier) @var_decl)
    [
        (object_creation_expression
            type: (type_identifier) @constructor_type)
        (object_creation_expression
            type: (generic_type (type_identifier) @constructor_type))
        (object_creation_expression
            type: (scoped_type_identifier
                (_) @scoped_constructor_qualifier
                (type_identifier) @scoped_constructor_type))
        (object_creation_expression
            type: (generic_type
                (scoped_type_identifier
                    (_) @scoped_constructor_qualifier
                    (type_identifier) @scoped_constructor_type)))
    ]
    (type_arguments (type_identifier) @type_arg)
    (cast_expression type: (type_identifier) @cast_type)
    (import_declaration
        (scoped_identifier
            name: (identifier) @import_name) @full_import)
    (class_declaration name: (identifier) @class_name)
    (interface_declaration name: (identifier) @interface_name)
    (function_declaration name: (identifier) @function_name)
    (field_declaration (variable_declarator name: (identifier) @field_decl_name))
    (super_interfaces (type_list (type_identifier) @super_interfaces))
    (superclass (type_identifier) @superclass)
"#;

impl GroovySupport {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_groovy::language())
            .unwrap();
        Self {
            parser: Mutex::new(parser),
        }
    }

    fn try_extract_ident_result(
        &self,
        query: &Query,
        match_: &QueryMatch,
        content: &str,
        position: &Position,
        name: &str,
        qual: Option<&str>,
    ) -> Option<IdentResult> {
        let name_idx = query.capture_index_for_name(name);
        let name_cap = match_.captures.iter().find(|c| Some(c.index) == name_idx)?;

        // Check if position is on the name
        if node_contains_position(&name_cap.node, position) {
            let ident = name_cap
                .node
                .utf8_text(content.as_bytes())
                .ok()?
                .to_string();
            let qualifier = if let Some(qual_name) = qual {
                let qual_idx = query.capture_index_for_name(qual_name);
                match_
                    .captures
                    .iter()
                    .find(|c| Some(c.index) == qual_idx)
                    .and_then(|cap| cap.node.utf8_text(content.as_bytes()).ok())
                    .map(|s| {
                        // Remove (...) including contents, then replace . with #
                        let re = regex::Regex::new(r"\([^)]*\)").unwrap();
                        let s = re.replace_all(s, "").replace(".", "#").to_string();
                        s.replace("new ", "")
                    })
            } else {
                None
            };
            return Some((ident, qualifier));
        }

        // Check if position is on the qualifier
        if let Some(qual_name) = qual {
            let qual_idx = query.capture_index_for_name(qual_name);
            if let Some(qual_cap) = match_.captures.iter().find(|c| Some(c.index) == qual_idx) {
                if node_contains_position(&qual_cap.node, position) {
                    if qual_cap.node.kind() == "identifier"
                        || qual_cap.node.kind() == "type_identifier"
                    {
                        let qual_text = qual_cap
                            .node
                            .utf8_text(content.as_bytes())
                            .ok()?
                            .to_string();
                        return Some((qual_text, None));
                    }
                    return self.get_ident_within_node(&qual_cap.node, content, position);
                }
            }
        }

        None
    }

    fn get_ident_within_node(
        &self,
        node: &Node,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        self.find_ident_at_position_impl(*node, content, position)
    }

    fn find_ident_at_position_impl(
        &self,
        root: Node,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        let query = Query::new(&self.get_ts_language(), IDENT_QUERY).ok()?;

        let mut cursor = QueryCursor::new();
        let mut result = None;

        cursor
            .matches(&query, root, content.as_bytes())
            .for_each(|m| {
                if result.is_some() {
                    return;
                }

                vec![
                    ("trivial_case", None),
                    ("method_name", Some("method_qualifier")),
                    ("this_method_name", Some("this_qualifier")),
                    ("field_name", Some("field_qualifier")),
                    ("arg_name", None),
                    ("var_decl", None),
                    ("constructor_type", None),
                    ("type_arg", None),
                    ("cast_type", None),
                    ("class_name", None),
                    ("interface_name", None),
                    ("function_name", None),
                    ("field_decl_name", None),
                    (
                        "scoped_constructor_type",
                        Some("scoped_constructor_qualifier"),
                    ),
                    ("super_interfaces", None),
                    ("superclass", None),
                ]
                .into_iter()
                .for_each(|(name, qual)| {
                    if let Some(r) =
                        self.try_extract_ident_result(&query, &m, content, position, name, qual)
                    {
                        result = Some(r);
                        return;
                    }
                });

                if let Some(r) = self.try_extract_import_ident(&query, &m, content, position) {
                    result = Some(r);
                    return;
                }
            });

        result
    }

    fn try_extract_import_ident(
        &self,
        query: &Query,
        match_: &QueryMatch,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        let name_idx = query.capture_index_for_name("import_name");
        let full_import_idx = query.capture_index_for_name("full_import");

        let name_cap = match_.captures.iter().find(|c| Some(c.index) == name_idx)?;
        let full_import_cap = match_
            .captures
            .iter()
            .find(|c| Some(c.index) == full_import_idx)?;

        let name_node = name_cap.node;
        let full_import_node = full_import_cap.node;

        if !node_contains_position(&name_node, position) {
            return None;
        }

        let full_text = &content[full_import_node.byte_range()];
        let name_text = &content[name_node.byte_range()];
        let scope_text = full_text.strip_suffix(&format!(".{}", name_text))?;

        Some((name_text.to_string(), Some(scope_text.to_string())))
    }

    fn find_in_current_scope(
        &self,
        scope_node: Node,
        content: &str,
        var_name: &str,
        reference_byte: usize,
    ) -> Option<String> {
        let mut cursor = scope_node.walk();

        if !cursor.goto_first_child() {
            return None;
        }

        loop {
            let child = cursor.node();

            // Only process nodes that come before the reference
            if child.start_byte() >= reference_byte {
                break;
            }

            match child.kind() {
                "variable_declaration" | "field_declaration" | "parameter" => {
                    if self.declares_variable(child, content, var_name) {
                        let type_node = child.child_by_field_name("type")?;

                        let position = Position {
                            line: type_node.start_position().row as u32,
                            character: type_node.start_position().column as u32,
                        };
                        return self.get_type_at_position(child, content, &position);
                    }
                }
                "expression_statement"
                | "assignment_expression"
                | "object_creation_expression"
                | "parameters" => {
                    if let Some(var_type) =
                        self.find_in_current_scope(child, content, var_name, reference_byte)
                    {
                        return Some(var_type);
                    }
                }
                _ => {}
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }

        None
    }

    fn declares_variable(&self, node: Node, content: &str, var_name: &str) -> bool {
        let query_str = r#"
        [
            (variable_declarator name: (identifier) @name)
            (parameter name: (identifier) @name)
            (field_declaration (variable_declarator name: (identifier) @name))
        ]
        "#;

        let names = ts_helper::get_many(self.get_ts_language(), &node, content, query_str);
        names.iter().any(|name| name == var_name)
    }

    fn parse_argument_list(&self, arg_list_node: &Node, content: &str) -> Vec<(String, Position)> {
        let mut args = Vec::new();
        let mut cursor = arg_list_node.walk();

        for child in arg_list_node.children(&mut cursor) {
            if child.is_named() {
                if let Ok(arg_text) = child.utf8_text(content.as_bytes()) {
                    let arg_name = arg_text.to_string();
                    let position = Position {
                        line: child.start_position().row as u32,
                        character: child.start_position().column as u32,
                    };
                    args.push((arg_name, position));
                }
            }
        }

        args
    }

    fn extract_method_parameters(&self, method_node: &Node, content: &str) -> Vec<String> {
        let mut cursor = method_node.walk();
        for child in method_node.children(&mut cursor) {
            if child.kind() == "parameters" {
                return self.parse_parameter_list(&child, content);
            }
        }
        vec![]
    }

    fn parse_parameter_list(&self, params_node: &Node, content: &str) -> Vec<String> {
        let mut params = Vec::new();
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter" {
                if let Some(type_node) = child.child_by_field_name("type") {
                    if let Ok(type_name) = type_node.utf8_text(content.as_bytes()) {
                        params.push(type_name.to_string());
                    }
                }
            }
        }
        params
    }
}

impl LanguageSupport for GroovySupport {
    fn get_language(&self) -> Language {
        Language::Groovy
    }

    fn get_ts_language(&self) -> tree_sitter::Language {
        tree_sitter_groovy::language()
    }

    fn parse(&self, file_path: &Path) -> Option<ParseResult> {
        let content = fs::read_to_string(file_path).ok()?;
        self.parse_str(&content)
    }

    fn parse_str(&self, content: &str) -> Option<ParseResult> {
        self.parser
            .try_lock()
            .expect("failed to get parser")
            .parse(content, None)
            .map(|tree| (tree, content.to_string()))
    }

    fn should_index(&self, node: &Node) -> bool {
        self.get_type(node).is_some()
    }

    fn get_range(&self, node: &Node) -> Option<Range> {
        let range = node.range();
        Some(Range {
            start: Position {
                line: range.start_point.row as u32,
                character: range.start_point.column as u32,
            },
            end: Position {
                line: range.end_point.row as u32,
                character: range.end_point.column as u32,
            },
        })
    }

    fn get_ident_range(&self, node: &Node) -> Option<Range> {
        let ident_node = match node.kind() {
            "class_declaration" | "method_declaration" => node.child_by_field_name("name")?,
            "field_declaration" | "constant_declaration" => {
                let declarator = node
                    .children(&mut node.walk())
                    .find(|n| n.kind() == "variable_declarator")?;
                declarator.child_by_field_name("name")?
            }
            _ => node
                .children(&mut node.walk())
                .find(|n| n.kind() == "identifier")?,
        };

        let range = ident_node.range();
        Some(Range {
            start: Position {
                line: range.start_point.row as u32,
                character: range.start_point.column as u32,
            },
            end: Position {
                line: range.end_point.row as u32,
                character: range.end_point.column as u32,
            },
        })
    }

    fn get_package_name(&self, tree: &Tree, content: &str) -> Option<String> {
        let query_str = "(package_declaration (scoped_identifier) @package)";
        ts_helper::get_one(
            self.get_ts_language(),
            &tree.root_node(),
            content,
            query_str,
        )
    }

    fn get_type(&self, node: &Node) -> Option<NodeType> {
        match node.kind() {
            "class_declaration" => Some(NodeType::Class),
            "interface_declaration" => Some(NodeType::Interface),
            "enum_declaration" => Some(NodeType::Enum),
            "function_declaration" => Some(NodeType::Function),
            "field_declaration" => node.parent().and_then(|parent| match parent.kind() {
                "class_body" => Some(NodeType::Field),
                _ => None,
            }),
            "constant_declaration" => Some(NodeType::Field),
            _ => None,
        }
    }

    fn get_short_name(&self, node: &Node, source: &str) -> Option<String> {
        let node_type = self.get_type(node);

        match node_type {
            Some(NodeType::Field) => {
                let query_str = r#"
                    (field_declaration (variable_declarator name: (identifier) @name))
                    (constant_declaration (variable_declarator name: (identifier) @name))
                    "#;
                ts_helper::get_one(self.get_ts_language(), node, source, &query_str)
            }
            Some(_) => {
                let node_kind = node.kind();
                let query_str = format!("({node_kind} name: (identifier) @name)");
                ts_helper::get_one(self.get_ts_language(), node, source, &query_str)
            }
            None => None,
        }
    }

    fn get_extends(&self, node: &Node, source: &str) -> Option<String> {
        let query_str = r#"(superclass (type_identifier) @superclass)"#;
        ts_helper::get_one(self.get_ts_language(), node, source, query_str)
    }

    fn get_implements(&self, node: &Node, source: &str) -> Vec<String> {
        let query_str = r#"(super_interfaces (type_list (type_identifier) @interface))"#;
        ts_helper::get_many(self.get_ts_language(), node, source, query_str)
    }

    fn get_modifiers(&self, node: &Node, source: &str) -> Vec<String> {
        let node_type = self.get_type(node);

        match node_type {
            Some(_) => {
                let node_kind = node.kind();
                let query_str = format!(
                    r#"
                ({node_kind}
                (modifiers
                    [
                        "public"
                        "private"
                        "protected"
                        "static"
                        "final"
                        "abstract"
                        "synchronized"
                        "native"
                        "strictfp"
                        "transient"
                        "volatile"
                    ] @modifier
                ))
                "#
                );
                ts_helper::get_many(self.get_ts_language(), node, source, &query_str)
            }
            None => Vec::new(),
        }
    }

    fn get_annotations(&self, node: &Node, source: &str) -> Vec<String> {
        let node_type = self.get_type(node);

        match node_type {
            Some(_) => {
                let node_kind = node.kind();
                let query_str = format!(
                    r#"
                ({node_kind}
                (modifiers
                    [
                        (marker_annotation name: (identifier) @annotation)
                        (annotation name: (identifier) @annotation)
                    ]
                ))
                "#
                );
                ts_helper::get_many(self.get_ts_language(), node, source, &query_str)
            }
            None => Vec::new(),
        }
    }

    fn get_documentation(&self, node: &Node, source: &str) -> Option<String> {
        let query_str = "(groovydoc_comment) @doc";
        ts_helper::get_one(self.get_ts_language(), node, source, query_str)
    }

    fn get_parameters(&self, node: &Node, source: &str) -> Option<Vec<ParameterResult>> {
        if let Some(NodeType::Function) = self.get_type(node) {
            let query_str = "(function_declaration (parameters (parameter) @arg))";
            let params = ts_helper::get_many(self.get_ts_language(), node, source, query_str)
                .into_iter()
                .map(|p| ts_helper::parse_parameter(&p))
                .collect();
            Some(params)
        } else {
            None
        }
    }

    fn get_return(&self, node: &Node, source: &str) -> Option<String> {
        let node_type = self.get_type(node);

        match node_type {
            Some(NodeType::Field) => {
                let query_str = r#"
                (field_declaration type: (_) @ret)
                (constant_declaration type: (_) @ret)
                "#;
                ts_helper::get_one(self.get_ts_language(), node, source, &query_str)
            }
            Some(NodeType::Function) => {
                let query_str = "(function_declaration (type_identifier) @ret)";
                ts_helper::get_one(self.get_ts_language(), node, source, query_str)
            }
            _ => None,
        }
    }

    fn get_imports(&self, tree: &Tree, source: &str) -> Vec<String> {
        let query_str = "(import_declaration) @doc";
        let explicit_imports =
            ts_helper::get_many(self.get_ts_language(), &tree.root_node(), source, query_str)
                .into_iter()
                .map(|i| i.strip_prefix("import ").unwrap_or_default().to_string())
                .collect::<Vec<String>>();

        GROOVY_IMPLICIT_IMPORTS
            .iter()
            .map(|s| s.to_string())
            .chain(explicit_imports)
            .collect()
    }

    fn get_type_at_position(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<String> {
        let query_text = r#"
        [
          (field_declaration type: (type_identifier) @identifier)
          (field_declaration type: (generic_type) @identifier)
          (variable_declaration type: (type_identifier) @identifier)
          (variable_declaration type: (generic_type) @identifier)
          (parameter type: (type_identifier) @identifier)
          (parameter type: (generic_type) @identifier)
          (interface_declaration name: (identifier) @identifier)
          (class_declaration name: (identifier) @identifier)
          (enum_declaration name: (identifier) @identifier)
        ]
        "#;
        let query = Query::new(&self.get_ts_language(), query_text).ok()?;

        let mut result = None;

        let mut cursor = QueryCursor::new();
        cursor
            .matches(&query, node, content.as_bytes())
            .find(|match_| {
                for capture in match_.captures.iter() {
                    let node = capture.node;
                    if node_contains_position(&node, position) {
                        let ident_name = node
                            .utf8_text(content.as_bytes())
                            .unwrap_or_default()
                            .to_string();

                        result = Some(ident_name);
                    }
                }

                result.is_some()
            });

        result
    }

    fn find_ident_at_position(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult> {
        self.find_ident_at_position_impl(tree.root_node(), content, position)
    }

    fn find_variable_type(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<String> {
        let mut current_node = get_node_at_position(tree, content, position)?;

        if var_name == "this" {
            let mut node = current_node;
            while let Some(parent) = node.parent() {
                if parent.kind() == "class_declaration" {
                    let type_node = parent.child_by_field_name("name")?;
                    let position = Position {
                        line: type_node.start_position().row as u32,
                        character: type_node.start_position().column as u32,
                    };
                    return self
                        .get_ident_within_node(&parent, content, &position)
                        .map(|(name, _)| name);
                }
                node = parent;
            }
            return None;
        }

        // Bubble up through scopes
        let reference_byte = current_node.start_byte();
        loop {
            if let Some(var_type) =
                self.find_in_current_scope(current_node, content, var_name, reference_byte)
            {
                return Some(var_type);
            }

            if let Some(parent) = current_node.parent() {
                current_node = parent;
            } else {
                break;
            }
        }

        None
    }

    fn extract_call_arguments(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<Vec<(String, Position)>> {
        let point = Point::new(position.line as usize, position.character as usize);
        let node = tree.root_node().descendant_for_point_range(point, point)?;

        let mut current = node;
        loop {
            let kind = current.kind();

            if kind == "method_invocation" {
                let mut cursor = current.walk();
                for child in current.children(&mut cursor) {
                    if child.kind() == "argument_list" {
                        return Some(self.parse_argument_list(&child, content));
                    }
                }

                // TODO: handle closures and object method invocation
                return Some(vec![]);
            }

            current = match current.parent() {
                Some(p) => p,
                None => return None,
            };
        }
    }

    fn get_literal_type(&self, tree: &Tree, content: &str, position: &Position) -> Option<String> {
        let point = Point::new(position.line as usize, position.character as usize);
        let mut node = tree.root_node().descendant_for_point_range(point, point)?;

        loop {
            match node.kind() {
                "map_literal" => return Some("Map".to_string()),
                "array_literal" => return Some("List".to_string()),

                // Numeric literals
                "decimal_integer_literal"
                | "hex_integer_literal"
                | "octal_integer_literal"
                | "binary_integer_literal" => {
                    // Check for long suffix (l or L)
                    if let Ok(text) = node.utf8_text(content.as_bytes()) {
                        if text.ends_with('l') || text.ends_with('L') {
                            return Some("Long".to_string());
                        }
                    }
                    return Some("Integer".to_string());
                }

                "decimal_floating_point_literal" | "hex_floating_point_literal" => {
                    if let Ok(text) = node.utf8_text(content.as_bytes()) {
                        let lower = text.to_lowercase();
                        if lower.ends_with('f') {
                            return Some("Float".to_string());
                        }
                    }
                    return Some("Double".to_string());
                }

                "true" | "false" => return Some("Boolean".to_string()),

                "string_literal" | "text_block" => return Some("String".to_string()),

                "null_literal" => return None,

                "regex_literal" => return Some("Pattern".to_string()),

                _ => {}
            }

            node = match node.parent() {
                Some(p) => p,
                None => return None,
            };
        }
    }

    fn get_method_receiver_and_params(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<(String, Vec<String>)> {
        let query_text = r#"
        [
           (class_declaration 
            name: (identifier) @receiver
            body: (class_body (function_declaration) @method))
          (interface_declaration 
            name: (identifier) @receiver
            body: (interface_body (function_declaration) @method))
          (enum_declaration 
            name: (identifier) @receiver
            body: (enum_body (function_declaration) @method))
        ]
        "#;
        let query = Query::new(&self.get_ts_language(), query_text).ok()?;

        let method_idx = query.capture_index_for_name("method");
        let receiver_idx = query.capture_index_for_name("receiver");

        if method_idx.is_none() || receiver_idx.is_none() {
            return None;
        }

        let method_idx = method_idx.unwrap();
        let receiver_idx = receiver_idx.unwrap();
        let mut result = None;
        let mut cursor = QueryCursor::new();
        cursor
            .matches(&query, node, content.as_bytes())
            .find(|match_| {
                let Some(method_capture) = match_.captures.iter().find(|c| c.index == method_idx)
                else {
                    return false;
                };

                if node_contains_position(&method_capture.node, position) {
                    let Some(receiver_capture) =
                        match_.captures.iter().find(|c| c.index == receiver_idx)
                    else {
                        return false;
                    };
                    if let Ok(text) = receiver_capture.node.utf8_text(content.as_bytes()) {
                        let params = self.extract_method_parameters(&method_capture.node, content);
                        result = Some((text.to_string(), params));
                        return true;
                    }
                }
                false
            });

        result
    }
}

#[allow(dead_code)]
mod tests {
    use tower_lsp::lsp_types::Position;
    use tree_sitter::Node;

    mod extract_call_arguments;
    mod find_ident_at_position;
    mod find_variable_type;
    mod get_imports;
    mod get_indexer_data;
    mod get_literal_type;
    mod get_method_receiver_and_params;

    fn find_position(content: &str, marker: &str) -> Position {
        content
            .lines()
            .enumerate()
            .find_map(|(line_num, line)| {
                line.find(marker)
                    .map(|col| Position::new(line_num as u32, col as u32))
            })
            .expect(&format!("Marker '{}' not found", marker))
    }

    fn find_node_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_node_by_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }
}
