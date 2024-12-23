use omnitool::{analyze_instructions, AnalysisProblem, FunctionDeclaration, Step};

#[test]
fn test_analyzes_1() {
    let instructions: Vec<Step> = serde_json::from_str(include_str!("instructions1.json")).unwrap();
    let declarations: Vec<FunctionDeclaration> =
        serde_json::from_str(include_str!("declarations1.json")).unwrap();

    let result = analyze_instructions(&instructions, declarations, vec![], None);

    println!("{:?}", result);
    assert_eq!(result.len(), 12);
    for step in result {
        if step.id == "indx7bOYBOfBzQHuUb5uw" {
            assert_eq!(step.problems.len(), 1);
            assert_eq!(
                step.problems[0],
                AnalysisProblem::Error {
                    code: "UNREACHABLE_CODE".to_owned(),
                    message: "This code is unreachable".to_owned(),
                    field: None
                }
            );
            continue;
        }
        assert!(step.problems.is_empty());
    }
}

#[test]
fn test_analyzes_2() {
    let instructions: Vec<Step> = serde_json::from_str(include_str!("instructions2.json")).unwrap();
    let declarations: Vec<FunctionDeclaration> =
        serde_json::from_str(include_str!("declarations2.json")).unwrap();

    let result = analyze_instructions(&instructions, declarations, vec![], None);

    println!("{:?}", result);
    assert_eq!(result.len(), 6);
    for step in result {
        if step.id == "kuiDpVb_mJOr1QFaQ44Mw" {
            let mut codes = step
                .problems
                .iter()
                .map(|p| p.code())
                .collect::<Vec<&str>>();
            codes.sort();
            assert_eq!(
                codes,
                vec![
                    "INCOMPATIBLE_VALUE",
                    "UNEXPECTED_PARAMETER",
                    "UNREACHABLE_CODE",
                ]
            );
            continue;
        } else if step.id == "dSIDq-Ga5PaRsNYWjkCZP" {
            let mut codes = step
                .problems
                .iter()
                .map(|p| p.code())
                .collect::<Vec<&str>>();
            codes.sort();
            assert_eq!(codes, vec!["INCOMPATIBLE_VALUE"]);
        } else {
            assert!(step.problems.is_empty());
        }
    }
}
