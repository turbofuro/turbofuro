use std::{collections::HashMap, pin::Pin};

use axum::body::Bytes;
use futures_util::Stream;
use reqwest::multipart::{Form, Part};
use tel::{describe, Description, StorageValue};
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param, eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
    resources::{FormDataDraft, Resource},
};

use super::{as_string, as_u64};

pub fn form_data_draft_resource_not_found() -> ExecutionError {
    ExecutionError::MissingResource {
        resource_type: FormDataDraft::get_type().into(),
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn create_form_data<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let fields_param = eval_optional_param_with_default(
        "fields",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Object(HashMap::new()),
    )?;

    let mut form: Form = Form::new();

    match fields_param {
        StorageValue::Object(object) => {
            for (key, value) in object {
                let value = value.to_string().map_err(ExecutionError::from)?;
                form = form.text(key, value)
            }
        }
        s => {
            return Err(ExecutionError::ParameterTypeMismatch {
                name: "headers".to_owned(),
                expected: Description::new_union(vec![
                    Description::new_base_type("array"),
                    Description::new_base_type("object"),
                ]),
                actual: describe(s),
            })
        }
    }

    context.resources.form_data.push(FormDataDraft(form));

    Ok(())
}

type HammerStream = Pin<Box<dyn Stream<Item = Result<Bytes, ExecutionError>> + Send + Sync>>;

struct StreamPart(HammerStream);

impl StreamPart {
    fn new(stream: HammerStream) -> Self {
        Self(stream)
    }
}

impl From<StreamPart> for reqwest::Body {
    fn from(val: StreamPart) -> Self {
        reqwest::Body::wrap_stream(val.0)
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn add_stream_field_to_form_data<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name_param = eval_param("name", parameters, &context.storage, &context.environment)?;
    let name = as_string(name_param, "name")?;

    let size_param =
        eval_optional_param("size", parameters, &context.storage, &context.environment)?;

    let filename_param = eval_optional_param(
        "filename",
        parameters,
        &context.storage,
        &context.environment,
    )?;

    let mime_param =
        eval_optional_param("mime", parameters, &context.storage, &context.environment)?;

    // Get form data from resources
    let mut form_data = context
        .resources
        .form_data
        .pop()
        .ok_or(form_data_draft_resource_not_found())?;

    let stream = context.resources.get_nearest_stream()?;

    let mut part: Part = {
        if let Some(size_param) = size_param {
            let size = as_u64(size_param, "size")?;
            Part::stream_with_length(StreamPart::new(stream), size)
        } else {
            Part::stream(StreamPart::new(stream))
        }
    };

    if let Some(filename) = filename_param {
        let filename = as_string(filename, "filename")?;
        part = part.file_name(filename);
    }

    if let Some(mime) = mime_param {
        let mime = as_string(mime, "mime")?;
        part = part
            .mime_str(&mime)
            .map_err(|e| ExecutionError::ParameterInvalid {
                name: "mime".to_string(),
                message: e.to_string(),
            })?;
    }

    form_data.0 = form_data.0.part(name, part);

    // Store the form data back to resources
    context.resources.form_data.push(form_data);

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn add_field_to_form_data<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name_param = eval_param("name", parameters, &context.storage, &context.environment)?;
    let name = as_string(name_param, "name")?;

    let value_param = eval_param("value", parameters, &context.storage, &context.environment)?;
    let value = as_string(value_param, "value")?;

    // Get form data from resources
    let mut form_data = context
        .resources
        .form_data
        .pop()
        .ok_or(form_data_draft_resource_not_found())?;

    form_data.0 = form_data.0.text(name, value);

    // Store the form data back to resources
    context.resources.form_data.push(form_data);

    Ok(())
}
