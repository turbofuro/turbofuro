use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use tel::StorageValue;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    errors::ExecutionError,
    evaluations::eval_string_param,
    executor::{ExecutionContext, Parameter},
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn get_uuid_v4(
    context: &mut ExecutionContext<'_>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let id = Uuid::new_v4();
    store_value(store_as, context, step_id, id.to_string().into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_uuid_v7(
    context: &mut ExecutionContext<'_>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let id = Uuid::now_v7();
    store_value(store_as, context, step_id, id.to_string().into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn jwt_decode(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let token = eval_string_param("token", parameters, context)?;
    let secret = eval_string_param("secret", parameters, context)?;

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

    store_value(store_as, context, step_id, token_data.claims).await?;
    Ok(())
}

#[cfg(test)]
mod test_crypto {
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
            ],
            "test",
            Some("token")
        ).await.unwrap();

        assert_eq!(
            eval("token.message", &context.storage, &context.environment).unwrap(),
            StorageValue::String("Hello World".to_owned())
        );
    }
}
