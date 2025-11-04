use std::collections::HashMap;

use serde_json::json;
use tel::{evaluate_value, parse, StorageValue};

#[test]
fn test_conditional_cases() {
    let file_contents = include_str!("conditions.js");
    let cases: Vec<&str> = file_contents.split('\n').collect::<_>();

    for case in cases.iter() {
        if case.starts_with("//") {
            continue;
        }
        println!("Case: {case:?}");
        let result = parse(case);
        let expr = result.expr.expect("Expected expression");

        println!("Program: {expr:?}");

        let storage_json = json!(
            {
                "request": {
                    "headers": {
                        "access-control-allow-origin": "*",
                        "user-agent": "Tel",
                    },
                    "body": {
                        "id": 1,
                        "name": "John Doe"
                    }
                },
                "numbers": [1, 2, 3, 4, 5],
            }
        );
        let storage: HashMap<String, StorageValue> = serde_json::from_value(storage_json).unwrap();
        let env_json = json!(
            {
                "REDIS_URL": "redis://localhost:6379",
                "TOKEN": "test",
                "NUMBER": 3
            }
        );
        let environment: HashMap<String, StorageValue> = serde_json::from_value(env_json).unwrap();

        match evaluate_value(expr, &storage, &environment).unwrap() {
            StorageValue::Boolean(f) => assert!(f),
            s => panic!("Expected boolean value got {s:?}"),
        }
    }
}
