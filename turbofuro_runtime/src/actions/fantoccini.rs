use fantoccini::{ClientBuilder, Locator};
use tel::{StorageValue, NULL};
use tracing::{debug, instrument};
use url::Url;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_opt_string_param, eval_string_param},
    executor::{ExecutionContext, Parameter},
    resources::{generate_resource_id, Resource, WebDriverClient, WebDriverElement},
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn get_client<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let connection_string = eval_opt_string_param("url", parameters, context)?
        .unwrap_or_else(|| "http://localhost:4444".to_owned());

    let url = Url::parse(&connection_string).map_err(|e| ExecutionError::ParameterInvalid {
        name: "connectionString".into(),
        message: e.to_string(),
    })?;

    let c = ClientBuilder::native()
        .connect(url.as_str())
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "connect".into(),
        })?;

    debug!("Created WebDriver client");

    let client_id = generate_resource_id();
    context
        .resources
        .add_webdriver_client(WebDriverClient(client_id, c));

    context
        .note_resource_provisioned(client_id, WebDriverClient::static_type())
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn execute<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let script = eval_string_param("script", parameters, context)?;

    let client = context
        .resources
        .use_webdriver_client()
        .ok_or_else(WebDriverClient::missing)?;

    let c = &mut client.1;
    let json = c
        .execute(&script, vec![])
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "execute_script".into(),
        })?;

    let client_id = client.0;
    context
        .note_resource_used(client_id, WebDriverClient::static_type())
        .await;

    let value: StorageValue =
        serde_json::from_value(json).map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "parse_script_output".into(),
        })?;

    store_value(store_as, context, step_id, value).await?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn goto<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let url = eval_string_param("url", parameters, context)?;

    let client = context
        .resources
        .use_webdriver_client()
        .ok_or_else(WebDriverClient::missing)?;

    let c = &mut client.1;
    c.goto(&url)
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "goto".into(),
        })?;

    let client_id = client.0;
    context
        .note_resource_used(client_id, WebDriverClient::static_type())
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn screenshot<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let element = &mut context
        .resources
        .use_webdriver_element()
        .ok_or_else(WebDriverElement::missing)?;

    let screenshot = element
        .1
        .screenshot()
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "screenshot".into(),
        })?;

    let element_id = element.0;
    context
        .note_resource_used(element_id, WebDriverElement::static_type())
        .await;

    store_value(
        store_as,
        context,
        step_id,
        StorageValue::new_byte_array(&screenshot),
    )
    .await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn click<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let element = &mut context
        .resources
        .use_webdriver_element()
        .ok_or_else(WebDriverElement::missing)?;

    element
        .1
        .click()
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "click".into(),
        })?;

    let element_id = element.0;
    context
        .note_resource_used(element_id, WebDriverElement::static_type())
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_text<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let element = &mut context
        .resources
        .use_webdriver_element()
        .ok_or_else(WebDriverElement::missing)?;

    let text = element
        .1
        .text()
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "get_text".into(),
        })?;

    let element_id = element.0;
    context
        .note_resource_used(element_id, WebDriverElement::static_type())
        .await;

    store_value(store_as, context, step_id, text.into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_html<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let element = &mut context
        .resources
        .use_webdriver_element()
        .ok_or_else(WebDriverElement::missing)?;

    let html = element
        .1
        .html(true)
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "get_html".into(),
        })?;

    let element_id = element.0;
    context
        .note_resource_used(element_id, WebDriverElement::static_type())
        .await;

    store_value(store_as, context, step_id, html.into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_attribute<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_string_param("name", parameters, context)?;
    let element = &mut context
        .resources
        .use_webdriver_element()
        .ok_or_else(WebDriverElement::missing)?;

    let attr = element
        .1
        .attr(name.as_str())
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "get_attribute".into(),
        })?;

    let element_id = element.0;
    context
        .note_resource_used(element_id, WebDriverElement::static_type())
        .await;

    store_value(
        store_as,
        context,
        step_id,
        attr.map(|s| s.into()).unwrap_or(NULL),
    )
    .await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_property<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let name = eval_string_param("name", parameters, context)?;
    let element = &mut context
        .resources
        .use_webdriver_element()
        .ok_or_else(WebDriverElement::missing)?;

    let prop = element
        .1
        .prop(name.as_str())
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "get_property".into(),
        })?;

    let element_id = element.0;
    context
        .note_resource_used(element_id, WebDriverElement::static_type())
        .await;

    store_value(
        store_as,
        context,
        step_id,
        prop.map(|s| s.into()).unwrap_or(NULL),
    )
    .await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn send_keys<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let input = eval_string_param("input", parameters, context)?;
    let element = &mut context
        .resources
        .use_webdriver_element()
        .ok_or_else(WebDriverElement::missing)?;

    element
        .1
        .send_keys(&input)
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "send_keys".into(),
        })?;

    let element_id = element.0;
    context
        .note_resource_used(element_id, WebDriverElement::static_type())
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_element<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let xpath = eval_opt_string_param("xpath", parameters, context)?;
    let css = eval_opt_string_param("css", parameters, context)?;

    let client = context
        .resources
        .use_webdriver_client()
        .ok_or_else(WebDriverClient::missing)?;
    let c = &mut client.1;

    let element =
        match (xpath, css) {
            (Some(xpath), None) => c.find(Locator::XPath(&xpath)).await.map_err(|e| {
                ExecutionError::WebDriverError {
                    message: e.to_string(),
                    stage: "find".into(),
                }
            })?,
            (None, Some(css)) => {
                c.find(Locator::Css(&css))
                    .await
                    .map_err(|e| ExecutionError::WebDriverError {
                        message: e.to_string(),
                        stage: "find".into(),
                    })?
            }
            (Some(_xpath), Some(_css)) => {
                return Err(ExecutionError::ParameterInvalid {
                    name: "locator".to_owned(),
                    message: "Can't use both xpath and css".to_owned(),
                })
            }
            (None, None) => {
                return Err(ExecutionError::ParameterInvalid {
                    name: "locator".to_owned(),
                    message: "Must provide either xpath or css".to_owned(),
                })
            }
        };

    let client_id = client.0;
    context
        .note_resource_used(client_id, WebDriverClient::static_type())
        .await;

    let element_id = generate_resource_id();
    context
        .resources
        .add_webdriver_element(WebDriverElement(element_id, element));

    context
        .note_resource_provisioned(element_id, WebDriverElement::static_type())
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_elements<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let xpath = eval_opt_string_param("xpath", parameters, context)?;
    let css = eval_opt_string_param("css", parameters, context)?;

    let client = context
        .resources
        .use_webdriver_client()
        .ok_or_else(WebDriverClient::missing)?;
    let c = &mut client.1;

    let elements =
        match (xpath, css) {
            (Some(xpath), None) => c.find_all(Locator::XPath(&xpath)).await.map_err(|e| {
                ExecutionError::WebDriverError {
                    message: e.to_string(),
                    stage: "find".into(),
                }
            })?,
            (None, Some(css)) => c.find_all(Locator::Css(&css)).await.map_err(|e| {
                ExecutionError::WebDriverError {
                    message: e.to_string(),
                    stage: "find".into(),
                }
            })?,
            (Some(_xpath), Some(_css)) => {
                return Err(ExecutionError::ParameterInvalid {
                    name: "locator".to_owned(),
                    message: "Can't use both xpath and css".to_owned(),
                })
            }
            (None, None) => {
                return Err(ExecutionError::ParameterInvalid {
                    name: "locator".to_owned(),
                    message: "Must provide either xpath or css".to_owned(),
                })
            }
        };

    let client_id = client.0;
    context
        .note_resource_used(client_id, WebDriverClient::static_type())
        .await;

    for element in elements {
        let element_id = generate_resource_id();
        context
            .resources
            .add_webdriver_element(WebDriverElement(element_id, element));

        context
            .note_resource_provisioned(element_id, WebDriverElement::static_type())
            .await;
    }

    Ok(())
}

pub async fn drop_element<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let removed = context.resources.pop_webdriver_element().ok_or_else(|| {
        ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "drop".to_owned(),
        }
    })?;

    context
        .note_resource_consumed(removed.0, removed.get_type())
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn select_option<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let label = eval_string_param("label", parameters, context)?;

    let element = &mut context
        .resources
        .use_webdriver_element()
        .ok_or_else(WebDriverElement::missing)?;

    element
        .1
        .select_by_label(&label)
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "send_keys".into(),
        })?;

    let element_id = element.0;
    context
        .note_resource_used(element_id, WebDriverElement::static_type())
        .await;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn drop_client<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let removed =
        context
            .resources
            .pop_webdriver_client()
            .ok_or_else(|| ExecutionError::WebDriverError {
                message: "No client found".to_owned(),
                stage: "drop".to_owned(),
            })?;

    context
        .note_resource_consumed(removed.0, removed.get_type())
        .await;

    Ok(())
}
