use std::pin::Pin;

use axum::body::Bytes;
use futures_util::Stream;
use reqwest::multipart::{Form, Part};
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_opt_string_param, eval_opt_u64_param, eval_string_param},
    executor::{ExecutionContext, Parameter},
    resources::{generate_resource_id, FormDataDraft, Resource},
};

#[instrument(level = "trace", skip_all)]
pub fn create_form_data<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let form: Form = Form::new();
    context
        .resources
        .add_form_data(FormDataDraft(generate_resource_id(), form));
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
pub async fn add_stream_part_to_form_data<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_string_param("name", parameters, context)?;
    let size_param = eval_opt_u64_param("size", parameters, context)?;
    let filename_param = eval_opt_string_param("filename", parameters, context)?;
    let mime_param = eval_opt_string_param("mime", parameters, context)?;

    // Get form data from resources
    let mut form_data = context
        .resources
        .pop_form_data()
        .ok_or_else(FormDataDraft::missing)?;

    // The resource is put back later
    // The use is noted later

    let (stream, metadata) = context.resources.get_stream()?;

    context
        .note_resource_consumed(metadata.id, metadata.type_)
        .await;

    let mut part: Part = {
        if let Some(size) = size_param {
            Part::stream_with_length(StreamPart::new(stream), size)
        } else {
            Part::stream(StreamPart::new(stream))
        }
    };

    if let Some(filename) = filename_param {
        part = part.file_name(filename);
    }

    if let Some(mime) = mime_param {
        part = part
            .mime_str(&mime)
            .map_err(|e| ExecutionError::ParameterInvalid {
                name: "mime".to_string(),
                message: e.to_string(),
            })?;
    }

    form_data.1 = form_data.1.part(name, part);

    context
        .note_resource_used(form_data.0, form_data.get_type())
        .await;

    // Store the form data back to resources
    context.resources.add_form_data(form_data);

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn add_text_part_to_form_data<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_string_param("name", parameters, context)?;
    let value = eval_string_param("value", parameters, context)?; // TODO: Add support for bytes?
    let filename = eval_opt_string_param("filename", parameters, context)?;
    let mime = eval_opt_string_param("mime", parameters, context)?;

    // Get form data from resources
    let mut form_data = context
        .resources
        .pop_form_data()
        .ok_or_else(FormDataDraft::missing)?;

    // The resource is put back later
    // The use is noted later

    let mut part: Part = Part::text(value);

    if let Some(filename) = filename {
        part = part.file_name(filename);
    }

    if let Some(mime) = mime {
        part = part
            .mime_str(&mime)
            .map_err(|e| ExecutionError::ParameterInvalid {
                name: "mime".to_string(),
                message: e.to_string(),
            })?;
    }

    form_data.1 = form_data.1.part(name, part);

    context
        .note_resource_used(form_data.0, form_data.get_type())
        .await;

    // Store the form data back to resources
    context.resources.add_form_data(form_data);

    Ok(())
}
