use std::{collections::HashSet, path::Path};

use tower_lsp::lsp_types::{Diagnostic, Position, Range};
use tree_sitter::{Node, Tree};

use crate::{languages::Language, node_kind::NodeKind};

pub type ParseResult = (Tree, String);

// (name, qualifier)
pub type IdentResult = (String, Option<String>);

// (name, type_name, default_value)
pub type ParameterResult = (String, Option<String>, Option<String>);

pub trait LanguageSupport: Send + Sync {
    fn get_language(&self) -> Language;
    fn get_ts_language(&self) -> tree_sitter::Language;
    fn parse(&self, file_path: &Path) -> Option<ParseResult>;
    fn parse_str(&self, source: &str) -> Option<ParseResult>;

    fn should_index(&self, node: &Node, _source: &str) -> bool {
        self.get_kind(node).is_some()
    }

    fn get_range(&self, node: &Node) -> Option<Range>;
    fn get_ident_range(&self, node: &Node) -> Option<Range>;

    /*
     * Identifier
     */
    fn get_package_name(&self, tree: &Tree, source: &str) -> Option<String>;
    fn get_kind(&self, node: &Node) -> Option<NodeKind>;
    fn get_short_name(&self, node: &Node, source: &str) -> Option<String>;

    /*
     * Hierarchy
     */
    fn get_extends(&self, node: &Node, source: &str) -> Option<String>;
    fn get_implements(&self, node: &Node, source: &str) -> Vec<String>;

    /*
     * Metadata
     */
    fn get_modifiers(&self, node: &Node, source: &str) -> Vec<String>;
    fn get_annotations(&self, node: &Node, source: &str) -> Vec<String>;
    fn get_documentation(&self, node: &Node, source: &str) -> Option<String>;
    fn get_parameters(&self, node: &Node, source: &str) -> Option<Vec<ParameterResult>>;
    fn get_return(&self, node: &Node, source: &str) -> Option<String>;

    // should also return implicit imports
    fn get_imports(&self, tree: &Tree, source: &str) -> Vec<String>;

    fn get_implicit_imports(&self) -> Vec<String>;

