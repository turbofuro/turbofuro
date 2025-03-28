use std::{time::Duration, vec};

use ollama_rs::{
    generation::{completion::request::GenerationRequest, images::Image},
    Ollama,
};
use tel::StorageValue;
use tracing::instrument;
use url::Url;

use crate::{
    errors::ExecutionError,
    evaluations::{
        eval_opt_string_param, eval_opt_u64_param, eval_optional_param, eval_string_param,
    },
    executor::{ExecutionContext, Parameter},
};

use super::store_value;

static DEFAULT_TIMEOUT: u64 = 60_000;

#[instrument(level = "trace", skip_all)]
pub async fn generate(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let url = eval_opt_string_param("url", parameters, context)?
        .unwrap_or_else(|| "http://localhost:11434".to_owned());
    let timeout = eval_opt_u64_param("timeout", parameters, context)?.unwrap_or(DEFAULT_TIMEOUT);

    let url = url
        .parse::<Url>()
        .map_err(|e| ExecutionError::ParameterInvalid {
            name: "url".to_owned(),
            message: e.to_string(),
        })?;

    // TODO: Reuse client, or actually a underlying HTTP client from reqwest/http_client module
    // Currently the field is private and we can't provide a custom client
    let ollama = Ollama::from_url(url);

    let model_name = eval_string_param("model", parameters, context)?;
    let prompt = eval_string_param("prompt", parameters, context)?;

    let images = eval_optional_param("images", parameters, context)?;
    let images = match images {
        Some(images) => match images {
            StorageValue::Array(arr) => {
                let mut images = vec![];
                for (i, image) in arr.into_iter().enumerate() {
                    match image {
                        StorageValue::String(s) => {
                            images.push(Image::from_base64(&s));
                        }
                        _ => {
                            return Err(ExecutionError::ParameterInvalid {
                                name: format!("images[{}]", i),
                                message: "Images must be an array of base64 encoded images"
                                    .to_owned(),
                            })
                        }
                    }
                }
                images
            }
            _ => {
                return Err(ExecutionError::ParameterInvalid {
                    name: "images".to_owned(),
                    message: "Images must be an array of base64 encoded images".to_owned(),
                })
            }
        },
        None => vec![],
    };

    let mut request = GenerationRequest::new(model_name, prompt);
    request = request.timeout(Duration::from_millis(timeout));
    for image in images {
        request = request.add_image(image);
    }

    let response =
        Ollama::generate(&ollama, request)
            .await
            .map_err(|e| ExecutionError::StateInvalid {
                message: "Could not generate".to_owned(),
                subject: "ollama".to_owned(),
                inner: e.to_string(),
            })?;

    store_value(store_as, context, step_id, response.response.into()).await?;
    Ok(())
}
