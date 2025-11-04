use std::{collections::HashMap, path::Path};

use crate::{
    errors::ExecutionError,
    evaluations::{
        eval_opt_string_param, eval_opt_u64_param, eval_optional_param_with_default,
        eval_string_param,
    },
    executor::{ExecutionContext, Parameter},
};
use tel::{describe, Description, StorageValue};
use tracing::instrument;
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::{
    p2::{
        pipe::{MemoryInputPipe, MemoryOutputPipe},
        WasiCtxBuilder,
    },
    preview1::{add_to_linker_async, WasiP1Ctx},
    DirPerms, FilePerms,
};

use super::store_value;

#[derive(Clone)]
struct WasmInstance {
    engine: Engine,
    module: Module,
    linker: Linker<WasiP1Ctx>,
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

        // A `Linker` is shared in the environment amongst all stores, and this
        // linker is used to instantiate the `module` above. This example only
        // adds WASI functions to the linker, notably the async versions built
        // on tokio.
        let mut linker = Linker::new(&engine);
        add_to_linker_async(&mut linker, |cx| cx)?;

        Ok(Self {
            engine,
            module,
            linker,
        })
    }
}

#[instrument(level = "debug", skip_all)]
pub async fn run_wasi(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    step_id: &str,
    store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;
    let input = eval_opt_string_param("input", parameters, context)?.unwrap_or("".to_owned());
    let fuel = eval_opt_u64_param("fuel", parameters, context)?.unwrap_or(u64::MAX);
    let max_async_yield = eval_opt_u64_param("maxAsyncYield", parameters, context)?;

    let env = eval_optional_param_with_default(
        "env",
        parameters,
        context,
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
            expected: Description::new_base_type("object").into(),
            actual: describe(s).into(),
        }),
    }?;

    let args =
        eval_optional_param_with_default("args", parameters, context, StorageValue::Array(vec![]))?;
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
            expected: Description::new_base_type("array").into(),
            actual: describe(s).into(),
        }),
    }?;

    let mut wasm = WasmInstance::new(path)?;
    let stdin = MemoryInputPipe::new(input.into_bytes());
    let stdout = MemoryOutputPipe::new(1_000_000); // Capture up to 1MB
    let stderr = MemoryOutputPipe::new(1_000_000); // Capture up to 1MB

    let wasi = WasiCtxBuilder::new()
        .preopened_dir(".", ".", DirPerms::all(), FilePerms::all())
        .map_err(|e| ExecutionError::WasmError {
            message: e.to_string(),
        })?
        .args(&args)
        .envs(&env?)
        .stdin(stdin)
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .build_p1();

    let mut store = Store::new(&wasm.engine, wasi);
    store.set_fuel(fuel)?;
    store.fuel_async_yield_interval(max_async_yield)?;

    let func = wasm
        .linker
        .module_async(&mut store, "", &wasm.module)
        .await?
        .get_default(&mut store, "")?
        .typed::<(), ()>(&store)?;

    func.call_async(&mut store, ()).await?;

    let data = stdout.contents();
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
                Parameter::tel("env", r#"{ "TEST_VAR": "Test Value" }"#),
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

    // These are advanced WASI tests that require a chonky WASI image
    // You can download images from here:
    // https://github.com/webassemblylabs/webassembly-language-runtimes
    // You should download them into the `turbofuro_runtime` directory.
    // These tests are commented out because they are slow and require a download.

    // #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // async fn test_wasi_python() {
    //     let mut t = ExecutionTest::default();
    //     let mut context = t.get_context();

    //     run_wasi(
    //         &mut context,
    //         &vec![
    //             Parameter::tel("path", r#""./python-3.12.0.wasm""#),
    //             Parameter::tel(
    //                 "args",
    //                 r#"["python", "-c", "print('Hello World from Python')"]"#,
    //             ),
    //             Parameter::tel("input", r#""""#),
    //         ],
    //         "test",
    //         Some("output"),
    //     )
    //     .await
    //     .unwrap();

    //     assert_eq!(
    //         eval("output", &context.storage, &context.environment).unwrap(),
    //         "Hello World from Python\n".into()
    //     );
    // }

    // #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // async fn test_wasi_ruby() {
    //     let mut t = ExecutionTest::default();
    //     let mut context = t.get_context();

    //     run_wasi(
    //         &mut context,
    //         &vec![
    //             Parameter::tel("path", r#""./ruby-3.2.2.wasm""#),
    //             Parameter::tel("args", r#"["ruby", "-e", "puts('Hello World from Ruby')"]"#),
    //             Parameter::tel("input", r#""""#),
    //         ],
    //         "test",
    //         Some("output"),
    //     )
    //     .await
    //     .unwrap();

    //     assert_eq!(
    //         eval("output", &context.storage, &context.environment).unwrap(),
    //         "Hello World from Ruby\n".into()
    //     );
    // }

    // #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // async fn test_wasi_php() {
    //     let mut t = ExecutionTest::default();
    //     let mut context = t.get_context();

    //     run_wasi(
    //         &mut context,
    //         &vec![
    //             Parameter::tel("path", r#""./php-cgi-8.2.6.wasm""#),
    //             Parameter::tel("env", r#"{}"#),
    //             Parameter::tel("input", r#""Hello World""#),
    //         ],
    //         "test",
    //         Some("output"),
    //     )
    //     .await
    //     .unwrap();

    //     assert_eq!(
    //         eval("output", &context.storage, &context.environment).unwrap(),
    //         StorageValue::String("X-Powered-By: PHP/8.2.6\r\nContent-type: text/html; charset=UTF-8\r\n\r\nHello World".to_owned())
    //     );
    // }
}
