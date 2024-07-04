use std::{collections::HashMap, time::Duration};

use crate::{
    actions::as_string,
    errors::ExecutionError,
    evaluations::{eval_optional_param, eval_param},
    executor::{ExecutionContext, Parameter},
};
use lettre::{
    address::AddressError,
    message::{Mailbox, MessageBuilder, MultiPart, SinglePart},
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
};
use tel::StorageValue;
use tracing::instrument;

use super::{as_string_or_array_string, store_value};

#[derive(Debug)]
enum Payload {
    SinglePart(SinglePart),
    MultiPart(MultiPart),
}

#[instrument(level = "trace", skip_all)]
pub async fn sendmail_smtp_html<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let text_param =
        eval_optional_param("text", parameters, &context.storage, &context.environment)?;

    let html_param = eval_param("html", parameters, &context.storage, &context.environment)?;
    let html_param = as_string(html_param, "html")?;

    match text_param {
        Some(text_param) => {
            let text_param = as_string(text_param, "text")?;
            sendmail_smtp(
                context,
                parameters,
                step_id,
                store_as,
                Payload::MultiPart(MultiPart::alternative_plain_html(text_param, html_param)),
            )
            .await
        }
        None => {
            sendmail_smtp(
                context,
                parameters,
                step_id,
                store_as,
                Payload::SinglePart(SinglePart::html(html_param)),
            )
            .await
        }
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn sendmail_smtp_text<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let text_param = eval_param("text", parameters, &context.storage, &context.environment)?;
    let text_param = as_string(text_param, "text")?;

    sendmail_smtp(
        context,
        parameters,
        step_id,
        store_as,
        Payload::SinglePart(SinglePart::plain(text_param)),
    )
    .await
}

async fn sendmail_smtp<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
    payload: Payload,
) -> Result<(), ExecutionError> {
    let from_param = eval_param("from", parameters, &context.storage, &context.environment)?;
    let from_param = as_string(from_param, "from")?;
    let from: Mailbox =
        from_param
            .parse()
            .map_err(|e: AddressError| ExecutionError::ParameterInvalid {
                name: "from".to_owned(),
                message: e.to_string(),
            })?;

    let subject_param = eval_param(
        "subject",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let subject_param = as_string(subject_param, "subject")?;

    let mut message_builder = MessageBuilder::new().from(from).subject(subject_param);

    let to_param = eval_param("to", parameters, &context.storage, &context.environment)?;
    let to_param: Vec<String> = as_string_or_array_string(to_param, "to")?;
    for to_param in to_param {
        let to: Mailbox =
            to_param
                .parse()
                .map_err(|e: AddressError| ExecutionError::ParameterInvalid {
                    name: "to".to_owned(),
                    message: e.to_string(),
                })?;
        message_builder = message_builder.to(to)
    }

    let connection_param = eval_param(
        "connectionUrl",
        parameters,
        &context.storage,
        &context.environment,
    )?;
    let connection_param = as_string(connection_param, "connectionUrl")?;

    let reply_to_param = eval_optional_param(
        "replyTo",
        parameters,
        &context.storage,
        &context.environment,
    )?;

    if let Some(reply_to) = reply_to_param {
        let reply_to = as_string_or_array_string(reply_to, "replyTo")?;
        for reply_to in reply_to {
            let reply_to: Mailbox =
                reply_to
                    .parse()
                    .map_err(|e: AddressError| ExecutionError::ParameterInvalid {
                        name: "replyTo".to_owned(),
                        message: e.to_string(),
                    })?;
            message_builder = message_builder.reply_to(reply_to)
        }
    }

    let bcc_param = eval_optional_param("bcc", parameters, &context.storage, &context.environment)?;
    if let Some(bcc) = bcc_param {
        let bcc = as_string_or_array_string(bcc, "bcc")?;
        for bcc in bcc {
            let bcc: Mailbox =
                bcc.parse()
                    .map_err(|e: AddressError| ExecutionError::ParameterInvalid {
                        name: "bcc".to_owned(),
                        message: e.to_string(),
                    })?;
            message_builder = message_builder.bcc(bcc)
        }
    }

    let cc_param = eval_optional_param("cc", parameters, &context.storage, &context.environment)?;
    if let Some(cc) = cc_param {
        let cc = as_string_or_array_string(cc, "cc")?;
        for cc in cc {
            let cc: Mailbox =
                cc.parse()
                    .map_err(|e: AddressError| ExecutionError::ParameterInvalid {
                        name: "cc".to_owned(),
                        message: e.to_string(),
                    })?;
            message_builder = message_builder.cc(cc)
        }
    }

    let mailer = AsyncSmtpTransport::<Tokio1Executor>::from_url(&connection_param)
        .map_err(|e| ExecutionError::ParameterInvalid {
            name: "connectionUrl".to_owned(),
            message: e.to_string(),
        })?
        .timeout(Some(Duration::from_secs(30)))
        .build();

    let message = match payload {
        Payload::SinglePart(singlepart) => message_builder.singlepart(singlepart),
        Payload::MultiPart(multipart) => message_builder.multipart(multipart),
    }
    .map_err(|e| ExecutionError::Unknown {
        message: e.to_string(),
    })?;

    let response = mailer
        .send(message)
        .await
        .map_err(|e| ExecutionError::Unknown {
            message: e.to_string(),
        })?;

    let mut data: HashMap<String, StorageValue> = HashMap::new();
    data.insert("code".to_owned(), response.code().to_string().into());
    data.insert(
        "message".to_owned(),
        response.message().collect::<String>().into(),
    );

    store_value(store_as, context, step_id, StorageValue::Object(data)).await?;

    Ok(())
}
