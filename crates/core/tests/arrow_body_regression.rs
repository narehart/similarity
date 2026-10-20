use similarity_core::{compute_edit_distance, parse_and_convert_to_tree, APTEDOptions, TreeNode};
use std::rc::Rc;

fn arrow_body(source: &str) -> Rc<TreeNode> {
    let tree = parse_and_convert_to_tree("arrow.ts", source).unwrap();
    let mut pending = vec![tree];
    while let Some(node) = pending.pop() {
        if node.label == "ArrowFunction" {
            return node.children.last().unwrap().clone();
        }
        pending.extend(node.children.iter().cloned());
    }
    panic!("No arrow function in {source}");
}

#[test]
fn concise_arrows_represent_their_implicit_return() {
    for expression in ["a + b", "calculate(a)", "42", "a => a + 1"] {
        let concise = arrow_body(&format!("const f = (a, b) => {expression};"));
        let explicit = arrow_body(&format!("const f = (a, b) => {{ return {expression}; }};"));
        assert_eq!(concise.label, "BlockStatement");
        assert_eq!(concise.children[0].label, "ReturnStatement");
        assert_eq!(
            compute_edit_distance(&concise, &explicit, &APTEDOptions::default()),
            0.0,
            "{expression}"
        );
    }
}

#[test]
fn expression_statements_do_not_become_returns() {
    let concise = arrow_body("const f = a => calculate(a);");
    let side_effect = arrow_body("const f = a => { calculate(a); };");
    assert_eq!(side_effect.label, "BlockStatement");
    assert_eq!(side_effect.children[0].label, "CallExpression");
    assert!(compute_edit_distance(&concise, &side_effect, &APTEDOptions::default()) > 0.0);
    let empty = arrow_body("const f = a => {};");
    assert!(empty.children.is_empty());
}
