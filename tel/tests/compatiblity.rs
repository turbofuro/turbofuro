use tel::{evaluate_description_notation, parse_description};

#[test]
fn test_description_compatibility_cases() {
    let file_contents = include_str!("compatiblity.js");
    let cases: Vec<&str> = file_contents.split('\n').collect::<_>();

    for window in cases.chunks(2) {
        let a = window[0];
        let b = window[1];
        if a.starts_with("//") || b.starts_with("//") {
            continue;
        }

        println!("A / B: {a:?} {b:?}");

        let a = {
            let result = parse_description(a);
            let expr = result.expr.expect("Expected expression");
            evaluate_description_notation(expr).unwrap()
        };

        let b = {
            let result = parse_description(b);
            let expr = result.expr.expect("Expected expression");
            evaluate_description_notation(expr).unwrap()
        };

        assert!(a.is_compatible(&b));
    }
}
