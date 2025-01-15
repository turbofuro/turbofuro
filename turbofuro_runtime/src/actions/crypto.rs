use hmac::{Hmac, Mac};
use image::EncodableLayout;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};
use tel::StorageValue;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_byte_array_param, eval_opt_string_param, eval_string_param},
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

    let algorithm = eval_opt_string_param("algorithm", parameters, context)?
        .unwrap_or_else(|| "HS256".to_owned());
    let algorithm: Algorithm = match algorithm.as_str() {
        "HS256" => Algorithm::HS256,
        "HS384" => Algorithm::HS384,
        "HS512" => Algorithm::HS512,
        // TODO: Implement these in the future
        // "RS256" => Algorithm::RS256,
        // "RS384" => Algorithm::RS384,
        // "RS512" => Algorithm::RS512,
        // "ES256" => Algorithm::ES256,
        // "ES384" => Algorithm::ES384,
        // "PS256" => Algorithm::PS256,
        // "PS384" => Algorithm::PS384,
        // "PS512" => Algorithm::PS512,
        _ => {
            return Err(ExecutionError::ParameterInvalid {
                name: "algorithm".to_owned(),
                message: "Invalid algorithm".to_owned(),
            })
        }
    };

    // TODO: Add more options?
    let validation = Validation::new(algorithm);
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

#[instrument(level = "trace", skip_all)]
pub async fn sha2(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let hash =
        eval_opt_string_param("hash", parameters, context)?.unwrap_or_else(|| "sha512".to_owned());
    let data = eval_byte_array_param("data", parameters, context)?;

    let result = match hash.as_str() {
        "sha512" => {
            let mut hasher = Sha512::new();
            hasher.update(data.as_bytes());
            StorageValue::new_byte_array(hasher.finalize().as_slice())
        }
        "sha256" => {
            let mut hasher = Sha256::new();
            hasher.update(data.as_bytes());
            StorageValue::new_byte_array(hasher.finalize().as_slice())
        }
        "sha384" => {
            let mut hasher = Sha384::new();
            hasher.update(data.as_bytes());
            StorageValue::new_byte_array(hasher.finalize().as_slice())
        }
        "sha224" => {
            let mut hasher = Sha224::new();
            hasher.update(data.as_bytes());
            StorageValue::new_byte_array(hasher.finalize().as_slice())
        }
        _ => {
            return Err(ExecutionError::ParameterInvalid {
                name: "hash".to_owned(),
                message: "Invalid hash algorithm".to_owned(),
            })
        }
    };

    store_value(store_as, context, step_id, result).await?;
    Ok(())
}

impl From<hmac::digest::InvalidLength> for ExecutionError {
    fn from(_error: hmac::digest::InvalidLength) -> Self {
        ExecutionError::ParameterInvalid {
            name: "key".to_owned(),
            message: "Invalid hash length".to_owned(),
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn hmac(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let hash =
        eval_opt_string_param("hash", parameters, context)?.unwrap_or_else(|| "sha512".to_owned());
    let data = eval_byte_array_param("data", parameters, context)?;
    let key = eval_byte_array_param("key", parameters, context)?;

    let result = match hash.as_str() {
        "sha512" => {
            let mut hmac = Hmac::<Sha512>::new_from_slice(&key).map_err(ExecutionError::from)?;
            hmac.update(data.as_bytes());
            StorageValue::new_byte_array(hmac.finalize().into_bytes().as_slice())
        }
        "sha256" => {
            let mut hmac = Hmac::<Sha256>::new_from_slice(&key).map_err(ExecutionError::from)?;
            hmac.update(data.as_bytes());
            StorageValue::new_byte_array(hmac.finalize().into_bytes().as_slice())
        }
        "sha384" => {
            let mut hmac = Hmac::<Sha384>::new_from_slice(&key).map_err(ExecutionError::from)?;
            hmac.update(data.as_bytes());
            StorageValue::new_byte_array(hmac.finalize().into_bytes().as_slice())
        }
        "sha224" => {
            let mut hmac = Hmac::<Sha224>::new_from_slice(&key).map_err(ExecutionError::from)?;
            hmac.update(data.as_bytes());
            StorageValue::new_byte_array(hmac.finalize().into_bytes().as_slice())
        }
        _ => {
            return Err(ExecutionError::ParameterInvalid {
                name: "hash".to_owned(),
                message: "Invalid hash algorithm".to_owned(),
            })
        }
    };

    store_value(store_as, context, step_id, result).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn hmac_verify(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let hash =
        eval_opt_string_param("hash", parameters, context)?.unwrap_or_else(|| "sha512".to_owned());
    let data = eval_byte_array_param("data", parameters, context)?;
    let key = eval_byte_array_param("key", parameters, context)?;
    let tag = eval_byte_array_param("tag", parameters, context)?;

    match hash.as_str() {
        "sha512" => {
            let mut hmac = Hmac::<Sha512>::new_from_slice(&key).map_err(ExecutionError::from)?;
            hmac.update(data.as_bytes());
            match hmac.verify_slice(&tag) {
                Ok(_) => true,
                Err(_) => {
                    return Err(ExecutionError::ParameterInvalid {
                        name: "key".to_owned(),
                        message: "Invalid key".to_owned(),
                    })
                }
            }
        }
        "sha256" => {
            let mut hmac = Hmac::<Sha256>::new_from_slice(&key).map_err(ExecutionError::from)?;
            hmac.update(data.as_bytes());
            match hmac.verify_slice(&tag) {
                Ok(_) => true,
                Err(_) => {
                    return Err(ExecutionError::ParameterInvalid {
                        name: "key".to_owned(),
                        message: "Invalid key".to_owned(),
                    })
                }
            }
        }
        "sha384" => {
            let mut hmac = Hmac::<Sha384>::new_from_slice(&key).map_err(ExecutionError::from)?;
            hmac.update(data.as_bytes());
            match hmac.verify_slice(&tag) {
                Ok(_) => true,
                Err(_) => {
                    return Err(ExecutionError::ParameterInvalid {
                        name: "key".to_owned(),
                        message: "Invalid key".to_owned(),
                    })
                }
            }
        }
        "sha224" => {
            let mut hmac = Hmac::<Sha224>::new_from_slice(&key).map_err(ExecutionError::from)?;
            hmac.update(data.as_bytes());
            match hmac.verify_slice(&tag) {
                Ok(_) => true,
                Err(_) => {
                    return Err(ExecutionError::ParameterInvalid {
                        name: "key".to_owned(),
                        message: "Invalid key".to_owned(),
                    })
                }
            }
        }
        _ => {
            return Err(ExecutionError::ParameterInvalid {
                name: "hash".to_owned(),
                message: "Invalid hash algorithm".to_owned(),
            })
        }
    };

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
