use std::{collections::HashMap, time::Duration};

use crate::{
    actor::{activate_actor, Actor, ActorCommand},
    errors::ExecutionError,
    evaluations::{
        eval_opt_string_param, eval_opt_u64_param, eval_optional_param_with_default, eval_param,
        eval_string_param, get_handlers_from_parameters,
    },
    executor::{ExecutionContext, Parameter},
    resources::{ActorLink, ActorResources, Resource},
};
use tel::{ObjectBody, StorageValue};
use tokio::{sync::oneshot, time::timeout};
use tracing::{debug, instrument};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn check_actor_exists<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let id = eval_string_param("id", parameters, context)?;

    let exists = { context.global.registry.actors.contains_key(&id) };
    store_value(store_as, context, step_id, exists.into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn spawn_actor<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let state_param =
        eval_optional_param_with_default("state", parameters, context, StorageValue::Null(None))?;

    let handlers = get_handlers_from_parameters(parameters);

    let debugger = context
        .global
        .debug_state
        .load()
        .get_debugger(&context.module.id);

    let actor = Actor::new(
        state_param,
        context.environment.clone(),
        context.module.clone(),
        context.global.clone(),
        ActorResources::default(),
        handlers,
        debugger,
    );
    let id = actor.get_id().to_owned();

    debug!("Spawning actor id: {}, module: {}", id, context.module.id);

    let actor_link = activate_actor(actor);

    context
        .global
        .registry
        .actors
        .insert(id.clone(), actor_link);

    store_value(store_as, context, step_id, id.into()).await?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn send<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let id = eval_string_param("id", parameters, context)?;

    let messenger = {
        context
            .global
            .registry
            .actors
            .get(&id)
            .ok_or_else(ActorLink::missing)
            .map(|r| r.value().clone())?
    };

    let message_param = eval_param("message", parameters, context)?;

    let mut storage = ObjectBody::new();
    storage.insert("message".to_owned(), message_param);

    // This is fire and forget
    messenger
        .send(ActorCommand::Run {
            handler: "onMessage".to_owned(),
            storage,
            references: HashMap::new(),
            sender: None,
            execution_id: None,
        })
        .await
        .expect("Failed to send message to actor");

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn request<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let id = eval_string_param("id", parameters, context)?;
    let timeout_millis = eval_opt_u64_param("timeout", parameters, context)?.unwrap_or(180_000); // 3 minutes

    let messenger = {
        context
            .global
            .registry
            .actors
            .get(&id)
            .ok_or_else(ActorLink::missing)
            .map(|r| r.value().clone())?
    };

    let message_param = eval_param("message", parameters, context)?;

    let mut storage = ObjectBody::new();
    storage.insert("message".to_owned(), message_param);

    // Let's wait for run to finish
    let (sender, receiver) = oneshot::channel();
    messenger
        .send(ActorCommand::Run {
            handler: "onRequest".to_owned(),
            storage,
            references: HashMap::new(),
            sender: Some(sender),
            execution_id: None,
        })
        .await
        .map_err(|e| ExecutionError::ActorCommandFailed {
            message: format!(
                "Could not send run command (handler: onRequest) to actor. Send error: {:?}",
                e
            ),
        })?;

    match timeout(Duration::from_millis(timeout_millis), receiver).await {
        Ok(response) => match response {
            Ok(response) => {
                let response = response?;
                store_value(store_as, context, step_id, response).await?;
            }
            Err(e) => {
                return Err(ExecutionError::ActorCommandFailed {
                    message: format!("Actor did not respond: {}", e),
                })
            }
        },
        Err(_) => {
            return Err(ExecutionError::ActorCommandFailed {
                message: "Timeout while waiting for actor response".to_owned(),
            })
        }
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn terminate<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let id = eval_opt_string_param("id", parameters, context)?
        .unwrap_or(context.actor_id.clone().into());

    let messenger = {
        context
            .global
            .registry
            .actors
            .get(&id)
            .ok_or_else(ActorLink::missing)
            .map(|r| r.value().clone())?
    };

    messenger.send(ActorCommand::Terminate).await.unwrap();
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_actor_id<'a>(
    context: &mut ExecutionContext<'a>,
    _parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    store_value(store_as, context, step_id, context.actor_id.clone().into()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

    #[tokio::test]
    async fn test_get_actor_id() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        let result = get_actor_id(&mut context, &vec![], "test", Some("id")).await;

        assert!(result.is_ok());
        assert_eq!(
            eval("id", &context.storage, &context.environment).unwrap(),
            context.actor_id.to_owned().into()
        );
    }

    #[tokio::test]
    async fn test_actor_spawning_check_terminate() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        spawn_actor(&mut context, &vec![], "test", Some("id"))
            .await
            .unwrap();

        let actor_id = eval("id", &context.storage, &context.environment);
        assert!(matches!(actor_id, Ok(StorageValue::String(_))));

        check_actor_exists(
            &mut context,
            &vec![Parameter::tel("id", "id")],
            "test",
            Some("exists"),
        )
        .await
        .unwrap();
        assert_eq!(
            eval("exists", &context.storage, &context.environment),
            Ok(StorageValue::Boolean(true))
        );

        terminate(&mut context, &vec![Parameter::tel("id", "id")], "test")
            .await
            .unwrap();

        // Wait for actor to terminate
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        check_actor_exists(
            &mut context,
            &vec![Parameter::tel("id", "id")],
            "test",
            Some("exists"),
        )
        .await
        .unwrap();
        assert_eq!(
            eval("exists", &context.storage, &context.environment),
            Ok(StorageValue::Boolean(false))
        );
    }
}
