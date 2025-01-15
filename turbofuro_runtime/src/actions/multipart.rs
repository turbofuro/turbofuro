use tel::{ObjectBody, StorageValue};
use tokio::sync::oneshot;
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    executor::{ExecutionContext, Parameter},
    resources::{
        generate_resource_id, MultipartManagerCommand, MultipartManagerFieldEvent, PendingFormData,
        PendingFormDataField, Resource,
    },
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn get_field<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let form_data = context
        .resources
        .use_pending_form_data()
        .ok_or_else(PendingFormData::missing)?;

    let (sender, receiver) = oneshot::channel::<MultipartManagerFieldEvent>();
    form_data
        .1
        .send(MultipartManagerCommand::GetNext { sender })
        .await
        .map_err(|_| ExecutionError::StateInvalid {
            message: "Failed to send get next field command to pending form data".to_owned(),
            subject: PendingFormData::static_type().into(),
            inner: "Send error".to_owned(),
        })?;
    let form_data_id = form_data.0;

    let field = receiver.await.map_err(|_| ExecutionError::StateInvalid {
        message: "Failed to receive field from pending form data".to_owned(),
        subject: PendingFormData::static_type().into(),
        inner: "Receive error".to_owned(),
    })?;

    context
        .note_resource_used(form_data_id, PendingFormData::static_type())
        .await;

    let storage = match field {
        MultipartManagerFieldEvent::Error => {
            let mut storage = ObjectBody::new();
            storage.insert("type".to_owned(), "error".into());
            storage
        }
        MultipartManagerFieldEvent::Empty => {
            let mut storage = ObjectBody::new();
            storage.insert("type".to_owned(), "empty".into());
            storage
        }
        MultipartManagerFieldEvent::File {
            name,
            filename,
            receiver,
            index,
            headers,
        } => {
            let field_id = generate_resource_id();
            context
                .resources
                .add_pending_form_data_field(PendingFormDataField(
                    generate_resource_id(),
                    receiver,
                ));
            context
                .note_resource_provisioned(field_id, PendingFormDataField::static_type())
                .await;

            let mut storage = ObjectBody::new();
            if let Some(name) = name {
                storage.insert("name".to_owned(), name.into());
            }
            if let Some(filename) = filename {
                storage.insert("filename".to_owned(), filename.into());
            }
            storage.insert("index".to_owned(), index.into());
            storage.insert("type".to_owned(), "file".into());

            let mut headers_object = ObjectBody::new();
            for (k, v) in headers {
                headers_object.insert(k, v.into());
            }
            storage.insert("headers".to_owned(), StorageValue::Object(headers_object));

            storage
        }
    };
    store_value(store_as, context, step_id, StorageValue::Object(storage)).await?;

    Ok(())
}
