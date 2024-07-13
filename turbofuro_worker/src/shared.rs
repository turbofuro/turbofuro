use std::{collections::HashMap, sync::Arc};

use async_recursion::async_recursion;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tracing::{debug, info_span, instrument, Instrument};
use turbofuro_runtime::executor::{CompiledModule, Function, Global, Import, Step, Steps};

use crate::{errors::WorkerError, module_version_resolver::SharedModuleVersionResolver};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleVersion {
    pub id: String,
    #[serde(rename = "moduleId")]
    pub module_id: String,
    pub instructions: Steps,
    pub handlers: HashMap<String, String>,
    pub imports: HashMap<String, Import>,
}

pub fn compile_module(module_version: ModuleVersion) -> CompiledModule {
    let local_functions: Vec<Function> = module_version
        .instructions
        .iter()
        .filter(|s| {
            matches!(
                s,
                Step::DefineFunction {
                    exported: false,
                    ..
                } | Step::DefineNativeFunction {
                    exported: false,
                    ..
                }
            )
        })
        .map(|s| match s {
            Step::DefineFunction {
                id,
                parameters: _,
                body,
                exported: _,
            } => Function::Normal {
                id: id.clone(),
                body: body.clone(),
            },
            Step::DefineNativeFunction {
                id,
                native_id,
                parameters: _,
                exported: _,
            } => Function::Native {
                id: id.to_owned(),
                native_id: native_id.to_owned(),
            },
            other => {
                panic!("Expected function definition, got step: {:?}", other)
            }
        })
        .collect_vec();

    let exported_functions: Vec<Function> = module_version
        .instructions
        .iter()
        .filter(|s| {
            matches!(
                s,
                Step::DefineFunction { exported: true, .. }
                    | Step::DefineNativeFunction { exported: true, .. }
            )
        })
        .map(|s| match s {
            Step::DefineFunction {
                id,
                parameters: _,
                body,
                exported: _,
            } => Function::Normal {
                id: id.clone(),
                body: body.clone(),
            },
            Step::DefineNativeFunction {
                id,
                native_id,
                parameters: _,
                exported: _,
            } => Function::Native {
                id: id.to_owned(),
                native_id: native_id.to_owned(),
            },
            other => {
                panic!("Expected function definition, got step: {:?}", other)
            }
        })
        .collect_vec();

    CompiledModule {
        id: module_version.id,
        module_id: module_version.module_id,
        local_functions,
        exported_functions,
        handlers: module_version.handlers,
        imports: HashMap::new(),
    }
}

#[async_recursion]
#[instrument(skip(global, module_version_resolver), level = "debug")]
pub async fn get_compiled_module(
    id: &str,
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
) -> Result<Arc<CompiledModule>, WorkerError> {
    let cached_module = {
        global
            .modules
            .read()
            .await
            .iter()
            .find(|m| m.id == id)
            .cloned()
    };

    if let Some(cached) = cached_module {
        debug!("Returning cached compiled module: {}", id);
        return Ok(cached);
    }

    debug!("Fetching module: {}", id);
    let module_version: ModuleVersion = {
        module_version_resolver
            .get_module_version(id)
            .instrument(info_span!("get_module_version"))
            .await?
    };

    // Resolve module version for each import
    let mut imports = HashMap::new();
    for (import_name, import) in &module_version.imports {
        debug!("Fetching import: {}", import_name);
        let imported = match import {
            Import::Cloud { id: _, version_id } => {
                get_compiled_module(version_id, global.clone(), module_version_resolver.clone())
                    .await?
            }
        };
        imports.insert(import_name.to_owned(), imported);
    }

    let mut compiled_module = compile_module(module_version);
    compiled_module.imports = imports;
    let module = Arc::new(compiled_module);
    {
        global.modules.write().await.push(module.clone());
    }
    Ok(module)
}

#[cfg(test)]
mod test_shared {
    use serde_json::json;

    use crate::shared::ModuleVersion;

    #[test]
    fn test_can_parse_module_version() {
        let value = json!(
            {
                "moduleId": "test",
                "id": "ZVnigLgKIJQ1d_nmeT71g",
                "type": "HTTP",
                "handlers": {
                  "onHttpRequest": "ZVnigLgKIJQ1d_nmeT71g"
                },
                "imports": {
                    "something": {
                        "type": "cloud",
                        "id": "test",
                        "versionId": "test"
                    }
                },
                "instructions": [
                  {
                    "type": "defineFunction",
                    "id": "ZVnigLgKIJQ1d_nmeT71g",
                    "body": [
                      {
                        "id": "jMwI6DurROzkjebyNhNyD",
                        "type": "call",
                        "callee": "import/http_server/respond_with",
                        "version": "1",
                        "parameters": [
                          {
                            "name": "body",
                            "type": "tel",
                            "expression": "{\n  message: \"Hello World\"\n}"
                          },
                          {
                            "name": "responseType",
                            "type": "tel",
                            "expression": "\"json\""
                          }
                        ]
                      }
                    ],
                    "name": "Handle request",
                    "version": "1.0.0",
                    "parameters": [
                      {
                        "name": "request",
                        "optional": false,
                        "description": "The incoming request object."
                      }
                    ]
                  }
                ]
              }
        );

        serde_json::from_value::<ModuleVersion>(value).unwrap();
    }

    #[test]
    fn test_can_parse_empty_module_version() {
        let value = json!(
            {
                "id": "ZVnigLgKIJQ1d_nmeT71g",
                "type": "HTTP",
                "handlers": {},
                "imports": {},
                "instructions": [],
                "moduleId": "test"
              }
        );

        serde_json::from_value::<ModuleVersion>(value).unwrap();
    }
}
