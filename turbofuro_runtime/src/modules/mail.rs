use std::{collections::HashMap, time::Duration};

use crate::{
    errors::ExecutionError,
    evaluations::{
        as_string_or_array_string, eval_opt_string_param, eval_optional_param, eval_param,
        eval_string_param,
    },
    executor::{ExecutionContext, Parameter},
};
use lettre::{
    address::AddressError,
    message::{Mailbox, MessageBuilder, MultiPart, SinglePart},
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
};
use tel::StorageValue;
use tracing::instrument;

use super::store_value;

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
    let text_param = eval_opt_string_param("text", parameters, context)?;
    let html = eval_string_param("html", parameters, context)?;

    match text_param {
        Some(text) => {
            sendmail_smtp(
                context,
                parameters,
                step_id,
                store_as,
                Payload::MultiPart(MultiPart::alternative_plain_html(text, html)),
            )
            .await
        }
        None => {
            sendmail_smtp(
                context,
                parameters,
                step_id,
                store_as,
                Payload::SinglePart(SinglePart::html(html)),
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
    let text = eval_string_param("text", parameters, context)?;

    sendmail_smtp(
        context,
        parameters,
        step_id,
        store_as,
        Payload::SinglePart(SinglePart::plain(text)),
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
    let from = eval_string_param("from", parameters, context)?;
    let from: Mailbox =
        from.parse()
            .map_err(|e: AddressError| ExecutionError::ParameterInvalid {
                name: "from".to_owned(),
                message: e.to_string(),
            })?;
    let subject = eval_string_param("subject", parameters, context)?;
    let mut message_builder = MessageBuilder::new().from(from).subject(subject);

    let to_param = eval_param("to", parameters, context)?;
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

    let connection_url = eval_string_param("connectionUrl", parameters, context)?;

    let reply_to_param = eval_optional_param("replyTo", parameters, context)?;

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

    let bcc_param = eval_optional_param("bcc", parameters, context)?;
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

    let cc_param = eval_optional_param("cc", parameters, context)?;
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

    let mailer = AsyncSmtpTransport::<Tokio1Executor>::from_url(&connection_url)
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
