use fantoccini::{ClientBuilder, Locator};
use tel::{StorageValue, NULL};
use tracing::{debug, instrument};
use url::Url;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_opt_string_param, eval_string_param},
    executor::{ExecutionContext, Parameter},
    resources::{Resource, WebDriverClient, WebDriverElement},
};

use super::store_value;

// #[instrument(level = "trace", skip_all)]
// pub async fn get_connection<'a>(
//     context: &mut ExecutionContext<'a>,
//     parameters: &Vec<Parameter>,
//     _step_id: &str,
// ) -> Result<(), ExecutionError> {
//     let connection_string = eval_string_param("connectionString", parameters, context)?;
//     let name = eval_opt_string_param("name", parameters, context)?.unwrap_or("default".to_owned());

//     // Check if we already have a connection pool with this name
//     let exists = { context.global.registry.webdriver_pools.contains_key(&name) };
//     if exists {
//         return Ok(());
//     }

//     let url = Url::parse(&connection_string).map_err(|e| ExecutionError::ParameterInvalid {
//         name: "connectionString".into(),
//         message: e.to_string(),
//     })?;

//     let manager = Manager::new("http://localhost:4444", ClientBuilder::native());
//     let pool = Pool::builder(manager).max_size(5).build().unwrap();

//     debug!("Created WebDriver connection pool: {}", name);

//     // Put to the registry
//     context
//         .global
//         .registry
//         .webdriver_pools
//         .insert(name, WebDriverPool(pool));

//     Ok(())
// }

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

    context.resources.webdriver_clients.push(WebDriverClient(c));

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
    // let params = eval_a("script", parameters, context)?;

    // Retrieve the connection pool
    let client = context
        .resources
        .webdriver_clients
        .last_mut()
        .ok_or_else(WebDriverClient::missing)?;

    let c = &mut client.0;
    let json = c
        .execute(&script, vec![])
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "execute_script".into(),
        })?;

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

    // Retrieve the connection pool
    let client = context
        .resources
        .webdriver_clients
        .last_mut()
        .ok_or_else(WebDriverClient::missing)?;

    let c = &mut client.0;
    c.goto(&url)
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "goto".into(),
        })?;

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
        .webdriver_elements
        .last_mut()
        .ok_or_else(|| ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "get_element".into(),
        })?
        .0;

    let screenshot = element
        .screenshot()
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "screenshot".into(),
        })?;

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
        .webdriver_elements
        .last_mut()
        .ok_or_else(|| ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "get_element".into(),
        })?
        .0;

    element
        .click()
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "click".into(),
        })?;

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
        .webdriver_elements
        .last_mut()
        .ok_or_else(|| ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "get_element".into(),
        })?
        .0;

    let text = element
        .text()
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "get_text".into(),
        })?;

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
        .webdriver_elements
        .last_mut()
        .ok_or_else(|| ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "get_element".into(),
        })?
        .0;

    let html = element
        .html(true)
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "get_html".into(),
        })?;

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
        .webdriver_elements
        .last_mut()
        .ok_or_else(|| ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "get_element".into(),
        })?
        .0;

    let attr = element
        .attr(name.as_str())
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "get_attribute".into(),
        })?;

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
        .webdriver_elements
        .last_mut()
        .ok_or_else(|| ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "get_element".into(),
        })?
        .0;

    let prop = element
        .prop(name.as_str())
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "get_property".into(),
        })?;

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
        .webdriver_elements
        .last_mut()
        .ok_or_else(|| ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "get_element".into(),
        })?
        .0;

    element
        .send_keys(&input)
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "send_keys".into(),
        })?;

    Ok(())
}

// async fn get_element<'a>(
//     context: &mut ExecutionContext<'a>,
//     parameters: &Vec<Parameter>,
//     step_id: &str,
//     store_as: Option<&str>,
// ) -> Result<Element, ExecutionError> {
//     let xpath = eval_opt_string_param("xpath", parameters, context)?;
//     let css = eval_opt_string_param("css", parameters, context)?;

//     // Retrieve the connection pool
//     let client = context
//         .resources
//         .webdriver_clients
//         .last_mut()
//         .ok_or_else(WebDriverClient::missing)?;
//     let c = &mut client.0;

//     let element =
//         match (xpath, css) {
//             (Some(xpath), None) => c.find(Locator::XPath(&xpath)).await.map_err(|e| {
//                 ExecutionError::WebDriverError {
//                     message: e.to_string(),
//                     stage: "find".into(),
//                 }
//             })?,
//             (None, Some(css)) => {
//                 c.find(Locator::Css(&css))
//                     .await
//                     .map_err(|e| ExecutionError::WebDriverError {
//                         message: e.to_string(),
//                         stage: "find".into(),
//                     })?
//             }
//             (Some(_xpath), Some(_css)) => {
//                 return Err(ExecutionError::ParameterInvalid {
//                     name: "locator".to_owned(),
//                     message: "Can't use both xpath and css".to_owned(),
//                 })
//             }
//             (None, None) => {
//                 return Err(ExecutionError::ParameterInvalid {
//                     name: "locator".to_owned(),
//                     message: "Must provide either xpath or css".to_owned(),
//                 })
//             }
//         };

//     Ok(element)
// }

#[instrument(level = "trace", skip_all)]
pub async fn get_element<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let xpath = eval_opt_string_param("xpath", parameters, context)?;
    let css = eval_opt_string_param("css", parameters, context)?;

    // Retrieve the connection pool
    let client = context
        .resources
        .webdriver_clients
        .last_mut()
        .ok_or_else(WebDriverClient::missing)?;
    let c = &mut client.0;

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

    context
        .resources
        .webdriver_elements
        .push(WebDriverElement(element));

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

    // Retrieve the connection pool
    let client = context
        .resources
        .webdriver_clients
        .last_mut()
        .ok_or_else(WebDriverClient::missing)?;
    let c = &mut client.0;

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

    context
        .resources
        .webdriver_elements
        .append(&mut elements.into_iter().map(WebDriverElement).collect());

    Ok(())
}

pub async fn drop_element<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let removed = &mut context.resources.webdriver_elements.pop();
    store_value(store_as, context, step_id, removed.is_some().into()).await?;

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
        .webdriver_elements
        .last_mut()
        .ok_or_else(|| ExecutionError::WebDriverError {
            message: "No element found".to_owned(),
            stage: "get_element".into(),
        })?
        .0;

    element
        .select_by_label(&label)
        .await
        .map_err(|e| ExecutionError::WebDriverError {
            message: e.to_string(),
            stage: "send_keys".into(),
        })?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn drop_client<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let removed = context.resources.webdriver_clients.pop();
    store_value(store_as, context, step_id, removed.is_some().into()).await?;

    Ok(())
}
