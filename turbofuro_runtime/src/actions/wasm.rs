use std::{collections::HashMap, path::Path};

use crate::{
    errors::ExecutionError,
    evaluations::{eval_optional_param_with_default, eval_param},
    executor::{ExecutionContext, Parameter},
};
use tel::{describe, Description, StorageValue};
use tracing::instrument;
use wasi_common::pipe::{ReadPipe, WritePipe};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::{ambient_authority, tokio::WasiCtxBuilder};

use super::{as_string, store_value};

#[derive(Clone)]
struct WasmInstance {
    engine: Engine,
    module: Module,
}

impl WasmInstance {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, ExecutionError> {
        let mut config = Config::new();
        config.async_support(true);
        config.consume_fuel(true);
        config.debug_info(true);
        let engine = Engine::new(&config)?;

        // This can take a while
        let module = Module::from_file(&engine, path)?;

        Ok(Self { engine, module })
    }
}

#[instrument(level = "debug", skip_all)]
pub async fn run_wasi(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let path = eval_param("path", parameters, &context.storage, &context.environment)?;
    let path = as_string(path, "path")?;

    let input = eval_optional_param_with_default(
        "input",
        parameters,
        &context.storage,
        &context.environment,
        "".into(),
    )?;
    let input = as_string(input, "input")?;

    let env = eval_optional_param_with_default(
        "env",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Object(HashMap::new()),
    )?;
    let env: Result<Vec<(String, String)>, ExecutionError> = match env {
        StorageValue::Object(obj) => Ok(obj
            .into_iter()
            .map(|(k, v)| {
                let value = v.to_string()?;

                Ok((k, value))
            })
            .collect()),
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "env".to_owned(),
            expected: Description::new_base_type("object"),
            actual: describe(s),
        }),
    }?;

    let args = eval_optional_param_with_default(
        "args",
        parameters,
        &context.storage,
        &context.environment,
        StorageValue::Array(vec![]),
    )?;
    let args = match args {
        StorageValue::Array(a) => {
            let mut args = Vec::new();
            for arg in a {
                args.push(arg.to_string()?);
            }
            Ok(args)
        }
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "args".to_owned(),
            expected: Description::new_base_type("array"),
            actual: describe(s),
        }),
    }?;

    let wasm = WasmInstance::new(path)?;
    let stdout = WritePipe::new_in_memory();
    let stdin = ReadPipe::from(input);

    let wasi = WasiCtxBuilder::new()
        .preopened_dir(
            wasmtime_wasi::Dir::open_ambient_dir(".", ambient_authority())?,
            "/",
        )
        .map_err(|e| ExecutionError::WasmError {
            message: e.to_string(),
        })?
        .args(&args)
        .map_err(|e| ExecutionError::WasmError {
            message: e.to_string(),
        })?
        .envs(&env?)
        .map_err(|e| ExecutionError::WasmError {
            message: e.to_string(),
        })?
        .stdin(Box::new(stdin))
        .stdout(Box::new(stdout.clone()))
        .inherit_stderr()
        .build();

    let mut store = Store::new(&wasm.engine, wasi);
    // TODO: Add ability to set fuel and async yield interval
    store.set_fuel(u64::MAX)?;
    store.fuel_async_yield_interval(Some(10000))?;

    let mut linker = Linker::new(&wasm.engine);
    // Add WASI for Tokio magic
    wasmtime_wasi::tokio::add_to_linker(&mut linker, |cx| cx)?;

    linker.module_async(&mut store, "", &wasm.module).await?;
    linker
        .get_default(&mut store, "")?
        .typed::<(), ()>(&store)?
        .call_async(store, ())
        .await?;

    let data = stdout.try_into_inner().unwrap_or_default().into_inner();
    let data = String::from_utf8_lossy(&data).to_string();

    store_value(store_as, context, step_id, data.into()).await?;

    Ok(())
}

#[cfg(test)]
mod test_wasm {
    use crate::{evaluations::eval, executor::ExecutionTest};

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_simple_wasi() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        // wasm-example.wasm is a simple WASI program built using Rust:
        //
        // use std::env;
        // fn main() {
        //     println!("WASI on Turbofuro!");
        //     println!("Arguments:");
        //     for arg in env::args() {
        //         println!("{}", arg);
        //     }
        //     println!("Environment:");
        //     for (key, value) in env::vars() {
        //         println!("{}={}", key, value);
        //     }
        //     print!("Stdin: ");
        //     let mut buffer = String::new();
        //     std::io::stdin().read_line(&mut buffer).unwrap();
        //     println!("{}", buffer);
        // }
        //
        // Compiled using: cargo build --target=wasm32-wasi --release

        run_wasi(
            &mut context,
            &vec![
                Parameter::tel("path", r#""./src/actions/wasm-example.wasm""#),
                Parameter::tel("args", r#"["arg1", "500"]"#),
                Parameter::tel(
                    "env",
                    r#"
                {
                    "TEST_VAR": "Test Value"
                }
                "#,
                ),
                Parameter::tel("input", r#""Test""#),
            ],
            "test",
            Some("output"),
        )
        .await
        .unwrap();

        assert_eq!(
            eval("output", &context.storage, &context.environment).unwrap(),
            r#"WASI on Turbofuro!
Arguments:
arg1
500
Environment:
TEST_VAR=Test Value
Stdin: Test
"#
            .into()
        );
    }
}
