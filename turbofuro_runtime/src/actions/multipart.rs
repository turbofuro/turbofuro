use tel::{ObjectBody, StorageValue};
use tokio::sync::oneshot;
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    executor::{ExecutionContext, Parameter},
    resources::{
        MultipartManagerCommand, MultipartManagerFieldEvent, PendingFormData, PendingFormDataField,
        Resource,
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
    // Remove al the pending form data fields
    context
        .resources
        .pending_form_data_fields
        .retain(|f| f.0.is_empty());

    let form_data = context
        .resources
        .pending_form_data
        .last()
        .ok_or_else(PendingFormData::missing)?;

    let (sender, receiver) = oneshot::channel::<MultipartManagerFieldEvent>();
    form_data
        .0
        .send(MultipartManagerCommand::GetNext { sender })
        .await
        .unwrap();

    let field = receiver.await.unwrap();
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
            context
                .resources
                .pending_form_data_fields
                .push(PendingFormDataField(receiver));

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
