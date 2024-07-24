use std::{collections::HashMap, sync::Arc};

use crate::{errors::WorkerError, module_version_resolver::SharedModuleVersionResolver};
use async_recursion::async_recursion;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tracing::{debug, info_span, instrument, Instrument};
use turbofuro_runtime::executor::{CompiledModule, Function, Global, Import, Step, Steps};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WorkerWarning {
    #[serde(rename_all = "camelCase")]
    ModuleCouldNotBeLoaded {
        module_id: String,
        module_version_id: String,
        error: WorkerError,
    },
    #[serde(rename_all = "camelCase")]
    ModuleStartupFailed {
        module_id: String,
        error: WorkerError,
    },
    #[serde(rename_all = "camelCase")]
    DebuggerActive { modules: Vec<String> },
    #[serde(rename_all = "camelCase")]
    HttpServerFailedToStart { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkerStoppingReason {
    SignalReceived,
    ConfigurationChanged,
    EnvironmentChanged,
    RequestedByCloudAgent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WorkerStatus {
    #[serde(rename_all = "camelCase")]
    Running {
        warnings: Vec<WorkerWarning>,
    },
    Starting {
        warnings: Vec<WorkerWarning>,
    },
    Stopping {
        reason: WorkerStoppingReason,
        warnings: Vec<WorkerWarning>,
    },
    Stopped {
        reason: WorkerStoppingReason,
        warnings: Vec<WorkerWarning>,
    },
}

impl WorkerStatus {
    pub fn get_warnings(&self) -> Vec<WorkerWarning> {
        match self {
            WorkerStatus::Running { warnings } => warnings.clone(),
            WorkerStatus::Starting { warnings } => warnings.clone(),
            WorkerStatus::Stopping { warnings, .. } => warnings.clone(),
            WorkerStatus::Stopped { warnings, .. } => warnings.clone(),
        }
    }

    pub fn add_warning(&mut self, warning: WorkerWarning) {
        match self {
            WorkerStatus::Running { warnings } => warnings.push(warning),
            WorkerStatus::Starting { warnings } => warnings.push(warning),
            WorkerStatus::Stopping { warnings, .. } => warnings.push(warning),
            WorkerStatus::Stopped { warnings, .. } => warnings.push(warning),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModuleVersion {
    pub id: String,
    pub module_id: String,
    pub instructions: Steps,
    pub handlers: HashMap<String, String>,
    pub imports: HashMap<String, Import>,
}

pub fn compile_module(module_version: &ModuleVersion) -> CompiledModule {
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
                name,
            } => Function::Normal {
                id: id.clone(),
                body: body.clone(),
                name: name.clone(),
            },
            Step::DefineNativeFunction {
                id,
                native_id,
                parameters: _,
                exported: _,
                name,
            } => Function::Native {
                id: id.to_owned(),
                native_id: native_id.to_owned(),
                name: name.to_owned(),
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
                name,
            } => Function::Normal {
                id: id.clone(),
                body: body.clone(),
                name: name.clone(),
            },
            Step::DefineNativeFunction {
                id,
                native_id,
                parameters: _,
                exported: _,
                name,
            } => Function::Native {
                id: id.to_owned(),
                name: name.to_owned(),
                native_id: native_id.to_owned(),
            },
            other => {
                panic!("Expected function definition, got step: {:?}", other)
            }
        })
        .collect_vec();

    CompiledModule {
        id: module_version.id.clone(),
        module_id: module_version.module_id.clone(),
        local_functions,
        exported_functions,
        handlers: module_version.handlers.clone(),
        imports: HashMap::new(),
    }
}

#[instrument(skip_all, level = "debug")]
pub async fn resolve_imports(
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
    imports: &HashMap<String, Import>,
) -> Result<HashMap<String, Arc<CompiledModule>>, WorkerError> {
    let mut resolved_imports: HashMap<String, Arc<CompiledModule>> = HashMap::new();
    for (import_name, import) in imports {
        debug!("Fetching import: {}", import_name);
        let imported = match import {
            Import::Cloud { id: _, version_id } => {
                resolve_and_install_module(
                    version_id,
                    global.clone(),
                    module_version_resolver.clone(),
                )
                .await?
            }
        };
        resolved_imports.insert(import_name.to_owned(), imported);
    }
    Ok(resolved_imports)
}

#[async_recursion]
pub async fn install_module(
    module_version: ModuleVersion,
    global: Arc<Global>,
    module_version_resolver: SharedModuleVersionResolver,
) -> Result<Arc<CompiledModule>, WorkerError> {
    let mut compiled_module = compile_module(&module_version);
    compiled_module.imports = resolve_imports(
        global.clone(),
        module_version_resolver.clone(),
        &module_version.imports,
    )
    .await?;
    let module = Arc::new(compiled_module);
    {
        global.modules.write().await.push(module.clone());
    }
    Ok(module)
}

pub async fn get_compiled_module(
    id: &str,
    global: Arc<Global>,
) -> Result<Arc<CompiledModule>, WorkerError> {
    global
        .modules
        .read()
        .await
        .iter()
        .find(|m| m.id == id)
        .cloned()
        .ok_or_else(|| WorkerError::ModuleVersionNotFound)
}

#[async_recursion]
#[instrument(skip(global, module_version_resolver), level = "debug")]
pub async fn resolve_and_install_module(
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

    install_module(module_version, global, module_version_resolver).await
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
