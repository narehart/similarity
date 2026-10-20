use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

fn scan(source: &str) -> Output {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("source.go");
    fs::write(&file, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_similarity-generic"))
        .args(["--language", "go", "--threshold", "0.99"])
        .arg(file)
        .output()
        .unwrap()
}

fn assert_pairs(source: &str, expected: bool) {
    let result = scan(source);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let output = String::from_utf8(result.stdout).unwrap();
    assert_eq!(output.contains("<->"), expected, "{source}\n{output}");
}

#[test]
fn distinct_globals_and_literal_keys_are_not_clones() {
    for source in [
        "package p\nfunc Legacy() int { value := legacyPolicy; return value }\nfunc Current() int { value := currentPolicy; return value }",
        "package p\nfunc Content(m map[string]string) string { value := m[\"content\"]; return value }\nfunc Error(m map[string]string) string { value := m[\"error\"]; return value }",
        "package p\nfunc Post(x int) int { return api.Post(x) }\nfunc Once(x int) int { return api.PostOnce(x) }",
        "package p\nfunc Rune() rune { return 'a' }\nfunc OtherRune() rune { return 'b' }",
        "package p\nfunc Imaginary() complex128 { return 1i }\nfunc OtherImaginary() complex128 { return 2i }",
    ] {
        assert_pairs(source, false);
    }
}

#[test]
fn tiny_exact_and_renamed_local_copies_remain_clones() {
    for source in [
        "package p\nfunc First(x int) int { return x + 1 }\nfunc Second(x int) int { return x + 1 }",
        "package p\nfunc First(xs []int) int { total := 0; for _, x := range xs { total += x * 2 }; return total }\nfunc Second(values []int) int { sum := 0; for _, item := range values { sum += item * 2 }; return sum }",
        "package p\ntype A struct{}\nfunc (a A) First(x int) int { return x + 1 }\nfunc (a A) Second(y int) int { return y + 1 }",
    ] {
        assert_pairs(source, true);
    }
}

#[test]
fn same_line_and_unicode_boundaries_do_not_include_neighbors() {
    assert_pairs("package p\nfunc First() string { return \"one\" }; func Second() string { return \"two\" }", false);
    assert_pairs("package p\n// 日本語\nfunc First() string { return \"一\" }; func Second() string { return \"二\" }", false);
}

#[test]
fn empty_methods_have_no_implementation_to_duplicate() {
    assert_pairs(
        "package p\ntype A struct{}\nfunc (A) First() {}\nfunc (A) Second() {}\nfunc External()",
        false,
    );
}

#[test]
fn syntax_errors_are_not_successful_empty_scans() {
    let result = scan("package p\nfunc Valid() int { return 1 }\nfunc Broken( {");
    assert!(!result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
}

#[test]
fn renamed_bindings_preserve_copies_across_go_scopes() {
    for (first, second) in [
        ("func First(items []int) int { total := 0; for _, item := range items { total += item }; return total }",
         "func Second(values []int) int { sum := 0; for _, value := range values { sum += value }; return sum }"),
        ("func First(x int) int { var y = x; const z = 2; return y + z }",
         "func Second(a int) int { var b = a; const c = 2; return b + c }"),
        ("func First(x int) int { y := x; { y := y + 1; use(y) }; return y }",
         "func Second(a int) int { b := a; { c := b + 1; use(c) }; return b }"),
        ("func First(x int) int { x, y := x, 1; return x + y }",
         "func Second(a int) int { a, b := a, 1; return a + b }"),
        ("func First(x int) func(int) int { return func(y int) int { return x + y } }",
         "func Second(a int) func(int) int { return func(b int) int { return a + b } }"),
        ("func First() (result int) { result = 2; return }",
         "func Second() (value int) { value = 2; return }"),
        ("func (r A) First(x int) int { return r.Value + x }",
         "func (s A) Second(y int) int { return s.Value + y }"),
        ("func First() { if x := read(); x > 0 { use(x) } else { use(x) }; use(x) }",
         "func Second() { if y := read(); y > 0 { use(y) } else { use(y) }; use(x) }"),
        ("func First() { for i := 0; i < 2; i++ { use(i) }; use(i) }",
         "func Second() { for j := 0; j < 2; j++ { use(j) }; use(i) }"),
        ("func First(ch chan int) { select { case x := <-ch: use(x); default: use(x) } }",
         "func Second(c chan int) { select { case y := <-c: use(y); default: use(x) } }"),
        ("func First(v any) { switch x := v.(type) { case int: x, y := x, 1; use(x, y); default: use(x) } }",
         "func Second(a any) { switch b := a.(type) { case int: b, c := b, 1; use(b, c); default: use(b) } }"),
        ("func First(key int) any { return map[int]int{key: key} }",
         "func Second(index int) any { return map[int]int{index: index} }"),
    ] {
        assert_pairs(&format!("package p\ntype A struct {{ Value int }}\n{first}\n{second}"), true);
    }
}

#[test]
fn local_normalization_does_not_erase_external_names_or_binding_identity() {
    for (first, second) in [
        (
            "func First() int { value := legacyPolicy; return value }",
            "func Second() int { result := currentPolicy; return result }",
        ),
        (
            "func First() int { value := value; return value }",
            "func Second() int { result := result; return result }",
        ),
        ("func First(x, y int) int { return x + x }", "func Second(a, b int) int { return a + b }"),
        (
            "func First(x int) int { { x := x + 1; use(x) }; return x }",
            "func Second(y int) int { { z := y + 1; use(y) }; return y }",
        ),
        (
            "func First() { for _, x := range x { use(x) } }",
            "func Second() { for _, y := range y { use(y) } }",
        ),
        (
            "func First(x int) func(int) int { return func(x int) int { return x } }",
            "func Second(a int) func(int) int { return func(b int) int { return a } }",
        ),
        ("func First(x A) int { return x.First }", "func Second(y A) int { return y.Second }"),
        ("func First(x int) any { return A{x: x} }", "func Second(y int) any { return A{y: y} }"),
    ] {
        assert_pairs(&format!("package p\n{first}\n{second}"), false);
    }
}
