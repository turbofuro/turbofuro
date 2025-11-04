use mlua::{Function, Lua, LuaSerdeExt, Value};
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param, eval_string_param},
    executor::{ExecutionContext, Parameter},
};

use super::store_value;

#[instrument(level = "trace", skip_all)]
pub async fn run_function(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let code: String = eval_string_param("code", parameters, context)?;
    let lua = Lua::new();

    let value = eval_optional_param("value", parameters, context)?
        .map(|v| lua.to_value(&v))
        .transpose()
        .map_err(|_e| ExecutionError::ParameterInvalid {
            name: "value".to_owned(),
            message: "Could not convert value to Lua value".to_owned(),
        })?;

    let handler = lua
        .load(&code)
        .eval::<Function>()
        .map_err(|e| ExecutionError::StateInvalid {
            message: "Could not load Lua code".to_owned(),
            subject: "lua".to_owned(),
            inner: e.to_string(),
        })?;

    match handler.call_async::<Value>(value).await {
        Ok(lua_resp) => {
            let output = lua
                .from_value(lua_resp)
                .map_err(|e| ExecutionError::StateInvalid {
                    message: "Could not convert Lua value to StorageValue".to_owned(),
                    subject: "lua".to_owned(),
                    inner: e.to_string(),
                })?;

            store_value(store_as, context, step_id, output).await?;
        }
        Err(err) => {
            return Err(ExecutionError::LuaError {
                message: err.to_string(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tel::StorageValue;

    use super::*;
    use crate::{evaluations::eval, executor::ExecutionTest};

    #[tokio::test]
    async fn test_lua_interop() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        let code = r#"
```lua
function (data)
    return data
end
```
        "#;

        let result = run_function(
            &mut context,
            &vec![Parameter::tel("code", code), Parameter::tel("value", "3.0")],
            "test",
            Some("output"),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(
            eval("output", &context.storage, &context.environment).unwrap(),
            StorageValue::Number(3.0)
        );
    }
}
