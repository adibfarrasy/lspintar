use lsp_core::{node_kind::NodeKind, util::strip_comment_signifiers};

use crate::{
    constants::{HOVER_ANNOTATION_FILTER, HOVER_MODIFIER_FILTER},
    models::symbol::{SymbolMetadata, SymbolParameter},
};

pub fn build_hover_parts(
    file_type: &str,
    package_name: &str,
    short_name: &str,
    symbol_type: &str,
    modifiers: &[String],
    metadata: &SymbolMetadata,
) -> Vec<String> {
    let mut parts = Vec::new();
    parts.push(format!("```{}", file_type));
    if !package_name.is_empty() {
        parts.push(format!("package {}", package_name));
        parts.push(String::new());
    }
    if let Some(annotations) = &metadata.annotations {
        for annotation in annotations {
            if !annotation.is_empty() && !HOVER_ANNOTATION_FILTER.contains(&annotation.as_str()) {
                parts.push(format!("@{}", annotation));
            }
        }
    }

    let node_kind = NodeKind::from_string(symbol_type);
    let modifiers_str = modifiers
        .iter()
        .cloned()
        .filter(|m| !HOVER_MODIFIER_FILTER.contains(&m.as_str()))
        .collect::<Vec<_>>()
        .join(" ");
    let mut signature_line = String::new();

    if !modifiers_str.is_empty() {
        signature_line.push_str(&modifiers_str);
        signature_line.push(' ');
    }

    match node_kind {
        Some(nk @ (NodeKind::Function | NodeKind::Field)) => {
            if let Some(kw) = nk.keyword(file_type) {
                signature_line.push_str(kw);
                signature_line.push(' ');
            }
            if file_type != "kotlin" {
                if let Some(ret) = &metadata.return_type {
                    signature_line.push_str(ret);
                    signature_line.push(' ');
                }
                signature_line.push_str(short_name);
            } else {
                signature_line.push_str(short_name);
            }
        }
        Some(ref nk) => {
            if let Some(kw) = nk.keyword(file_type) {
                signature_line.push_str(kw);
                signature_line.push(' ');
            }

            signature_line.push_str(short_name);
        }
        None => signature_line.push_str(short_name),
    }

    if let Some(params) = &metadata.parameters
        && !params.is_empty()
    {
        let format_param = |p: &SymbolParameter| {
            let mut s = match &p.type_name {
                Some(t) => {
                    if file_type == "kotlin" {
                        format!("{}: {}", p.name, t)
                    } else {
                        format!("{} {}", t, p.name)
                    }
                }
                None => p.name.clone(),
            };

            if let Some(default) = &p.default_value {
                s.push_str(&format!(" = {}", default));
            }

            s
        };
        if params.len() > 3 {
            signature_line.push('(');
            for (i, param) in params.iter().enumerate() {
                let sep = if i < params.len() - 1 { "," } else { "\n" };
                signature_line.push_str(&format!("\n\t{}{}", format_param(param), sep));
            }
            signature_line.push(')');
        } else {
            let params_str = params
                .iter()
                .map(format_param)
                .collect::<Vec<_>>()
                .join(", ");
            signature_line.push_str(&format!("({})", params_str));
        }
    }

    if file_type == "kotlin".to_string() {
        if let Some(ret) = &metadata.return_type {
            signature_line.push_str(": ");
            signature_line.push_str(ret);
            signature_line.push(' ');
        }
    }

    parts.push(signature_line);

    if metadata.documentation.is_some() {
        parts.push(String::new());
        parts.push("---".to_string());
    }
    parts.push("```".to_string());
    if let Some(doc) = &metadata.documentation
        && !doc.is_empty()
    {
        parts.push(strip_comment_signifiers(doc));
    }

    parts
}
