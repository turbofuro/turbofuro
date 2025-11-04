use std::collections::HashMap;

use regex::Regex;
use tel::{StorageValue, NULL};
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::eval_string_param,
    executor::{ExecutionContext, Parameter},
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn matches(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let expression = eval_string_param("expression", parameters, context)?;
    let value = eval_string_param("value", parameters, context)?;

    let regex = Regex::new(&expression).map_err(|e| ExecutionError::ParameterInvalid {
        name: "expression".to_owned(),
        message: e.to_string(),
    })?;

    store_value(store_as, context, step_id, regex.is_match(&value).into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn find_all(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let expression = eval_string_param("expression", parameters, context)?;
    let value = eval_string_param("value", parameters, context)?;

    let regex = Regex::new(&expression).map_err(|e| ExecutionError::ParameterInvalid {
        name: "expression".to_owned(),
        message: e.to_string(),
    })?;

    let mut matches = vec![];
    for found in regex.find_iter(&value) {
        let mut match_object: HashMap<String, StorageValue> = HashMap::new();
        match_object.insert("text".to_owned(), found.as_str().into());
        match_object.insert("start".to_owned(), found.start().into());
        match_object.insert("end".to_owned(), found.end().into());
        matches.push(StorageValue::Object(match_object));
    }

    store_value(store_as, context, step_id, StorageValue::Array(matches)).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn find_first(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let expression = eval_string_param("expression", parameters, context)?;
    let value = eval_string_param("value", parameters, context)?;

    let regex = Regex::new(&expression).map_err(|e| ExecutionError::ParameterInvalid {
        name: "expression".to_owned(),
        message: e.to_string(),
    })?;

    if let Some(found) = regex.find(&value) {
        let mut match_object: HashMap<String, StorageValue> = HashMap::new();
        match_object.insert("text".to_owned(), found.as_str().into());
        match_object.insert("start".to_owned(), found.start().into());
        match_object.insert("end".to_owned(), found.end().into());
        store_value(
            store_as,
            context,
            step_id,
            StorageValue::Object(match_object),
        )
        .await?;
    } else {
        store_value(store_as, context, step_id, NULL).await?;
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn replace(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let expression = eval_string_param("expression", parameters, context)?;
    let value = eval_string_param("value", parameters, context)?;
    let replacement = eval_string_param("replacement", parameters, context)?;

    let regex = Regex::new(&expression).map_err(|e| ExecutionError::ParameterInvalid {
        name: "expression".to_owned(),
        message: e.to_string(),
    })?;

    let output = regex.replace(&value, replacement).to_string();
    store_value(store_as, context, step_id, output.into()).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn replace_all(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let expression = eval_string_param("expression", parameters, context)?;
    let value = eval_string_param("value", parameters, context)?;
    let replacement = eval_string_param("replacement", parameters, context)?;

    let regex = Regex::new(&expression).map_err(|e| ExecutionError::ParameterInvalid {
        name: "expression".to_owned(),
        message: e.to_string(),
    })?;

    let output = regex.replace_all(&value, replacement).to_string();
    store_value(store_as, context, step_id, output.into()).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn capture(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let expression = eval_string_param("expression", parameters, context)?;
    let value = eval_string_param("value", parameters, context)?;

    let regex = Regex::new(&expression).map_err(|e| ExecutionError::ParameterInvalid {
        name: "expression".to_owned(),
        message: e.to_string(),
    })?;

    let mut groups = vec![];
    if let Some(capture) = regex.captures(&value) {
        for m in capture.iter() {
            if let Some(m) = m {
                let mut match_object: HashMap<String, StorageValue> = HashMap::new();
                match_object.insert("text".to_owned(), m.as_str().into());
                match_object.insert("start".to_owned(), m.start().into());
                match_object.insert("end".to_owned(), m.end().into());
                groups.push(StorageValue::Object(match_object));
            } else {
                groups.push(NULL);
            }
        }
    }

    store_value(store_as, context, step_id, StorageValue::Array(groups)).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn capture_all(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let expression = eval_string_param("expression", parameters, context)?;
    let value = eval_string_param("value", parameters, context)?;

    let regex = Regex::new(&expression).map_err(|e| ExecutionError::ParameterInvalid {
        name: "expression".to_owned(),
        message: e.to_string(),
    })?;

    let mut captures = vec![];
    for capture in regex.captures_iter(&value) {
        let mut groups = vec![];
        for m in capture.iter() {
            if let Some(m) = m {
                let mut match_object: HashMap<String, StorageValue> = HashMap::new();
                match_object.insert("text".to_owned(), m.as_str().into());
                match_object.insert("start".to_owned(), m.start().into());
                match_object.insert("end".to_owned(), m.end().into());
                groups.push(StorageValue::Object(match_object));
            } else {
                groups.push(NULL);
            }
        }
        captures.push(StorageValue::Array(groups));
    }

    store_value(store_as, context, step_id, StorageValue::Array(captures)).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

    #[tokio::test]
    async fn test_not_matches() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        matches(
            &mut context,
            &vec![
                Parameter::tel("expression", "\"^hello$\""),
                Parameter::tel("value", "\"hello world\""),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output", &context.storage, &context.environment).unwrap(),
            false.into()
        );
    }

    #[tokio::test]
    async fn test_matches() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        matches(
            &mut context,
            &vec![
                Parameter::tel("expression", "\"^hello$\""),
                Parameter::tel("value", "\"hello\""),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output", &context.storage, &context.environment).unwrap(),
            true.into()
        );
    }

    #[tokio::test]
    async fn test_find() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        find_first(
            &mut context,
            &vec![
                Parameter::tel("expression", "\"\\\\b\\\\w{4}\\\\b\""),
                Parameter::tel("value", "\"hello world test chonk buba\""),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output.start", &context.storage, &context.environment).unwrap(),
            12.into()
        );
        assert_eq!(
            eval("output.end", &context.storage, &context.environment).unwrap(),
            16.into()
        );
        assert_eq!(
            eval("output.text", &context.storage, &context.environment).unwrap(),
            "test".into()
        );
    }

    #[tokio::test]
    async fn test_find_all() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        find_all(
            &mut context,
            &vec![
                Parameter::tel("expression", "\"\\\\b\\\\w{4}\\\\b\""),
                Parameter::tel("value", "\"hello world test chonk gato\""),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output[0].start", &context.storage, &context.environment).unwrap(),
            12.into()
        );
        assert_eq!(
            eval("output[0].end", &context.storage, &context.environment).unwrap(),
            16.into()
        );
        assert_eq!(
            eval("output[0].text", &context.storage, &context.environment).unwrap(),
            "test".into()
        );

        assert_eq!(
            eval("output[1].start", &context.storage, &context.environment).unwrap(),
            23.into()
        );
        assert_eq!(
            eval("output[1].end", &context.storage, &context.environment).unwrap(),
            27.into()
        );
        assert_eq!(
            eval("output[1].text", &context.storage, &context.environment).unwrap(),
            "gato".into()
        );
    }

    #[tokio::test]
    async fn test_replace() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        replace(
            &mut context,
            &vec![
                Parameter::tel("expression", "\"\\\\s(dog(s)?)\""),
                Parameter::tel("value", "\"cat dog cats dogs\""),
                Parameter::tel("replacement", "\"\""),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output", &context.storage, &context.environment).unwrap(),
            "cat cats dogs".into()
        );
    }

    #[tokio::test]
    async fn test_replace_all() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        replace_all(
            &mut context,
            &vec![
                Parameter::tel("expression", "\"\\\\s(dog(s)?)\""),
                Parameter::tel("value", "\"cat dog cats dogs\""),
                Parameter::tel("replacement", "\"\""),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output", &context.storage, &context.environment).unwrap(),
            "cat cats".into()
        );
    }

    #[tokio::test]
    async fn test_captures() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        capture(
            &mut context,
            &vec![
                Parameter::tel("expression", "\"(dog(s)?)\""),
                Parameter::tel("value", "\"cat dogs cats\""),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output[0].text", &context.storage, &context.environment).unwrap(),
            "dogs".into()
        );
        assert_eq!(
            eval("output[1].text", &context.storage, &context.environment).unwrap(),
            "dogs".into()
        );
        assert_eq!(
            eval("output[2].text", &context.storage, &context.environment).unwrap(),
            "s".into()
        );
        assert_eq!(
            eval("output.length", &context.storage, &context.environment).unwrap(),
            3.into()
        );
    }

    #[tokio::test]
    async fn test_captures_all() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        capture_all(
            &mut context,
            &vec![
                Parameter::tel("expression", "\"(dog(s)?)\""),
                Parameter::tel("value", "\"cat dog cats dogs\""),
                Parameter::tel("replacement", "\"\""),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output[0][0].text", &context.storage, &context.environment).unwrap(),
            "dog".into()
        );
        assert_eq!(
            eval("output[1][0].text", &context.storage, &context.environment).unwrap(),
            "dogs".into()
        );
        assert_eq!(
            eval("output[1][1].text", &context.storage, &context.environment).unwrap(),
            "dogs".into()
        );
        assert_eq!(
            eval("output[1][2].text", &context.storage, &context.environment).unwrap(),
            "s".into()
        );
        assert_eq!(
            eval("output.length", &context.storage, &context.environment).unwrap(),
            2.into()
        );
    }
}