    fn get_type_at_position(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<String>;

    fn find_ident_at_position(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<IdentResult>;

    fn find_variable_type(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<String>;

    fn find_variable_declaration(
        &self,
        tree: &Tree,
        content: &str,
        var_name: &str,
        position: &Position,
    ) -> Option<(Option<String>, Position)>; // (type, position)

    fn find_declarations_in_scope(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Vec<(String, Option<String>)>; // (var_name, type_name)

    fn extract_call_arguments(
        &self,
        tree: &Tree,
        content: &str,
        position: &Position,
    ) -> Option<Vec<(String, Position)>>;

    fn get_literal_type(&self, tree: &Tree, content: &str, position: &Position) -> Option<String>;

    fn get_method_receiver_and_params(
        &self,
        node: Node,
        content: &str,
        position: &Position,
    ) -> Option<(String, Vec<String>)>;

    fn collect_diagnostics(&self, tree: &Tree, source: &str) -> Vec<Diagnostic>;

    /// Returns all type name references (usage sites, not declarations) with their ranges.
    /// Used by the server to check for unresolved symbols against the index.
    /// Default returns empty — languages implement this to opt in.
    fn get_type_references(&self, _tree: &Tree, _source: &str) -> Vec<(String, Range)> {
        vec![]
    }

    /// Returns the short names of all types declared in this file (classes, interfaces, enums).
    /// Used to skip locally declared types when checking for unresolved symbols.
    fn get_declared_type_names(&self, _tree: &Tree, _source: &str) -> Vec<String> {
        vec![]
    }

    /// Returns class declarations in this file with enough data to check for unimplemented
    /// abstract methods: name, location of the class keyword, whether it's abstract,
    /// direct parents (extends + implements), and the set of method names it defines.
    fn get_class_declarations(&self, _tree: &Tree, _source: &str) -> Vec<ClassDeclarationData> {
        vec![]
    }

    /// Returns all `new T(...)` expressions in the file.
    /// Used to check whether a directly instantiated type is abstract.
    /// Default returns empty — languages without an explicit `new` keyword (e.g. Kotlin) skip this.
    fn get_object_creations(&self, _tree: &Tree, _source: &str) -> Vec<ObjectCreationData> {
        vec![]
    }

    /// Returns all qualified member-access call sites of the form `receiver.method(...)`.
    /// Only includes sites where the receiver is a simple identifier.
    /// Used to detect method_not_found, inaccessible_member, and static_member_via_instance.
    /// Default returns empty — languages opt in by implementing this.
    fn get_member_accesses(&self, _tree: &Tree, _source: &str) -> Vec<MemberAccessData> {
        vec![]
    }

    /// Returns all generic type usages with their type argument counts.
    /// E.g. `List<String>` → `("List", 1, range)`, `Map<K,V>` → `("Map", 2, range)`.
    /// Used to detect wrong_type_argument_count.
    fn get_generic_type_usages(&self, _tree: &Tree, _source: &str) -> Vec<GenericTypeUsage> {
        vec![]
    }

    /// Returns all methods that override a parent method, with their declared return type and
    /// the short name of the containing class.
    /// Java/Groovy: methods with `@Override` annotation.
    /// Kotlin: functions with the `override` modifier.
    fn get_override_methods(&self, _tree: &Tree, _source: &str) -> Vec<OverrideMethodData> {
        vec![]
    }

    /// Returns variable declarations where a numeric primitive is initialised from an identifier,
    /// so the server can check whether that identifier has a wider numeric type (narrowing_conversion).
    /// Only Java and Groovy implement this; Kotlin outlaws implicit numeric conversions at the
    /// language level so there is nothing to check.
    fn get_narrowing_candidates(&self, _tree: &Tree, _source: &str) -> Vec<NarrowingCandidateData> {
        vec![]
    }

    /// Returns all method call sites where the receiver is a simple identifier.
    /// Used to detect wrong_argument_types.
    /// Java/Groovy/Kotlin all implement this.
    fn get_method_call_sites(&self, _tree: &Tree, _source: &str) -> Vec<MethodCallSiteData> {
        vec![]
    }

    /// Returns true when `name` is a syntactically valid identifier in this language
    /// and is not a reserved keyword.  Default checks ASCII rules
    /// (letter or `_`/`$` followed by letters, digits, `_`, `$`) and delegates
    /// keyword filtering to `reserved_keywords`.
    fn is_valid_identifier(&self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        let mut chars = name.chars();
        let first = chars.next().unwrap();
        let is_ident_start = |c: char| c.is_ascii_alphabetic() || c == '_' || c == '$';
        let is_ident_cont = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
        if !is_ident_start(first) {
            return false;
        }
        if !chars.all(is_ident_cont) {
            return false;
        }
        !self.reserved_keywords().contains(name)
    }

    /// Reserved keywords for this language.  Used by `is_valid_identifier`.
    fn reserved_keywords(&self) -> &'static HashSet<&'static str>;

    /// Given the declaration position of a local variable or parameter, return
    /// the ranges of all identifier occurrences in the file that resolve to
    /// that declaration.  The result includes the declaration's own identifier
    /// range.  References that resolve to a shadowing inner declaration are
    /// excluded by construction.
    ///
    /// Returns `None` when the language does not distinguish locals from
    /// fields at this position, or when the declaration cannot be located.
    fn find_local_references(
        &self,
        _tree: &Tree,
        _content: &str,
        _decl_position: &Position,
    ) -> Option<Vec<Range>> {
        None
    }
}

/// One argument at a method call site, with enough information for the server to
/// determine or infer its type.
pub struct CallArgData {
    /// The tree-sitter node kind of the argument expression, e.g. `"decimal_integer_literal"`,
    /// `"string_literal"`, `"identifier"`.  For complex expressions the kind will be something
    /// else (e.g. `"method_invocation"`) and the server will skip type checking for that arg.
    pub node_kind: String,
    /// The text of the argument as written in source.
    pub text: String,
    /// Source range of the argument — used to look up variable types when `node_kind` is
    /// `"identifier"`.
    pub range: Range,
}

/// A method call site with argument information.
pub struct MethodCallSiteData {
    /// The receiver's identifier text (e.g. `"foo"` for `foo.bar(...)`).
    pub receiver_name: String,
    /// Range of the receiver — used to resolve its declared type.
    pub receiver_range: Range,
    /// The method name being called.
    pub method_name: String,
    /// Range of the method name identifier — where diagnostics are anchored.
    pub method_range: Range,
    /// The arguments to the call, one entry per positional argument.
    pub args: Vec<CallArgData>,
}

/// Data about a class (or enum) declaration extracted from a source file.
pub struct ClassDeclarationData {
    pub name: String,
    /// Range of the class name identifier — where diagnostics are anchored.
    pub ident_range: Range,
    /// True when the class itself is declared abstract (it is allowed to leave methods unimplemented).
    pub is_abstract: bool,
    /// Direct parent names as written in source: the extends type and all implements types.
    pub parents: Vec<String>,
    /// Short names of all methods defined directly in this class body (not inherited).
    pub defined_method_names: HashSet<String>,
}

/// A `new T(...)` expression site.
pub struct ObjectCreationData {
    /// The short type name as written in source, e.g. `"ArrayList"`.
    pub type_name: String,
    /// Range of the type name identifier — where the diagnostic is anchored.
    pub range: Range,
}

/// A qualified member-access call `receiver.method(...)` where the receiver is a simple identifier.
pub struct MemberAccessData {
    /// The receiver's identifier text as written in source (e.g. `"foo"` for `foo.bar()`).
    pub receiver_name: String,
    /// The method/field name being accessed.
    pub member_name: String,
    /// Range of the member name identifier — where diagnostics are anchored.
    pub member_range: Range,
    /// Range of the receiver expression — used to look up its declared type.
    pub receiver_range: Range,
}

/// A generic type usage site, e.g. `List<String>` or `Map<K, V>`.
pub struct GenericTypeUsage {
    /// The base type name as written in source, e.g. `"List"`.
    pub type_name: String,
    /// Number of type arguments supplied at this site.
    pub arg_count: usize,
    /// Range of the full generic type expression — where diagnostics are anchored.
    pub range: Range,
}

/// A method that explicitly overrides a parent method.
pub struct OverrideMethodData {
    /// Short name of the class that directly declares this override.
    pub containing_class: String,
    /// Name of the overriding method.
    pub method_name: String,
    /// Declared return type as written in source (e.g. `"String"`, `"List<Int>"`).
    /// `None` when there is no explicit return type (void / Unit).
    pub return_type: Option<String>,
    /// Range of the method name identifier — where diagnostics are anchored.
    pub range: Range,
}

/// A variable declaration where a numeric primitive is assigned from a simple identifier,
/// allowing the server to check for narrowing conversions.
pub struct NarrowingCandidateData {
    /// The declared numeric type (e.g. `"int"`, `"float"`).
    pub declared_type: String,
    /// The identifier on the right-hand side (the variable being read).
    pub rhs_name: String,
    /// Range of the RHS identifier — where diagnostics are anchored.
    pub range: Range,
}
