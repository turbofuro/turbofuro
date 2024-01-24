use tel::parse;

#[test]
fn test_failures_cases() {
    let file_contents = include_str!("failures.js");
    let cases: Vec<&str> = file_contents.split('\n').collect::<_>();

    for case in cases.iter() {
        if case.starts_with("//") {
            continue;
        }

        let result = parse(case);
        assert!(!result.errors.is_empty());

        println!("FAILURE TEST");
        println!("Case: {:?}", case);
        println!("Result: {:?}", result);
    }
}
