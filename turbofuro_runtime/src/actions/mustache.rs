use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_param, eval_string_param},
    executor::{ExecutionContext, Parameter},
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn render_template(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let raw_template = eval_string_param("template", parameters, context)?;

    // TODO: Cache compiled templates
    let template =
        mustache::compile_str(&raw_template).map_err(|e| ExecutionError::ParameterInvalid {
            name: "template".into(),
            message: e.to_string(),
        })?;

    let data = eval_param("data", parameters, &context.storage, &context.environment)?;

    let output = template
        .render_to_string(&data)
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Could not render template".into(),
            subject: "template".into(),
            inner: e.to_string(),
        })?;

    store_value(store_as, context, step_id, output.into()).await?;
    Ok(())
}

#[cfg(test)]
mod test_mustache {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

    #[tokio::test]
    async fn test_render_template() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        render_template(
            &mut context,
            &vec![
                Parameter::tel("template", "\"Hello {{name}}! This is {{test}}\""),
                Parameter::tel("data", "{ \"name\": \"World\", \"test\": \"a test\" }"),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output", &context.storage, &context.environment).unwrap(),
            "Hello World! This is a test".into()
        );
    }
}
