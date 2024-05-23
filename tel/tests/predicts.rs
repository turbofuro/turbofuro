use std::collections::HashMap;

use serde_json::json;
use tel::{describe, predict_description, parse, StorageValue};

#[test]
fn test_predicts_cases() {
    let file_contents = include_str!("predicts.js");
    let cases: Vec<&str> = file_contents.split('\n').collect::<_>();

    for case in cases.iter() {
        if case.starts_with("//") {
            continue;
        }

        let splited: Vec<&str> = case.split("CHECK").collect();
        let case = splited[0];
        let expected = splited[1].trim();

        println!("Case: {:?}", case);
        let result = parse(case);
        // println!("Program: {:?}", program);

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
        let mut storage_description = storage
            .into_iter()
            .map(|(k, v)| (k, describe(v)))
            .collect::<HashMap<_, _>>();
        storage_description.insert("anything".to_owned(), tel::Description::Any);
        let env_json = json!(
            {
                "REDIS_URL": "redis://localhost:6379",
                "TOKEN": "test",
                "NUMBER": 3
            }
        );
        let environment: HashMap<String, StorageValue> = serde_json::from_value(env_json).unwrap();
        let environment_description = environment
            .into_iter()
            .map(|(k, v)| (k, describe(v)))
            .collect::<HashMap<_, _>>();

        let description = predict_description(
            result.expr.unwrap(),
            &storage_description,
            &environment_description,
        );

        assert_eq!(expected, description.to_form_string());
    }
}
