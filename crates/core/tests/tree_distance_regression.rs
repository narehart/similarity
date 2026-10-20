use similarity_core::apted::{compute_edit_distance, APTEDOptions};
use similarity_core::tree::TreeNode;
use std::rc::Rc;

fn block(children: usize) -> Rc<TreeNode> {
    let mut root = TreeNode::new("block".into(), String::new(), 0);
    for id in 1..=children {
        root.add_child(Rc::new(TreeNode::new("statement".into(), String::new(), id)));
    }
    Rc::new(root)
}

#[test]
fn replacing_unequal_trees_accounts_for_the_other_tree() {
    let options = APTEDOptions::default();
    for count in [1, 10, 200] {
        assert_eq!(compute_edit_distance(&block(0), &block(count), &options), count as f64);
        assert_eq!(compute_edit_distance(&block(count), &block(0), &options), count as f64);
    }
}

#[test]
fn identical_trees_still_have_zero_distance() {
    assert_eq!(compute_edit_distance(&block(10), &block(10), &APTEDOptions::default()), 0.0);
}
