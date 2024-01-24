use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use tel::StorageValue;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_param, eval_saver_param},
    executor::{ExecutionContext, Parameter},
};

use super::as_string;

#[instrument(level = "trace", skip_all)]
pub fn get_uuid_v4(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let id = Uuid::new_v4();

    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;

    context.add_to_storage(step_id, selector, id.to_string().into())?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn get_uuid_v7(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let id = Uuid::now_v7();

    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;

    context.add_to_storage(step_id, selector, id.to_string().into())?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub fn jwt_decode(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let token = as_string(
        eval_param("token", parameters, &context.storage, &context.environment)?,
        "token",
    )?;

    let secret = as_string(
        eval_param("secret", parameters, &context.storage, &context.environment)?,
        "secret",
    )?;

    // TODO: Add more options?
    // let should_validate = eval_optional_param_with_default(
    //     "validate",
    //     parameters,
    //     &context.storage,
    //     &context.environment,
    //     true.into(),
    // )?
    // .to_boolean()
    // .map_err(ExecutionError::from)?;

    // TODO: Add more options?
    let validation = Validation::new(Algorithm::HS256);
    let token_data = decode::<StorageValue>(
        &token,
        &DecodingKey::from_secret(secret.as_ref()),
        &validation,
    )
    .map_err(|e| ExecutionError::JwtDecodingFailed {
        message: e.to_string(),
    })?;

    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;

    context.add_to_storage(step_id, selector, token_data.claims)?;

    Ok(())
}

#[cfg(test)]
mod test_jwt {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

    #[tokio::test]
    async fn test_jwt_decode_successfully() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        jwt_decode(
            &mut context,
            &vec![
                Parameter::tel("token", "\"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJtZXNzYWdlIjoiSGVsbG8gV29ybGQiLCJleHAiOjQ1NTI1NTI1MjJ9.qRQDe--OzD0QanQeieeE-cJadvrPf7Gpck-fa4l78Ro\""),

            Parameter::tel("secret", "\"secret\""),
            Parameter::tel("saveAs", "token"),
            ],
            "test"
        ).unwrap();

        assert_eq!(
            eval("token.message", &context.storage, &context.environment).unwrap(),
            StorageValue::String("Hello World".to_owned())
        );
    }
}
