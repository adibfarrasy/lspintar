#[cfg(test)]
mod probe {
    use tree_sitter::{Node, Parser};

    fn dump(node: Node, depth: usize, src: &str, out: &mut String) {
        let indent = "  ".repeat(depth);
        let text = node
            .utf8_text(src.as_bytes())
            .unwrap_or("")
            .replace('\n', " ");
        let preview = if text.len() > 40 { &text[..40] } else { &text };
        out.push_str(&format!("{indent}{}  «{preview}»\n", node.kind()));
        let mut c = node.walk();
        for child in node.children(&mut c) {
            dump(child, depth + 1, src, out);
        }
    }

    fn run(label: &str, src: &str) {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_scala::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        let mut out = String::new();
        out.push_str(&format!("====== {label} ======\n"));
        dump(tree.root_node(), 0, src, &mut out);
        eprintln!("{out}");
    }

    #[test]
    #[ignore]
    fn probe_ast_shapes() {
        run(
            "extends_with",
            "class C extends A with B with D { }",
        );
        run(
            "trait_extends",
            "trait T extends A with B { }",
        );
        run(
            "modifiers_annotations",
            "@Deprecated(\"x\")\n@Override\nfinal private class Foo extends Bar { def m: Int = 1 }",
        );
        run(
            "function_params_return",
            "def f(a: Int, b: String = \"x\", c: List[Int]): Map[String, Int] = ???",
        );
        run(
            "class_params",
            "class P(val x: Int, var y: String = \"z\") extends A",
        );
        run(
            "case_class",
            "case class CC(a: Int, b: String) extends Base with Mix",
        );
        run(
            "scaladoc",
            "/** hello\n  * doc\n  */\nclass D",
        );
        run(
            "override_def",
            "class C extends A { override def f(x: Int): String = \"y\" }",
        );
        run(
            "given_extension",
            "given intOrd: Ord[Int] = ???\nextension (s: String) def shout: String = s.toUpperCase",
        );
        run(
            "type_def",
            "type Alias = Map[String, Int]",
        );
        run(
            "object_with_doc_and_annot",
            "/** hi */\n@experimental\nobject Z { val a = 1 }",
        );
    }
}
