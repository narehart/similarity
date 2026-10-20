use std::collections::HashMap;
use tree_sitter::Node;

pub(crate) fn local_names(function: Node, source: &str) -> HashMap<usize, String> {
    let mut bindings =
        Bindings { source, scopes: vec![HashMap::new()], next: 0, names: HashMap::new() };
    bindings.function(function);
    bindings.names
}

struct Bindings<'a> {
    source: &'a str,
    scopes: Vec<HashMap<String, Option<usize>>>,
    next: usize,
    names: HashMap<usize, String>,
}

impl Bindings<'_> {
    fn text(&self, node: Node) -> &str {
        node.utf8_text(self.source.as_bytes()).unwrap_or("")
    }

    fn record(&mut self, node: Node, binding: usize) {
        self.names.insert(node.start_byte(), format!("$local:{binding}"));
    }

    fn declare(&mut self, node: Node, reuse: bool) {
        let name = self.text(node).to_string();
        if name == "_" {
            return;
        }
        let existing = self.scopes.last().unwrap().get(&name).copied().flatten();
        let binding = match (reuse, existing) {
            (true, Some(binding)) => binding,
            _ => {
                let binding = self.next;
                self.next += 1;
                self.scopes.last_mut().unwrap().insert(name, Some(binding));
                binding
            }
        };
        self.record(node, binding);
    }

    fn declarations(&mut self, node: Node, field: &str, reuse: bool) {
        for name in node.children_by_field_name(field, &mut node.walk()) {
            if name.kind() == "identifier" {
                self.declare(name, reuse);
            }
        }
    }

    fn function(&mut self, node: Node) {
        for field in ["receiver", "parameters", "result"] {
            if let Some(list) = node.child_by_field_name(field) {
                if list.kind() == "parameter_list" {
                    for parameter in list.named_children(&mut list.walk()) {
                        self.declarations(parameter, "name", false);
                    }
                }
            }
        }
        // Parameters and named results share the function body's block scope.
        if let Some(body) = node.child_by_field_name("body") {
            self.children(body);
        }
    }

    fn children(&mut self, node: Node) {
        for child in node.named_children(&mut node.walk()) {
            self.visit(child);
        }
    }

    fn assignment(&mut self, node: Node) {
        // New short-declaration bindings are not visible in their own initializers.
        if let Some(right) = node.child_by_field_name("right") {
            self.visit(right);
        }
        if let Some(left) = node.child_by_field_name("left") {
            let declares = node.children(&mut node.walk()).any(|child| child.kind() == ":=");
            for name in left.named_children(&mut left.walk()) {
                if declares && name.kind() == "identifier" {
                    self.declare(name, true);
                } else {
                    self.visit(name);
                }
            }
        }
    }

    fn type_switch(&mut self, node: Node) {
        self.scopes.push(HashMap::new());
        for field in ["initializer", "value"] {
            if let Some(value) = node.child_by_field_name(field) {
                self.visit(value);
            }
        }
        let alias = node.child_by_field_name("alias").and_then(|list| list.named_child(0));
        let binding = alias.filter(|name| self.text(*name) != "_").map(|name| {
            let binding = self.next;
            self.next += 1;
            self.record(name, binding);
            (self.text(name).to_string(), binding)
        });
        for case in node.named_children(&mut node.walk()) {
            if matches!(case.kind(), "type_case" | "default_case") {
                self.scopes.push(HashMap::new());
                for ty in case.children_by_field_name("type", &mut case.walk()) {
                    self.visit(ty);
                }
                // The guard declares a separate variable in each implicit case block.
                if let Some((name, binding)) = &binding {
                    self.scopes.last_mut().unwrap().insert(name.clone(), Some(*binding));
                }
                for (index, child) in case.named_children(&mut case.walk()).enumerate() {
                    if case.field_name_for_named_child(index as u32) != Some("type") {
                        self.visit(child);
                    }
                }
                self.scopes.pop();
            }
        }
        self.scopes.pop();
    }

    fn keyed_element(&mut self, node: Node) {
        if let Some(key) = node.child_by_field_name("key") {
            let bare_identifier = key.named_child(0).is_some_and(|n| n.kind() == "identifier");
            let mut parent = node.parent();
            let mut collection_type = false;
            while let Some(ancestor) = parent {
                if ancestor.kind() == "composite_literal" {
                    collection_type = ancestor.child_by_field_name("type").is_some_and(|ty| {
                        matches!(
                            ty.kind(),
                            "map_type" | "slice_type" | "array_type" | "implicit_length_array_type"
                        )
                    });
                    break;
                }
                if ancestor.kind() == "literal_value"
                    && ancestor.parent().is_some_and(|p| p.kind() == "literal_element")
                {
                    break;
                }
                parent = ancestor.parent();
            }
            // A bare key of a named type may be a struct field. Without type resolution,
            // preserve it rather than accidentally renaming a field that matches a local.
            if !bare_identifier || collection_type {
                self.visit(key);
            }
        }
        if let Some(value) = node.child_by_field_name("value") {
            self.visit(value);
        }
    }

    fn visit(&mut self, node: Node) {
        match node.kind() {
            "identifier" => {
                if let Some(Some(binding)) =
                    self.scopes.iter().rev().find_map(|scope| scope.get(self.text(node)))
                {
                    self.record(node, *binding);
                }
            }
            "func_literal" => {
                self.scopes.push(HashMap::new());
                self.function(node);
                self.scopes.pop();
            }
            "block"
            | "if_statement"
            | "for_statement"
            | "expression_switch_statement"
            | "expression_case"
            | "type_case"
            | "communication_case"
            | "default_case" => {
                self.scopes.push(HashMap::new());
                self.children(node);
                self.scopes.pop();
            }
            "short_var_declaration" | "range_clause" | "receive_statement" => self.assignment(node),
            "var_spec" | "const_spec" => {
                for field in ["type", "value"] {
                    if let Some(value) = node.child_by_field_name(field) {
                        self.visit(value);
                    }
                }
                self.declarations(node, "name", false);
            }
            "type_spec" | "type_alias" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let name = self.text(name).to_string();
                    self.scopes.last_mut().unwrap().insert(name, None);
                }
            }
            "type_switch_statement" => self.type_switch(node),
            "keyed_element" => self.keyed_element(node),
            _ => self.children(node),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn identifiers(function: &str) -> Vec<String> {
        let source = format!("package p\n{function}");
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_go::LANGUAGE.into()).unwrap();
        let tree = parser.parse(&source, None).unwrap();
        assert!(!tree.root_node().has_error());
        let function = tree.root_node().named_child(1).unwrap();
        let names = local_names(function, &source);
        let mut pending = vec![function.child_by_field_name("body").unwrap()];
        let mut result = Vec::new();
        while let Some(node) = pending.pop() {
            if node.kind() == "identifier" {
                result.push(
                    names
                        .get(&node.start_byte())
                        .cloned()
                        .unwrap_or_else(|| node.utf8_text(source.as_bytes()).unwrap().to_string()),
                );
            }
            let children: Vec<_> = node.named_children(&mut node.walk()).collect();
            pending.extend(children.into_iter().rev());
        }
        result
    }

    #[test]
    fn initializers_resolve_before_new_bindings() {
        assert_eq!(
            identifiers("func F() { x := x; var y = y; use(x, y) }"),
            ["$local:0", "x", "$local:1", "y", "use", "$local:0", "$local:1"]
        );
        assert_eq!(
            identifiers("func F(xs []int) { for _, xs := range xs { use(xs) } }"),
            ["_", "$local:1", "$local:0", "use", "$local:1"]
        );
    }

    #[test]
    fn short_declarations_reuse_only_the_current_block() {
        assert_eq!(
            identifiers("func F(x int) { x, y := x, 1; { x := x; use(x) }; use(x, y) }"),
            [
                "$local:0", "$local:1", "$local:0", "$local:2", "$local:0", "use", "$local:2",
                "use", "$local:0", "$local:1"
            ]
        );
    }

    #[test]
    fn initializer_and_case_bindings_do_not_escape_their_scopes() {
        assert_eq!(
            identifiers("func F() { if x := read(); x > 0 { use(x) } else { use(x) }; use(x) }"),
            ["$local:0", "read", "$local:0", "use", "$local:0", "use", "$local:0", "use", "x"]
        );
        assert_eq!(identifiers("func F(ch chan int) { select { case x := <-ch: use(x); default: use(x) }; use(x) }"),
            ["$local:1", "$local:0", "use", "$local:1", "use", "x", "use", "x"]);
    }

    #[test]
    fn closures_resolve_captures_and_shadowing_separately() {
        assert_eq!(identifiers("func F(x int) { use(func(y int) int { return x + y }); use(func(x int) int { return x }) }"),
            ["use", "$local:1", "$local:0", "$local:1", "use", "$local:2", "$local:2"]);
    }

    #[test]
    fn types_and_struct_keys_do_not_become_local_variable_references() {
        assert_eq!(identifiers("func F(x int) { { type x int; use(x(1)) }; use(x); use(A{x: x}); use(map[int]int{x: x}) }"),
            ["use", "x", "use", "$local:0", "use", "x", "$local:0", "use", "$local:0", "$local:0"]);
    }

    #[test]
    fn type_switch_guard_binding_starts_after_case_types() {
        assert_eq!(
            identifiers("func F(v any) { const x = 1; switch x := v.(type) { case [x]int: use(x); default: use(x) }; use(x) }"),
            ["$local:1", "$local:2", "$local:0", "$local:1", "use", "$local:2", "use", "$local:2", "use", "$local:1"]
        );
    }
}
