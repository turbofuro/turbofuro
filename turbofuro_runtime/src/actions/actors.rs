use crate::{
    actions::{as_string, get_handlers_from_parameters},
    actor::{activate_actor, Actor, ActorCommand},
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param, eval_saver_param},
    executor::{ExecutionContext, Parameter},
    resources::{ActorLink, ActorResources, Resource},
};
use tel::{ObjectBody, StorageValue};
use tracing::{debug, instrument};

fn actor_not_found() -> ExecutionError {
    ExecutionError::MissingResource {
        resource_type: ActorLink::get_type().into(),
    }
}

#[instrument(level = "trace", skip_all)]
pub async fn check_actor_exists<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let id_param = eval_param("id", parameters, &context.storage, &context.environment)?;
    let id = as_string(id_param, "id")?;

    let exists = { context.global.registry.actors.contains_key(&id) };
    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;
    context.add_to_storage(step_id, selector, exists.into())?;
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn spawn_actor<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let state_param = eval_optional_param_with_default(
        "state",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Null(None),
    )?;

    let handlers = get_handlers_from_parameters(parameters);

    let actor = Actor::new(
        state_param,
        context.environment.clone(),
        context.module.clone(),
        context.global.clone(),
        ActorResources::default(),
        handlers,
    );
    let id = actor.get_id().to_owned();

    debug!("Spawning actor id: {}, module: {}", id, context.module.id);

    let sender = activate_actor(actor);

    context
        .global
        .registry
        .actors
        .insert(id.clone(), ActorLink(sender));

    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;

    context.add_to_storage(step_id, selector, id.into())?;

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn send_command<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let id_param = eval_param("id", parameters, &context.storage, &context.environment)?;
    let id = as_string(id_param, "id")?;

    let messenger = {
        context
            .global
            .registry
            .actors
            .get(&id)
            .ok_or_else(actor_not_found)
            .map(|r| r.value().0.clone())?
    };

    let message_param = eval_param(
        "message",
        parameters,
        &context.storage,
        &context.environment,
    )?;

    let mut storage = ObjectBody::new();
    storage.insert("message".to_owned(), message_param);

    // TODO: Wait for run to finish?
    // let (sender, receiver) = oneshot::channel();
    messenger
        .send(ActorCommand::Run {
            handler: "onMessage".to_owned(),
            storage,
            sender: None,
        })
        .await
        .expect("Failed to send message to actor");
    // let _ = receiver.await.expect("Actor did not respond");

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn terminate<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
) -> Result<(), ExecutionError> {
    let id_param = eval_optional_param_with_default(
        "id",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::String(context.actor_id.clone()),
    )?;
    let id = as_string(id_param, "id")?;

    let messenger = {
        context
            .global
            .registry
            .actors
            .get(&id)
            .ok_or_else(actor_not_found)
            .map(|r| r.value().0.clone())?
    };

    messenger.send(ActorCommand::Terminate).await.unwrap();
    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn get_actor_id<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    step_id: &str,
) -> Result<(), ExecutionError> {
    let selector = eval_saver_param(
        "saveAs",
        parameters,
        &mut context.storage,
        &context.environment,
    )?;

    context.add_to_storage(step_id, selector, context.actor_id.clone().into())?;

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

        let result =
            get_actor_id(&mut context, &vec![Parameter::tel("saveAs", "id")], "test").await;

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

        spawn_actor(&mut context, &vec![Parameter::tel("saveAs", "id")], "test")
            .await
            .unwrap();

        let actor_id = eval("id", &context.storage, &context.environment);
        assert!(matches!(actor_id, Ok(StorageValue::String(_))));

        check_actor_exists(
            &mut context,
            &vec![
                Parameter::tel("id", "id"),
                Parameter::tel("saveAs", "exists"),
            ],
            "test",
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
            &vec![
                Parameter::tel("id", "id"),
                Parameter::tel("saveAs", "exists"),
            ],
            "test",
        )
        .await
        .unwrap();
        assert_eq!(
            eval("exists", &context.storage, &context.environment),
            Ok(StorageValue::Boolean(false))
        );
    }
}
