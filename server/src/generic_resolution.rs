/// Generic type resolution: JVM Signature attribute parsing, type-arg string
/// parsing, binding construction, and type-variable substitution.
///
/// This implements the core of `InferChainStepType` from type-inference.allium:
/// given a receiver type like `List<String>` and a method whose Signature says
/// it returns `E`, produce `String` by binding E→String and substituting.
use std::collections::HashMap;

use classfile_parser::{attribute_info::AttributeInfo, constant_info::ConstantInfo};

// ---------------------------------------------------------------------------
// JVM Signature attribute helpers
// ---------------------------------------------------------------------------

/// Reads the raw "Signature" attribute string from a list of class-file
/// attributes and the corresponding constant pool.
pub fn read_signature_attr(
    attributes: &[AttributeInfo],
    pool: &[ConstantInfo],
) -> Option<String> {
    for attr in attributes {
        let name_idx = attr.attribute_name_index as usize;
        if name_idx == 0 || name_idx > pool.len() {
            continue;
        }
        if let ConstantInfo::Utf8(u) = &pool[name_idx - 1] {
            if u.utf8_string == "Signature" && attr.info.len() >= 2 {
                let sig_idx = u16::from_be_bytes([attr.info[0], attr.info[1]]) as usize;
                if sig_idx > 0 && sig_idx <= pool.len() {
                    if let ConstantInfo::Utf8(s) = &pool[sig_idx - 1] {
                        return Some(s.utf8_string.clone());
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Parsing class-level type parameter names from a JVM class signature
// ---------------------------------------------------------------------------

/// Extracts the ordered list of type parameter names from a JVM class
/// signature string (the value of the `Signature` attribute on a class).
///
/// Example: `"<E:Ljava/lang/Object;>Ljava/util/AbstractList<TE;>;"` → `["E"]`
/// Example: `"<K:Ljava/lang/Object;V:Ljava/lang/Object;>..."` → `["K", "V"]`
pub fn parse_class_type_params(class_sig: &str) -> Vec<String> {
    if !class_sig.starts_with('<') {
        return vec![];
    }
    let bytes = class_sig.as_bytes();
    let mut params = Vec::new();
    let mut i = 1; // skip '<'

    while i < bytes.len() && bytes[i] != b'>' {
        // Read the type parameter name (everything up to ':')
        let start = i;
        while i < bytes.len() && bytes[i] != b':' && bytes[i] != b'>' {
            i += 1;
        }
        let name = class_sig[start..i].to_string();
        if name.is_empty() {
            break;
        }
        params.push(name);

        // Skip class bound: ':'  followed by a (possibly absent) ClassTypeSignature
        // Skip interface bounds: ':' followed by a ClassTypeSignature, repeated
        while i < bytes.len() && bytes[i] == b':' {
            i += 1; // skip ':'
            if i < bytes.len() && bytes[i] != b':' && bytes[i] != b'>' {
                i = skip_type_sig(bytes, i);
            }
        }
    }

    params
}

// ---------------------------------------------------------------------------
// Parsing method generic return type from a JVM method signature
// ---------------------------------------------------------------------------

/// Extracts the ordered list of method-level type parameter names from a JVM
/// method signature string (the value of the `Signature` attribute on a method).
///
/// Example: `"<R:Ljava/lang/Object;>(...)TR;"` → `["R"]`
/// Example: `"<K:Ljava/lang/Object;V:Ljava/lang/Object;>(...)..."` → `["K", "V"]`
/// Example: `"(I)TE;"` → `[]`  (no method-level type params)
pub fn parse_method_type_params(method_sig: &str) -> Vec<String> {
    // Method-level formal type parameters share the same '<…>' prefix format as
    // class signatures, so parse_class_type_params handles them correctly.
    parse_class_type_params(method_sig)
}

/// Extracts the return type from a JVM method signature string, preserving
/// generic type variable names (e.g. `E`, `List<E>`).
///
/// Example: `"(I)TE;"` → `Some("E")`
/// Example: `"()Ljava/util/stream/Stream<TE;>;"` → `Some("Stream<E>")`
pub fn parse_method_generic_return(method_sig: &str) -> Option<String> {
    let sig = if method_sig.starts_with('<') {
        skip_formal_type_params(method_sig)
    } else {
        method_sig
    };

    // Skip '(' ... ')'
    let sig = sig.strip_prefix('(')?;
    let bytes = sig.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b')' {
        i = skip_type_sig(bytes, i);
    }
    if i >= bytes.len() {
        return None;
    }
    i += 1; // skip ')'

    parse_type_sig_to_string(&sig[i..])
}

// ---------------------------------------------------------------------------
// Parsing method generic parameter types from a JVM method signature
// ---------------------------------------------------------------------------

/// Extracts the ordered list of parameter types from a JVM method signature,
/// preserving generic type variable names (e.g. `Consumer<T>`, `Function1<T, Unit>`).
///
/// Example: `"(Ljava/util/function/Consumer<TT;>;)V"` → `["Consumer<T>"]`
/// Example: `"(ILjava/util/function/Function<TE;TR;>;)TR;"` → `["int", "Function<E, R>"]`
pub fn parse_method_generic_params(method_sig: &str) -> Vec<String> {
    let sig = if method_sig.starts_with('<') {
        skip_formal_type_params(method_sig)
    } else {
        method_sig
    };

    let Some(sig) = sig.strip_prefix('(') else {
        return vec![];
    };
    let bytes = sig.as_bytes();
    let mut i = 0;
    let mut params = Vec::new();

    while i < bytes.len() && bytes[i] != b')' {
        let start = i;
        i = skip_type_sig(bytes, i);
        if let Some(t) = parse_type_sig_to_string(&sig[start..]) {
            params.push(t);
        }
    }

    params
}

// ---------------------------------------------------------------------------
// Internal JVM signature traversal helpers
// ---------------------------------------------------------------------------

fn skip_formal_type_params(sig: &str) -> &str {
    let bytes = sig.as_bytes();
    let mut i = 1; // skip '<'
    let mut depth = 1i32;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    &sig[i..]
}

fn skip_type_sig(bytes: &[u8], i: usize) -> usize {
    if i >= bytes.len() {
        return i;
    }
    match bytes[i] {
        b'L' => skip_class_type_sig(bytes, i),
        b'T' => {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b';' {
                j += 1;
            }
            j + 1 // skip the ';'
        }
        b'[' => skip_type_sig(bytes, i + 1),
        _ => i + 1, // primitive or 'V'
    }
}

fn skip_class_type_sig(bytes: &[u8], mut i: usize) -> usize {
    i += 1; // skip 'L'
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                depth += 1;
                i += 1;
            }
            b'>' => {
                depth -= 1;
                i += 1;
            }
            b';' if depth == 0 => {
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    i
}

fn parse_type_sig_to_string(sig: &str) -> Option<String> {
    let bytes = sig.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    match bytes[0] {
        b'V' => Some("void".to_string()),
        b'I' => Some("int".to_string()),
        b'J' => Some("long".to_string()),
        b'D' => Some("double".to_string()),
        b'F' => Some("float".to_string()),
        b'Z' => Some("boolean".to_string()),
        b'B' => Some("byte".to_string()),
        b'C' => Some("char".to_string()),
        b'S' => Some("short".to_string()),
        b'T' => {
            // Type variable: T<name>;
            let end = sig[1..].find(';')?;
            Some(sig[1..end + 1].to_string())
        }
        b'L' => parse_class_type_sig_to_string(&sig[1..]),
        b'[' => {
            let inner = parse_type_sig_to_string(&sig[1..])?;
            Some(format!("{}[]", inner))
        }
        _ => None,
    }
}

fn parse_class_type_sig_to_string(sig: &str) -> Option<String> {
    // sig is everything after 'L', e.g. "java/util/List<TE;>;" or "java/lang/String;"
    let bytes = sig.as_bytes();

    // Find where the class name ends (before '<' or ';')
    let mut name_end = 0;
    while name_end < bytes.len() && bytes[name_end] != b'<' && bytes[name_end] != b';' {
        name_end += 1;
    }

    // Simple name = last slash-separated segment
    let class_name = sig[..name_end].split('/').next_back()?;
    // Also strip inner-class '$' separators: prefer the last component
    let class_name = class_name.split('$').next_back().unwrap_or(class_name);

    if name_end >= bytes.len() || bytes[name_end] == b';' {
        return Some(class_name.to_string());
    }

    // We have type arguments starting at '<'
    let args_sig = &sig[name_end..]; // starts with '<'
    let args = parse_type_args_from_sig(args_sig)?;
    if args.is_empty() {
        Some(class_name.to_string())
    } else {
        Some(format!("{}<{}>", class_name, args.join(", ")))
    }
}

fn parse_type_args_from_sig(sig: &str) -> Option<Vec<String>> {
    // sig starts with '<'
    if !sig.starts_with('<') {
        return None;
    }
    let mut args = Vec::new();
    let bytes = sig.as_bytes();
    let mut i = 1; // skip '<'

    while i < bytes.len() && bytes[i] != b'>' {
        let arg = match bytes[i] {
            b'*' => {
                i += 1;
                "?".to_string()
            }
            b'+' | b'-' => {
                i += 1;
                let s = parse_type_sig_to_string(&sig[i..]).unwrap_or_else(|| "?".to_string());
                i = skip_type_sig(bytes, i);
                s
            }
            _ => {
                let s = parse_type_sig_to_string(&sig[i..]).unwrap_or_else(|| "?".to_string());
                i = skip_type_sig(bytes, i);
                s
            }
        };
        args.push(arg);
    }

    Some(args)
}

// ---------------------------------------------------------------------------
// Type-arg string parsing  ("List<String>" → ("List", ["String"]))
// ---------------------------------------------------------------------------

/// Parses a human-readable generic type string into its base name and type
/// argument list.
///
/// - `"String"` → `("String", [])`
/// - `"List<String>"` → `("List", ["String"])`
/// - `"Map<String, List<Integer>>"` → `("Map", ["String", "List<Integer>"])`
pub fn parse_type_ref(s: &str) -> (String, Vec<String>) {
    let s = s.trim();
    let Some(lt) = s.find('<') else {
        return (s.to_string(), vec![]);
    };
    let name = s[..lt].trim().to_string();
    let rest = &s[lt + 1..];
    let Some(gt) = find_matching_gt(rest) else {
        return (name, vec![]);
    };
    let args = split_comma_at_depth_zero(&rest[..gt]);
    (name, args)
}

fn find_matching_gt(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            '>' => return Some(i),
            _ => {}
        }
    }
    None
}

fn split_comma_at_depth_zero(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                let part = s[start..i].trim().to_string();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim().to_string();
    if !last.is_empty() {
        parts.push(last);
    }
    parts
}

// ---------------------------------------------------------------------------
// Binding construction and type-variable substitution
// ---------------------------------------------------------------------------

/// Builds a map from type parameter name to concrete type argument.
///
/// `type_params = ["E"]`, `type_args = ["String"]` → `{"E": "String"}`
/// `type_params = ["K", "V"]`, `type_args = ["String", "Integer"]`
///    → `{"K": "String", "V": "Integer"}`
pub fn build_type_bindings(
    type_params: &[String],
    type_args: &[String],
) -> HashMap<String, String> {
    type_params
        .iter()
        .zip(type_args.iter())
        .map(|(p, a)| (p.clone(), a.clone()))
        .collect()
}

/// Substitutes type variable references in `type_str` using `bindings`.
///
/// - `"E"` + `{E→"String"}` → `"String"`
/// - `"List<E>"` + `{E→"String"}` → `"List<String>"`
/// - `"Map<K, V>"` + `{K→"String", V→"Integer"}` → `"Map<String, Integer>"`
/// - Unbound variables are left as-is.
pub fn substitute_type_vars(type_str: &str, bindings: &HashMap<String, String>) -> String {
    let (name, args) = parse_type_ref(type_str);
    let resolved_name = bindings.get(&name).cloned().unwrap_or(name);
    if args.is_empty() {
        return resolved_name;
    }
    let resolved_args: Vec<String> = args
        .iter()
        .map(|a| substitute_type_vars(a, bindings))
        .collect();
    // If the name itself was a type var that resolved to something generic-looking,
    // attach the args to the resolved name
    format!("{}<{}>", resolved_name, resolved_args.join(", "))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_class_type_params_single() {
        let sig = "<E:Ljava/lang/Object;>Ljava/util/AbstractList<TE;>;Ljava/util/List<TE;>;";
        assert_eq!(parse_class_type_params(sig), vec!["E"]);
    }

    #[test]
    fn test_parse_class_type_params_two() {
        let sig = "<K:Ljava/lang/Object;V:Ljava/lang/Object;>Ljava/lang/Object;";
        assert_eq!(parse_class_type_params(sig), vec!["K", "V"]);
    }

    #[test]
    fn test_parse_class_type_params_none() {
        let sig = "Ljava/lang/Object;";
        assert_eq!(parse_class_type_params(sig), Vec::<String>::new());
    }

    #[test]
    fn test_parse_method_generic_return_type_var() {
        // E get(int index)
        assert_eq!(
            parse_method_generic_return("(I)TE;"),
            Some("E".to_string())
        );
    }

    #[test]
    fn test_parse_method_generic_return_parameterized() {
        // Stream<E> stream()
        let sig = "()Ljava/util/stream/Stream<TE;>;";
        assert_eq!(
            parse_method_generic_return(sig),
            Some("Stream<E>".to_string())
        );
    }

    #[test]
    fn test_parse_method_generic_return_void() {
        assert_eq!(parse_method_generic_return("()V"), Some("void".to_string()));
    }

    #[test]
    fn test_parse_method_generic_return_concrete() {
        // String toString()
        assert_eq!(
            parse_method_generic_return("()Ljava/lang/String;"),
            Some("String".to_string())
        );
    }

    #[test]
    fn test_parse_type_ref_no_args() {
        assert_eq!(parse_type_ref("String"), ("String".to_string(), vec![]));
    }

    #[test]
    fn test_parse_type_ref_single_arg() {
        let (name, args) = parse_type_ref("List<String>");
        assert_eq!(name, "List");
        assert_eq!(args, vec!["String"]);
    }

    #[test]
    fn test_parse_type_ref_two_args() {
        let (name, args) = parse_type_ref("Map<String, Integer>");
        assert_eq!(name, "Map");
        assert_eq!(args, vec!["String", "Integer"]);
    }

    #[test]
    fn test_parse_type_ref_nested() {
        let (name, args) = parse_type_ref("Map<String, List<Integer>>");
        assert_eq!(name, "Map");
        assert_eq!(args, vec!["String", "List<Integer>"]);
    }

    #[test]
    fn test_substitute_simple_var() {
        let mut b = HashMap::new();
        b.insert("E".to_string(), "String".to_string());
        assert_eq!(substitute_type_vars("E", &b), "String");
    }

    #[test]
    fn test_substitute_in_parameterized() {
        let mut b = HashMap::new();
        b.insert("E".to_string(), "String".to_string());
        assert_eq!(substitute_type_vars("List<E>", &b), "List<String>");
    }

    #[test]
    fn test_substitute_two_vars() {
        let mut b = HashMap::new();
        b.insert("K".to_string(), "String".to_string());
        b.insert("V".to_string(), "Integer".to_string());
        assert_eq!(
            substitute_type_vars("Map<K, V>", &b),
            "Map<String, Integer>"
        );
    }

    #[test]
    fn test_substitute_unbound_passthrough() {
        let b = HashMap::new();
        assert_eq!(substitute_type_vars("List<E>", &b), "List<E>");
    }

    // ---------------------------------------------------------------------------
    // parse_method_type_params
    // ---------------------------------------------------------------------------

    #[test]
    fn test_parse_method_type_params_single() {
        // <R> R apply(T t) — one method-level type param
        let sig = "<R:Ljava/lang/Object;>(Ljava/lang/Object;)TR;";
        assert_eq!(parse_method_type_params(sig), vec!["R"]);
    }

    #[test]
    fn test_parse_method_type_params_two() {
        // <K, V> Map<K,V> toMap(...)
        let sig = "<K:Ljava/lang/Object;V:Ljava/lang/Object;>(...)Ljava/util/Map<TK;TV;>;";
        assert_eq!(parse_method_type_params(sig), vec!["K", "V"]);
    }

    #[test]
    fn test_parse_method_type_params_none() {
        // no formal type params — plain method signature
        let sig = "(Ljava/lang/String;)Ljava/lang/String;";
        assert_eq!(parse_method_type_params(sig), Vec::<String>::new());
    }

    #[test]
    fn test_parse_method_type_params_matches_return() {
        // Verify that the extracted param "R" aligns with the return type variable
        // returned by parse_method_generic_return for the same signature.
        let sig = "<R:Ljava/lang/Object;>(Ljava/util/function/Function;)TR;";
        let params = parse_method_type_params(sig);
        let ret = parse_method_generic_return(sig);
        assert_eq!(params, vec!["R"]);
        assert_eq!(ret, Some("R".to_string()));
    }

    // ---------------------------------------------------------------------------
    // TypeBindingPrecedence: receiver wins over call-site via build_type_bindings merge
    // ---------------------------------------------------------------------------

    #[test]
    fn test_receiver_binding_takes_precedence_over_call_site() {
        // Scenario: class Foo<R> has method <R> R transform(...)
        // Receiver binds R → "Integer" (from Foo<Integer>).
        // Call-site explicitly provides R → "String".
        // Receiver must win.
        let receiver_type_params = vec!["R".to_string()];
        let receiver_type_args = vec!["Integer".to_string()];
        let receiver_bindings = build_type_bindings(&receiver_type_params, &receiver_type_args);

        let method_type_params = vec!["R".to_string()];
        let call_site_type_args = vec!["String".to_string()];
        let call_site_bindings = build_type_bindings(&method_type_params, &call_site_type_args);

        // Merge: receiver first, call-site fills only unbound params.
        let mut merged = receiver_bindings.clone();
        for (param, bound) in call_site_bindings {
            merged.entry(param).or_insert(bound);
        }

        assert_eq!(merged.get("R").map(String::as_str), Some("Integer"));
    }

    #[test]
    fn test_call_site_binding_fills_unbound_method_param() {
        // Scenario: class Container has method <R> R convert(...)
        // Receiver has no binding for R (R is method-level only).
        // Call-site provides R → "String".
        let receiver_type_params: Vec<String> = vec!["E".to_string()];
        let receiver_type_args = vec!["Long".to_string()];
        let receiver_bindings = build_type_bindings(&receiver_type_params, &receiver_type_args);

        let method_type_params = vec!["R".to_string()];
        let call_site_type_args = vec!["String".to_string()];
        let call_site_bindings = build_type_bindings(&method_type_params, &call_site_type_args);

        let mut merged = receiver_bindings.clone();
        for (param, bound) in call_site_bindings {
            merged.entry(param).or_insert(bound);
        }

        // Receiver bound E, call-site bound R — both coexist, no conflict.
        assert_eq!(merged.get("E").map(String::as_str), Some("Long"));
        assert_eq!(merged.get("R").map(String::as_str), Some("String"));
    }
}
