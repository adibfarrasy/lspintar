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

    #[test]
    #[ignore]
    fn probe_position_ast_shapes() {
        run(
            "call_expression",
            "object O { def use = foo(1, \"x\", bar) }",
        );
        run(
            "member_access_call",
            "object O { def use = obj.method(1, 2) }",
        );
        run(
            "field_access",
            "object O { def use = obj.field }",
        );
        run(
            "lambda_assigned_to_val",
            "object O { val f = (x: Int) => x + 1 }",
        );
        run(
            "var_assignment",
            "object O { def use = { var x = 1; x = 2; x } }",
        );
        run(
            "literals",
            "object O { val a = 1; val b = 1L; val c = 1.0; val d = 1.0f; val e = true; val s = \"hi\"; val n = null; val c2 = 'x' }",
        );
        run(
            "interpolated_string",
            "object O { val n = s\"$x value\" }",
        );
        run(
            "match_case",
            "object O { def m(x: Any) = x match { case y: Int => y; case _ => 0 } }",
        );
        run(
            "for_comp",
            "object O { def m = for { x <- xs; y <- ys if x > 0 } yield x + y }",
        );
        run(
            "infix_op",
            "object O { val r = a plus b }",
        );
        run(
            "this_super",
            "class C extends B { def m = { this.x; super.f() } }",
        );
        run(
            "qualified_id",
            "object O { val r = scala.collection.immutable.List(1, 2) }",
        );
    }
}
