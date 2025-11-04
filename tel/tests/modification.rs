use std::collections::HashMap;

use serde_json::json;
use tel::{evaluate_selector, evaluate_value, parse, store_value, StorageValue};

#[test]
fn test_modification_cases() {
    let file_contents = include_str!("modification.js");
    let cases: Vec<&str> = file_contents.split('\n').collect::<_>();

    for case in cases.iter() {
        if case.starts_with("//") {
            continue;
        }

        let splited: Vec<&str> = case.split("CHECK").collect();
        println!("Case: {splited:?}");
        let case = splited[0];
        let expected = splited[1];

        let modifier_result = parse(case);
        println!("Modifier: {modifier_result:?}");

        let check_result = parse(expected);
        println!("Check: {check_result:?}");

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
        let mut storage: HashMap<String, StorageValue> =
            serde_json::from_value(storage_json).unwrap();
        let env_json = json!(
            {
                "REDIS_URL": "redis://localhost:6379",
                "TOKEN": "test",
                "NUMBER": 3
            }
        );
        let environment: HashMap<String, StorageValue> = serde_json::from_value(env_json).unwrap();

        let selector =
            evaluate_selector(modifier_result.expr.unwrap(), &storage, &environment).unwrap();

        store_value(
            &selector,
            &mut storage,
            tel::StorageValue::String("hello".to_string()),
        )
        .unwrap();

        println!(
            "Storage: {}",
            serde_json::to_string_pretty(&storage).unwrap()
        );

        match evaluate_value(check_result.expr.unwrap(), &storage, &environment).unwrap() {
            StorageValue::Boolean(f) => assert!(f),
            _ => panic!("Expected boolean value"),
        }
    }
}
