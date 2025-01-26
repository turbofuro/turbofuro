mod utils;

use serde::Serialize as SerdeSerialize;
use serde_derive::{Deserialize, Serialize};
use serde_wasm_bindgen::Serializer;
use utils::set_panic_hook;
use wasm_bindgen::prelude::*;

const SERIALIZER: Serializer = Serializer::new().serialize_maps_as_objects(true);

use std::{
    collections::HashMap,
    fmt::{self, Display},
};

use tel::{
    parse, parse_description, DescribedEnvironment, DescribedStorage, Description,
    LayeredDescribedStorage, ObjectBody, ObjectDescription, Selector, SelectorDescription,
    StorageValue, TelError, TelParseError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepProbe {
    pub id: String,
    pub output: Description,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepAnalysis {
    pub id: String,
    pub problems: Vec<AnalysisProblem>,
    pub after: Option<HashMap<String, Description>>,
    pub before: Option<HashMap<String, Description>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ResourceOperation {
    Provision,
    Consumption,
    Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEvent {
    resource: String,
    operation: ResourceOperation,
}

fn is_worker_resource(resource: &str) -> bool {
    resource == "postgres_connection"
        || resource == "redis_connection"
        || resource == "http_client"
        || resource == "actor_link"
        || resource == "libsql_connection"
        || resource == "webdriver_client"
}

fn resource_matches(tested: &str, by: &str) -> bool {
    if by == "streamable" {
        return tested == "pending_http_request"
            || tested == "pending_http_response"
            || tested == "pending_form_data_field"
            || tested == "file_handle";
    }
    by == tested
}

#[derive(Debug, Clone)]
struct PredicatedResource {
    resource: String,
    consumable: bool,
}

impl PredicatedResource {
    fn new_consumable(resource: &str) -> PredicatedResource {
        PredicatedResource {
            resource: resource.to_owned(),
            consumable: true,
        }
    }

    fn new_static(resource: &str) -> PredicatedResource {
        PredicatedResource {
            resource: resource.to_owned(),
            consumable: false,
        }
    }
}

impl ResourceEvent {
    fn new_from_annotation(annotation: &FunctionAnnotation) -> Option<Self> {
        match annotation {
            FunctionAnnotation::Provision { resource } => Some(ResourceEvent {
                operation: ResourceOperation::Provision,
                resource: resource.clone(),
            }),
            FunctionAnnotation::Consumption { resource } => Some(ResourceEvent {
                operation: ResourceOperation::Consumption,
                resource: resource.clone(),
            }),
            FunctionAnnotation::Requirement { resource } => Some(ResourceEvent {
                operation: ResourceOperation::Usage,
                resource: resource.clone(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FakeEnvironment {
    value: Description,
}

impl Default for FakeEnvironment {
    fn default() -> Self {
        Self {
            value: Description::new_base_type("string"),
        }
    }
}

impl DescribedEnvironment for FakeEnvironment {
    fn get(&self, _key: &str) -> Option<&Description> {
        Some(&self.value)
    }
}

#[derive(Debug, Clone)]
struct Context {
    storage: HashMap<String, Description>,
    environment: FakeEnvironment,
    references: HashMap<String, Option<String>>,
    inside_loop: bool,
    resources: Vec<PredicatedResource>,
    resource_events: Vec<ResourceEvent>,
    functions: Vec<FunctionDeclaration>,
    ended: bool,
    steps_to_capture: Vec<String>,
    output_description: Option<Description>,
}

impl Context {
    pub fn find_resource(&mut self, resource: &str) -> Option<PredicatedResource> {
        self.resources
            .iter()
            .find(|r| resource_matches(r.resource.as_str(), resource))
            .cloned()
    }

    pub fn consume_resource(&mut self, resource: &str) -> Option<PredicatedResource> {
        let to_remove = self
            .resources
            .iter()
            .rposition(|r| resource_matches(&r.resource, resource) && r.consumable);

        if let Some(i) = to_remove {
            let item = self.resources.remove(i);
            return Some(item.clone());
        }
        None
    }

    pub fn add_resource(&mut self, resource: &str) {
        self.resources
            .push(PredicatedResource::new_consumable(resource));
    }
}

pub fn parse_and_predict_description<T: DescribedStorage, E: DescribedEnvironment>(
    expression: String,
    storage: &T,
    environment: &E,
) -> Description {
    let result = parse(&expression);
    if !result.errors.is_empty() {
        return Description::Error {
            error: TelError::ParseError {
                errors: result.errors,
            },
        };
    }
    let expr = result.expr.ok_or(TelError::Unknown {
        message: "No expression".into(),
    });
    match expr {
        Ok(expr) => tel::predict_description(expr, storage, environment),
        Err(error) => Description::Error { error },
    }
}

pub fn parse_and_evaluate_description_notation(description: &str) -> Description {
    let result = parse_description(description);
    if !result.errors.is_empty() {
        return Description::Error {
            error: TelError::ParseError {
                errors: result.errors,
            },
        };
    }
    let expr = result.expr.ok_or(TelError::Unknown {
        message: "No expression".into(),
    });
    match expr {
        Ok(expr) => match tel::evaluate_description_notation(expr) {
            Ok(value) => value,
            Err(error) => Description::Error { error },
        },

        Err(error) => Description::Error { error },
    }
}

pub struct Analysis {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ParameterType {
    Tel,
    FunctionRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParameterDefinition {
    name: String,
    description: Option<String>,
    optional: bool,
    #[serde(rename = "type")]
    type_: ParameterType,
    value_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDeclaration {
    id: String,
    parameters: Vec<ParameterDefinition>,
    output_description: Option<String>,
    decorator: Option<bool>,
    annotations: Vec<FunctionAnnotation>,
    //   // Null if this is a local function
    //   moduleVersionId?: string;
    //   outputDescription?: string;
    //   decorator?: boolean;
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FunctionAnnotation {
    Exported,
    ModuleStarter,
    ModuleStopper,
    Environment { key: String, strict: bool },
    Provision { resource: String },
    Consumption { resource: String },
    Requirement { resource: String },
    ActorCreator,
    Throws { code: String },
    Version { version: String },
}

fn default_exported() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    Local {
        function_id: String,
    },
    Import {
        import_name: String,
        function_id: String,
    },
}

impl Display for Callee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Callee::Local { function_id } => write!(f, "local/{}", function_id),
            Callee::Import {
                import_name,
                function_id,
            } => write!(f, "import/{}/{}", import_name, function_id),
        }
    }
}

impl serde::Serialize for Callee {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

struct CalleeVisitor;

impl<'de> serde::de::Visitor<'de> for CalleeVisitor {
    type Value = Callee;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string with format 'local/ID' or 'import/MODULE_ID:ID'")
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        let parts: Vec<&str> = value.split('/').collect();

        match parts.as_slice() {
            ["local", function_id] => Ok(Callee::Local {
                function_id: function_id.to_string(),
            }),
            ["import", module_version_id, function_id] => Ok(Callee::Import {
                import_name: module_version_id.to_string(),
                function_id: function_id.to_string(),
            }),
            _ => Err(serde::de::Error::custom("Invalid format for Callee")),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Callee {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(CalleeVisitor)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Parameter {
    Tel { name: String, expression: String },
    FunctionRef { name: String, id: String },
}

impl Parameter {
    pub fn tel(name: &str, expression: &str) -> Parameter {
        Parameter::Tel {
            name: name.to_owned(),
            expression: expression.to_owned(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Parameter::Tel { name, .. } => name,
            Parameter::FunctionRef { name, .. } => name,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Branch {
    pub condition: String,
    pub steps: Steps,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum Step {
    #[serde(rename_all = "camelCase")]
    Call {
        id: String,
        callee: Callee,
        parameters: Vec<Parameter>,
        store_as: Option<String>,
        #[serde(default)]
        disabled: bool,
        body: Option<Steps>, // Only if decorator is called
    },
    #[serde(rename_all = "camelCase")]
    DefineFunction {
        id: String,
        parameters: Vec<ParameterDefinition>,
        #[serde(default)]
        annotations: Vec<FunctionAnnotation>,
        #[serde(default = "default_exported")]
        exported: bool,
        name: String,
        #[serde(default)]
        disabled: bool,
        #[serde(default)]
        decorator: bool,
        body: Steps,
        output_description: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    DefineNativeFunction {
        id: String,
        name: String,
        native_id: String,
        parameters: Vec<ParameterDefinition>,
        #[serde(default)]
        annotations: Vec<FunctionAnnotation>,
        #[serde(default = "default_exported")]
        exported: bool,
        #[serde(default)]
        disabled: bool,
        #[serde(default)]
        decorator: bool,
        output_description: Option<String>,
    },
    If {
        id: String,
        condition: String,
        then: Steps,
        branches: Option<Vec<Branch>>,
        #[serde(rename = "else")]
        else_: Option<Steps>,
        #[serde(default)]
        disabled: bool,
    },
    ForEach {
        id: String,
        items: String,
        item: String,
        body: Steps,
        #[serde(default)]
        disabled: bool,
    },
    While {
        id: String,
        condition: String,
        body: Steps,
        #[serde(default)]
        disabled: bool,
    },
    Return {
        id: String,
        value: Option<String>,
        #[serde(default)]
        disabled: bool,
    },
    Break {
        id: String,
        #[serde(default)]
        disabled: bool,
    },
    Continue {
        id: String,
        #[serde(default)]
        disabled: bool,
    },
    Assign {
        id: String,
        value: String,
        to: String,
        #[serde(default)]
        disabled: bool,
    },
    Try {
        id: String,
        body: Steps,
        catch: Steps,
        #[serde(default)]
        disabled: bool,
    },
    Throw {
        id: String,
        code: String,
        message: String,
        details: Option<String>,
        metadata: Option<String>,
        #[serde(default)]
        disabled: bool,
    },
    #[serde(rename_all = "camelCase")]
    Parse {
        id: String,
        description: String,
        value: String,
        store_as: String,
        #[serde(default)]
        disabled: bool,
    },
    #[serde(rename_all = "camelCase")]
    Transform {
        id: String,
        value: String,
        filter_by: Option<String>,
        order_by: Option<String>,
        map: Option<String>,
        store_as: String,
        #[serde(default)]
        disabled: bool,
    },
}

impl Step {
    pub fn get_step_id(&self) -> &str {
        match self {
            Step::Call { id, .. } => id,
            Step::If { id, .. } => id,
            Step::ForEach { id, .. } => id,
            Step::While { id, .. } => id,
            Step::DefineFunction { id, .. } => id,
            Step::Return { id, .. } => id,
            Step::Break { id, .. } => id,
            Step::Continue { id, .. } => id,
            Step::Assign { id, .. } => id,
            Step::DefineNativeFunction { id, .. } => id,
            Step::Try { id, .. } => id,
            Step::Throw { id, .. } => id,
            Step::Parse { id, .. } => id,
            Step::Transform { id, .. } => id,
        }
    }

    pub fn is_disabled(&self) -> bool {
        match self {
            Step::Call { disabled, .. } => *disabled,
            Step::If { disabled, .. } => *disabled,
            Step::ForEach { disabled, .. } => *disabled,
            Step::While { disabled, .. } => *disabled,
            Step::Return { disabled, .. } => *disabled,
            Step::Break { disabled, .. } => *disabled,
            Step::Continue { disabled, .. } => *disabled,
            Step::Assign { disabled, .. } => *disabled,
            Step::DefineNativeFunction { disabled, .. } => *disabled,
            Step::Try { disabled, .. } => *disabled,
            Step::Throw { disabled, .. } => *disabled,
            Step::Parse { disabled, .. } => *disabled,
            Step::Transform { disabled, .. } => *disabled,
            Step::DefineFunction { disabled, .. } => *disabled,
        }
    }
}

pub type Steps = Vec<Step>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AnalysisProblem {
    Error {
        code: String,
        message: String,
        field: Option<String>,
    },
    Warning {
        code: String,
        message: String,
        field: Option<String>,
    },
}

impl AnalysisProblem {
    pub fn code(&self) -> &str {
        match self {
            AnalysisProblem::Error { code, .. } => code,
            AnalysisProblem::Warning { code, .. } => code,
        }
    }

    pub fn new_from_parse_error(error: TelParseError, field: Option<String>) -> Self {
        AnalysisProblem::Error {
            code: "PARSE_ERROR".to_owned(),
            message: error.to_string(),
            field,
        }
    }
}

fn check_expression(
    context: &mut Context,
    value_expression: String,
    declaration_description: Option<String>,
    field: Option<String>,
) -> Vec<AnalysisProblem> {
    let mut problems = vec![];

    let parsed = tel::parse(&value_expression);
    let mut parse_errors: Vec<AnalysisProblem> = parsed
        .errors
        .iter()
        .map(|e| AnalysisProblem::new_from_parse_error(e.clone(), field.clone()))
        .collect();
    problems.append(&mut parse_errors);

    if let Some(expr) = parsed.expr {
        let value_description =
            tel::predict_description(expr, &context.storage, &context.environment);

        if let Some(error) = value_description.error() {
            problems.push(AnalysisProblem::Error {
                code: "RESOLVES_TO_ERROR".to_owned(),
                message: format!("Expression resolves to error: {}", error),
                field: field.clone(),
            });
        }

        // If there is output description, check if it is compatible
        if let Some(declaration_description) = declaration_description {
            let declaration_description =
                parse_and_evaluate_description_notation(&declaration_description);

            if !value_description.is_compatible(&declaration_description) {
                problems.push(AnalysisProblem::Error {
                    code: "INCOMPATIBLE_VALUE".to_owned(),
                    message: format!(
                        "Expected {} but got {}",
                        declaration_description, value_description
                    ),
                    field: field.clone(),
                });
            }
        }
    }

    problems
}

fn check_not_always_boolean(
    description: &Description,
    field: Option<String>,
) -> Vec<AnalysisProblem> {
    let description = description.to_boolean();

    match description {
        Description::BooleanValue { value } => {
            vec![AnalysisProblem::Warning {
                code: "CONSTANT_CONDITION".to_owned(),
                message: format!("Value is always {}", value),
                field,
            }]
        }
        _ => vec![],
    }
}

fn store_in_storage(
    context: &mut Context,
    store_as: Option<String>,
    description: Description,
) -> Vec<AnalysisProblem> {
    let mut problems = vec![];
    if let Some(store_as) = store_as {
        let selector = tel::parse(&store_as);
        let mut parse_errors: Vec<AnalysisProblem> = selector
            .errors
            .iter()
            .map(|e| AnalysisProblem::new_from_parse_error(e.clone(), Some("storeAs".to_owned())))
            .collect();
        problems.append(&mut parse_errors);

        if let Some(expr) = selector.expr {
            let selector =
                tel::evaluate_selector_description(expr, &context.storage, &context.environment);
            for selector in selector.into_iter() {
                match selector {
                    SelectorDescription::Static { selector } => {
                        match tel::store_description(
                            &selector,
                            &mut context.storage,
                            description.clone(),
                        ) {
                            Ok(()) => {}
                            Err(error) => {
                                problems.push(AnalysisProblem::Error {
                                    field: Some("storeAs".to_owned()),
                                    code: "STORAGE_PREDICTION_FAILED".to_owned(),
                                    message: error.to_string(),
                                });
                            }
                        }
                    }
                    SelectorDescription::Error { error } => {
                        problems.push(AnalysisProblem::Error {
                            field: Some("storeAs".to_owned()),
                            code: "STORAGE_SELECTOR_FAILED".to_owned(),
                            message: error.to_string(),
                        });
                    }
                    SelectorDescription::Unknown => {
                        problems.push(AnalysisProblem::Warning {
                            field: Some("storeAs".to_owned()),
                            code: "STORAGE_SELECTOR_UNKNOWN".to_owned(),
                            message: "Unknown selector".to_owned(),
                        });
                    }
                }
            }
        }
    }
    problems
}

fn analyze_step(
    context: &mut Context,
    step: &Step,
    previous: Option<&Analysis>,
) -> Vec<StepAnalysis> {
    let mut list = vec![];
    let capture_storage = context
        .steps_to_capture
        .contains(&step.get_step_id().to_owned());
    let mut analysis = StepAnalysis {
        id: step.get_step_id().to_owned(),
        problems: vec![],
        after: None,
        before: {
            if capture_storage {
                Some(context.storage.clone())
            } else {
                None
            }
        },
    };

    if step.is_disabled() {
        return list;
    }

    if context.ended {
        analysis.problems.push(AnalysisProblem::Error {
            code: "UNREACHABLE_CODE".to_owned(),
            message: "This code is unreachable".to_owned(),
            field: None,
        });
    }

    match step {
        Step::Call {
            callee,
            parameters,
            store_as,
            body,
            ..
        } => {
            // For functions we need to check:
            // - Check if the function is found - done
            // - Check if all required parameters are present - done
            // - Check if an additional parameter is present - done
            // - Parse any tel expressions - done
            // - Parse any function references
            // - Track the resources
            // - Update storage with output description - dopne
            let function = context
                .functions
                .iter()
                .find(|f| match callee {
                    Callee::Local { function_id } => *function_id == f.id,
                    Callee::Import { function_id, .. } => f.id == *function_id,
                })
                .cloned();

            if function.is_none() {
                analysis.problems.push(AnalysisProblem::Error {
                    code: "FUNCTION_NOT_FOUND".to_owned(),
                    message: format!("Function {} not found", callee),
                    field: None,
                });
            }

            if let Some(body) = body {
                for step in body {
                    list.append(&mut analyze_step(context, step, previous));
                }
            }

            if let Some(function) = function {
                // Check parameters
                for definition in &function.parameters {
                    let parameter = parameters
                        .iter()
                        .find(|p| p.name() == definition.name)
                        .cloned();

                    match parameter {
                        Some(parameter) => {
                            match parameter {
                                Parameter::Tel {
                                    name: _,
                                    expression,
                                } => {
                                    let mut problems = check_expression(
                                        context,
                                        expression,
                                        definition.value_description.clone(),
                                        Some(definition.name.clone()),
                                    );
                                    analysis.problems.append(&mut problems);
                                }
                                Parameter::FunctionRef { name, id } => {
                                    let function =
                                        context.functions.iter().find(|f| *f.id == id).cloned();
                                    match function {
                                        Some(_function) => {
                                            // TODO: Check if the function is compatible
                                        }
                                        None => {
                                            analysis.problems.push(AnalysisProblem::Error {
                                                code: "FUNCTION_NOT_FOUND".to_owned(),
                                                message: format!("Function {} not found", id),
                                                field: Some(name.clone()),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        None => {
                            if !definition.optional {
                                analysis.problems.push(AnalysisProblem::Error {
                                    code: "MISSING_PARAMETER".to_owned(),
                                    message: format!("Missing parameter {}", definition.name),
                                    field: Some(definition.name.clone()),
                                });
                            }
                        }
                    }
                }

                // Apply resources
                for annotation in &function.annotations {
                    // Add entry
                    let entry = ResourceEvent::new_from_annotation(annotation);
                    if let Some(entry) = entry {
                        context.resource_events.push(entry);
                    }

                    // Apply top resource list
                    match annotation {
                        FunctionAnnotation::Provision { resource } => {
                            // TODO: Decide how to handle worker resources
                            if is_worker_resource(resource) {
                                continue;
                            }

                            context.add_resource(resource);
                        }
                        FunctionAnnotation::Consumption { resource } => {
                            // TODO: Decide how to handle worker resources
                            if is_worker_resource(resource) {
                                continue;
                            }

                            let found = context.consume_resource(resource);
                            if found.is_none() {
                                // Let's check if there non-consumable resource
                                let found_non_consumable = context.find_resource(resource);

                                if found_non_consumable.is_some() {
                                    analysis.problems.push(AnalysisProblem::Error {
                                        code: "MISSING_RESOURCE".to_owned(),
                                        message: format!(
                                            "Missing resource {} that can be consumed",
                                            resource
                                        ),
                                        field: None,
                                    });
                                } else {
                                    analysis.problems.push(AnalysisProblem::Error {
                                        code: "MISSING_RESOURCE".to_owned(),
                                        message: format!("Missing resource {}", resource),
                                        field: None,
                                    });
                                }
                            }
                        }
                        FunctionAnnotation::Requirement { resource } => {
                            // TODO: Decide how to handle worker resources
                            if is_worker_resource(resource) {
                                continue;
                            }

                            let found = context.find_resource(resource);
                            if found.is_none() {
                                analysis.problems.push(AnalysisProblem::Error {
                                    code: "MISSING_RESOURCE".to_owned(),
                                    message: format!("Missing resource {}", resource),
                                    field: None,
                                });
                            }
                        }
                        _ => {}
                    }
                }

                // Check if there are additional parameters
                for parameter in parameters {
                    // Check if the parameter is present in the function definition
                    if !function
                        .parameters
                        .iter()
                        .any(|p| p.name == parameter.name())
                    {
                        analysis.problems.push(AnalysisProblem::Warning {
                            code: "UNEXPECTED_PARAMETER".to_owned(),
                            message: format!("Unexpected parameter {}", parameter.name()),
                            field: Some(parameter.name().to_owned()),
                        });
                    }
                }

                // Update storage with output description
                if let Some(output_description) = function.output_description {
                    let description = parse_and_evaluate_description_notation(&output_description);

                    let (description, mut problems) =
                        description_to_problems(description, Some("outputDescription".to_owned()));
                    analysis.problems.append(&mut problems);

                    let mut problems = store_in_storage(
                        context,
                        store_as.clone(),
                        description.unwrap_or_default(),
                    );
                    analysis.problems.append(&mut problems);
                } else {
                    let mut problems =
                        store_in_storage(context, store_as.clone(), Description::Any);
                    analysis.problems.append(&mut problems);
                }
            }
        }
        Step::DefineFunction {
            parameters,
            body,
            output_description,
            annotations,
            ..
        } => {
            // Add predicated resources from annotations
            let mut resources: Vec<PredicatedResource> = vec![];
            for annotation in annotations {
                match annotation {
                    FunctionAnnotation::Consumption { resource } => {
                        resources.push(PredicatedResource::new_consumable(resource));
                    }
                    FunctionAnnotation::Requirement { resource } => {
                        // Makes sure that this resource is only used, but not consumed
                        resources.push(PredicatedResource::new_static(resource));
                    }
                    _ => {}
                }
            }

            let mut function_context = Context {
                storage: HashMap::new(),
                environment: FakeEnvironment::default(),
                inside_loop: false,
                resources,
                functions: context.functions.clone(),
                ended: false,
                steps_to_capture: context.steps_to_capture.clone(),
                references: context.references.clone(),
                output_description: output_description.clone().and_then(|o| {
                    let description = parse_and_evaluate_description_notation(&o);
                    let (correct, mut problems) =
                        description_to_problems(description, Some("outputDescription".to_owned()));
                    analysis.problems.append(&mut problems);
                    correct
                }),
                resource_events: vec![],
            };
            for parameter in parameters {
                match parameter.type_ {
                    ParameterType::Tel => {
                        let value_description = parameter
                            .value_description
                            .clone()
                            .map(|s| parse_and_evaluate_description_notation(&s))
                            .unwrap_or_else(|| Description::Any);

                        let (value_description, mut problems) = description_to_problems(
                            value_description,
                            Some(format!("{}.valueDescription", parameter.name)),
                        );
                        analysis.problems.append(&mut problems);

                        function_context.storage.insert(
                            parameter.name.clone(),
                            value_description.unwrap_or_default(),
                        );
                    }
                    // TODO: Parse function references
                    ParameterType::FunctionRef => {
                        function_context
                            .references
                            .insert(parameter.name.clone(), None);
                    }
                }
            }
            for step in body {
                list.append(&mut analyze_step(&mut function_context, step, previous));
            }

            // TODO: Compute annotations
            // For every sub-context let's observe the resources
        }
        Step::DefineNativeFunction {
            parameters,
            output_description,
            ..
        } => {
            if let Some(output_description) = output_description {
                let description = parse_and_evaluate_description_notation(output_description);
                let (_, mut problems) =
                    description_to_problems(description, Some("outputDescription".to_owned()));
                analysis.problems.append(&mut problems);
            }

            for parameter in parameters {
                match parameter.type_ {
                    ParameterType::Tel => {
                        let value_description = parameter
                            .value_description
                            .clone()
                            .map(|s| parse_and_evaluate_description_notation(&s))
                            .unwrap_or_else(|| Description::Any);

                        let (_value_description, mut problems) = description_to_problems(
                            value_description,
                            Some(format!("{}.valueDescription", parameter.name)),
                        );
                        analysis.problems.append(&mut problems);
                    }
                    // TODO: Parse function references
                    ParameterType::FunctionRef => {}
                }
            }
        }
        Step::If {
            condition,
            then,
            branches,
            else_,
            ..
        } => {
            let condition = parse_and_predict_description(
                condition.into(),
                &context.storage,
                &context.environment,
            );
            let (condition, mut problems) =
                description_to_problems(condition, Some("condition".to_owned()));
            analysis.problems.append(&mut problems);

            if let Some(condition) = condition {
                let mut problems =
                    check_not_always_boolean(&condition, Some("condition".to_owned()));
                analysis.problems.append(&mut problems);
            }

            let mut then_context = context.clone();
            for step in then {
                list.append(&mut analyze_step(&mut then_context, step, previous));
            }
            drop(then_context);

            if let Some(branches) = branches {
                for (i, branch) in branches.iter().enumerate() {
                    let condition = parse_and_predict_description(
                        branch.condition.clone(),
                        &context.storage,
                        &context.environment,
                    );
                    let (condition, mut problems) = description_to_problems(
                        condition,
                        Some(format!("branches[{}].condition", i)),
                    );
                    analysis.problems.append(&mut problems);
                    if let Some(condition) = condition {
                        let mut problems = check_not_always_boolean(
                            &condition,
                            Some("branches[{}].condition".to_owned()),
                        );
                        analysis.problems.append(&mut problems);
                    }

                    let mut branch_context = context.clone();
                    for step in &branch.steps {
                        list.append(&mut analyze_step(&mut branch_context, step, previous));
                    }
                    drop(branch_context);
                }
            }

            if let Some(else_) = else_ {
                let mut else_branch = context.clone();
                for step in else_ {
                    list.append(&mut analyze_step(&mut else_branch, step, previous));
                }
                drop(else_branch);
            }
        }
        Step::ForEach {
            items, item, body, ..
        } => {
            let items =
                parse_and_predict_description(items.into(), &context.storage, &context.environment);
            let item_description = items.get_part();
            store_in_storage(context, Some(item.clone()), item_description.clone());

            let in_loop = context.inside_loop;
            context.inside_loop = true;
            for step in body {
                list.append(&mut analyze_step(context, step, previous));
            }

            context.inside_loop = in_loop;
        }
        Step::While {
            condition, body, ..
        } => {
            let condition = parse_and_predict_description(
                condition.into(),
                &context.storage,
                &context.environment,
            );
            let (condition, mut problems) =
                description_to_problems(condition, Some("condition".to_owned()));
            analysis.problems.append(&mut problems);

            if let Some(condition) = condition {
                let mut problems =
                    check_not_always_boolean(&condition, Some("condition".to_owned()));
                analysis.problems.append(&mut problems);
            }

            let in_loop = context.inside_loop;
            context.inside_loop = true;
            for step in body {
                list.append(&mut analyze_step(context, step, previous));
            }
            context.inside_loop = in_loop;
        }
        Step::Return { value, .. } => {
            if let Some(value) = value {
                let value = parse_and_predict_description(
                    value.clone(),
                    &context.storage,
                    &context.environment,
                );
                let (value, mut problems) =
                    description_to_problems(value, Some("value".to_owned()));
                analysis.problems.append(&mut problems);

                if let Some(output_description) = &context.output_description {
                    let value = value.unwrap_or_default();
                    if !value.is_compatible(output_description) {
                        analysis.problems.push(AnalysisProblem::Error {
                            code: "INCOMPATIBLE_VALUE".to_owned(),
                            message: format!("Expected {} but got {}", output_description, value),
                            field: Some("value".to_owned()),
                        });
                    }
                } else {
                    analysis.problems.push(AnalysisProblem::Warning {
                        code: "UNEXPECTED_VALUE".to_owned(),
                        message: "This function has no output description".to_owned(),
                        field: Some("value".to_owned()),
                    });
                }
            } else if let Some(output_description) = &context.output_description {
                let value = Description::Null;
                if !value.is_compatible(output_description) {
                    analysis.problems.push(AnalysisProblem::Error {
                        code: "INCOMPATIBLE_VALUE".to_owned(),
                        message: format!(
                            "Expected {} but no value is returned",
                            output_description,
                        ),
                        field: None,
                    });
                }
            }

            context.ended = true;
        }
        Step::Break { .. } | Step::Continue { .. } => {
            if !context.inside_loop {
                analysis.problems.push(AnalysisProblem::Error {
                    code: "BREAK_OR_CONTINUE_OUTSIDE_LOOP".to_owned(),
                    message: "Break or continue outside loop".to_owned(),
                    field: None,
                });
            }
        }
        Step::Assign { value, to, .. } => {
            let value = parse_and_predict_description(
                value.clone(),
                &context.storage,
                &context.environment,
            );
            let (value, mut problems) = description_to_problems(value, Some("value".to_owned()));
            analysis.problems.append(&mut problems);

            let mut problems =
                store_in_storage(context, Some(to.clone()), value.unwrap_or_default());
            analysis.problems.append(&mut problems);
        }
        Step::Try { body, catch, .. } => {
            let mut catch_context = context.clone();
            catch_context.storage.insert(
                "error".to_owned(),
                Description::BaseType {
                    field_type: "object.error".to_owned(),
                },
            );
            for step in body {
                list.append(&mut analyze_step(context, step, previous));
            }

            for step in catch {
                list.append(&mut analyze_step(&mut catch_context, step, previous));
            }
            drop(catch_context);
        }
        Step::Throw { .. } => {
            context.ended = true;
        }
        Step::Parse {
            description,
            value,
            store_as,
            ..
        } => {
            // For parse we need to check:
            // - predict new storage
            // - parse description
            // We assume it can be parsed
            let value = parse_and_predict_description(
                value.clone(),
                &context.storage,
                &context.environment,
            );
            let (_value, mut problems) = description_to_problems(value, Some("value".to_owned()));
            analysis.problems.append(&mut problems);

            let description = parse_and_evaluate_description_notation(description);
            let (description, mut problems) =
                description_to_problems(description, Some("description".to_owned()));
            analysis.problems.append(&mut problems);

            // TODO: Check if the value is compatible-ish with the description
            let mut problems = store_in_storage(
                context,
                Some(store_as.clone()),
                description.unwrap_or_default(),
            );
            analysis.problems.append(&mut problems);
        }
        Step::Transform {
            value,
            filter_by,
            order_by,
            map,
            store_as,
            ..
        } => {
            // Before we start, let's check if the value is OK
            let value = parse_and_predict_description(
                value.clone(),
                &context.storage,
                &context.environment,
            );
            let (value, mut problems) = description_to_problems(value, Some("value".to_owned()));
            analysis.problems.append(&mut problems);

            // Check filterBy
            if let Some(filter_by) = filter_by {
                if let Some(ref value) = value {
                    // Prepare context storage for the evaluation
                    let item_description = value.get_part();
                    let mut top: ObjectDescription = HashMap::new();
                    top.insert("value".to_owned(), item_description);
                    let storage = LayeredDescribedStorage {
                        top: &top,
                        down: &context.storage,
                    };

                    // Evaluate
                    let item_description = parse_and_predict_description(
                        filter_by.to_string(),
                        &storage,
                        &context.environment,
                    );
                    let (valid_filter, mut problems) =
                        description_to_problems(item_description, Some("filterBy".to_owned()));
                    analysis.problems.append(&mut problems);

                    if let Some(filter_output) = valid_filter {
                        let mut problems =
                            check_not_always_boolean(&filter_output, Some("filterBy".to_owned()));
                        analysis.problems.append(&mut problems);
                    }
                }
            }

            // Check orderBy
            if let Some(order_by) = order_by {
                if let Some(ref value) = value {
                    // Prepare context storage for the evaluation
                    let item_description = value.get_part();
                    let mut top: ObjectDescription = HashMap::new();
                    top.insert("a".to_owned(), item_description.clone());
                    top.insert("b".to_owned(), item_description);
                    let storage = LayeredDescribedStorage {
                        top: &top,
                        down: &context.storage,
                    };

                    // Evaluate
                    let item_description = parse_and_predict_description(
                        order_by.to_string(),
                        &storage,
                        &context.environment,
                    );
                    let (valid_order, mut problems) =
                        description_to_problems(item_description, Some("orderBy".to_owned()));
                    analysis.problems.append(&mut problems);

                    if let Some(order_output) = valid_order {
                        if !order_output.is_compatible(&Description::new_base_type("number")) {
                            analysis.problems.push(AnalysisProblem::Error {
                                code: "INVALID_FIELD".to_owned(),
                                message: "Order by must return a number".to_owned(),
                                field: Some("orderBy".to_owned()),
                            });
                        }
                    }
                }
            }

            // Check map, which will affect the output
            if let Some(map) = map {
                if let Some(value) = value {
                    // Prepare context storage for the evaluation
                    let item_description = value.get_part();
                    let mut top: ObjectDescription = HashMap::new();
                    top.insert("value".to_owned(), item_description);
                    let storage = LayeredDescribedStorage {
                        top: &top,
                        down: &context.storage,
                    };

                    // Evaluate
                    let item_description = parse_and_predict_description(
                        map.to_string(),
                        &storage,
                        &context.environment,
                    );
                    let value = Description::Array {
                        item_type: Box::new(item_description),
                        length: None,
                    };
                    let mut problems = store_in_storage(context, Some(store_as.clone()), value);
                    analysis.problems.append(&mut problems);
                } else {
                    let mut problems =
                        store_in_storage(context, Some(store_as.clone()), Description::Any);
                    analysis.problems.append(&mut problems);
                }
            } else {
                // Consider the output to be the same, except some items are missing or mixed
                if let Some(ref value) = value {
                    let item_description = value.get_part();
                    let value = Description::Array {
                        item_type: Box::new(item_description),
                        length: None,
                    };

                    let mut problems = store_in_storage(context, Some(store_as.clone()), value);
                    analysis.problems.append(&mut problems);
                } else {
                    let mut problems =
                        store_in_storage(context, Some(store_as.clone()), Description::Any);
                    analysis.problems.append(&mut problems);
                }
            }
        }
    }

    if capture_storage {
        analysis.after = Some(context.storage.clone());
    }

    list.push(analysis);
    list
}

fn description_to_problems(
    description: Description,
    field: Option<String>,
) -> (Option<Description>, Vec<AnalysisProblem>) {
    let mut problems = vec![];
    match description {
        Description::Error { error } => {
            problems.push(AnalysisProblem::Error {
                code: "ERROR_PREDICTED".to_owned(),
                message: error.to_string(),
                field,
            });
            (None, problems)
        }
        Description::Union { of } => {
            let mut union = vec![];
            for item in of {
                let (description, mut item_problems) =
                    description_to_problems(item.clone(), field.clone());
                problems.append(&mut item_problems);
                if let Some(description) = description {
                    union.push(description);
                }
            }
            (Some(Description::Union { of: union }), problems)
        }
        description => (Some(description), problems),
    }
}

/// Analyzes a list of steps and declarations
///
/// Returns a list of analysis results for each step
pub fn analyze_instructions(
    steps: &Steps,
    declarations: Vec<FunctionDeclaration>,
    steps_to_capture: Vec<String>,
    previous: Option<&Analysis>,
) -> Vec<StepAnalysis> {
    let mut list = vec![];
    for step in steps {
        let mut context = Context {
            storage: HashMap::new(),
            environment: FakeEnvironment::default(),
            inside_loop: false,
            resources: vec![],
            functions: declarations.clone(),
            ended: false,
            steps_to_capture: steps_to_capture.clone(),
            references: HashMap::new(),
            output_description: None,
            resource_events: vec![],
        };
        let mut output = analyze_step(&mut context, step, previous);
        list.append(&mut output);
    }
    list
}

#[cfg(test)]
mod test_analyzer {
    use super::*;

    fn test_context() -> Context {
        Context {
            storage: HashMap::new(),
            environment: FakeEnvironment::default(),
            inside_loop: false,
            resources: vec![],
            functions: vec![],
            ended: false,
            steps_to_capture: vec![],
            references: HashMap::new(),
            output_description: None,
            resource_events: vec![],
        }
    }

    #[test]
    fn test_simple() {
        let mut context = test_context();
        let steps = analyze_step(
            &mut context,
            &Step::Assign {
                id: "1".to_owned(),
                value: "\"hello world\"".to_owned(),
                to: "message".to_owned(),
                disabled: false,
            },
            None,
        );
        assert_eq!(steps.len(), 1);
        assert_eq!(steps.first().unwrap().problems.len(), 0);
    }

    #[test]
    fn test_assign_property() {
        let mut context = test_context();
        let steps = analyze_step(
            &mut context,
            &Step::Assign {
                id: "1".to_owned(),
                value: "5 5".to_owned(),
                to: "message".to_owned(),
                disabled: false,
            },
            None,
        );
        assert_eq!(steps.len(), 1);
        assert_eq!(steps.first().unwrap().problems.len(), 1);
    }
}

/**
 * We are skipping TypeScript generation to make our own super-cool type definitions
 */
#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT: &'static str = r#"
export type Step = any;

export type FunctionDeclaration = any;

export type AnalysisProblem = {
  type: 'error' | 'warning';
  code: string;
  message: string;
  field?: string;
};

export type StepAnalysis = {
  id: string;
  problems: AnalysisProblem[];
  after?: Record<string, Description>;
  before?: Record<string, Description>;
}

export function analyze(steps: Step[], declarations: FunctionDeclaration[], stepsToCapture: string[]): StepAnalysis[];

///
/// TEL beings here
///

export interface TelParseError {
  from: number;
  to: number;
  severity: string;
  message: string;
  actions: TelParseAction[];
}

export interface TelParseAction {
  name: string;
  code: string;
}

export type TelError =
  | {
      code: 'PARSE_ERROR';
      errors: TelParseError[];
    }
  | {
      code: 'CONVERSION_ERROR';
      message: string;
      from: string;
      to: string;
    }
  | {
      code: 'NOT_INDEXABLE';
      message: string;
      subject: string;
    }
  | {
      code: 'NO_ATTRIBUTE';
      message: string;
      subject: string;
      attribute: string;
    }
  | {
      code: 'INVALID_SELECTOR';
      message: string;
    }
  | {
      code: 'UNSUPPORTED_OPERATION';
      operation: string;
      message: string;
    }
  | {
      code: 'FUNCTION_NOT_FOUND';
      message: string;
    }
  | {
      code: 'INDEX_OUT_OF_BOUNDS';
      index: number;
      max: number;
    }
  | {
      code: 'INVALID_INDEX';
      subject: string;
      message: string;
    };

export type Description =
  | {
      type: 'null';
    }
  | {
      type: 'stringValue';
      value: string;
    }
  | {
      type: 'numberValue';
      value: number;
    }
  | {
      type: 'booleanValue';
      value: boolean;
    }
  | {
      type: 'object';
      value: Record<string, Description>;
    }
  | {
      type: 'exactArray';
      value: Description[];
    }
  | {
      type: 'array';
      length?: number;
      itemType: Description;
    }
  | {
      type: 'baseType';
      fieldType: string;
    }
  | {
      type: 'union';
      of: Description[];
    }
  | {
      type: 'error';
      error: TelError;
    }
  | {
      type: 'any';
    };

export interface BinaryOp {
  binaryOp: {
    lhs: SpannedExpr;
    op:
      | 'add'
      | 'subtract'
      | 'multiply'
      | 'divide'
      | 'modulo'
      | 'eq'
      | 'neq'
      | 'gt'
      | 'gte'
      | 'lt'
      | 'lte'
      | 'and'
      | 'or';
    rhs: SpannedExpr;
  };
}

export interface UnaryOp {
  unaryOp: {
    op: 'plus' | 'minus' | 'negation';
    expr: SpannedExpr;
  };
}

export interface MethodCall {
  methodCall: {
    callee: SpannedExpr;
    name: string;
    arguments: SpannedExpr[];
  };
}

export interface Attribute {
  attribute: {
    value: SpannedExpr;
    attribute: string;
  };
}

export interface Slice {
  slice: [SpannedExpr, SpannedExpr];
}

export interface Identifier {
  identifier: string;
}

export interface Environment {
  environment: string;
}

export interface Number {
  number: number;
}

export interface String {
  string: string;
}

export interface Boolean {
  boolean: boolean;
}

export interface Array {
  array: SpannedExpr[];
}

export interface Object {
  object: { [key: string]: SpannedExpr };
}

export interface If {
  if: {
    condition: SpannedExpr;
    then: SpannedExpr;
    otherwise: SpannedExpr;
  };
}

export type Expr =
  | {
      if: {
        condition: SpannedExpr;
        then: SpannedExpr;
        otherwise: SpannedExpr;
      };
    }
  | { number: number }
  | { string: string }
  | {
      multilineString: {
        value: string;
        tag: string;
      };
    }
  | { boolean: boolean }
  | { array: SpannedExpr[] }
  | { object: { [key: string]: SpannedExpr } }
  | { identifier: string }
  | { environment: string }
  | { attribute: [value: SpannedExpr, attribute: string] }
  | { slice: [SpannedExpr, SpannedExpr] }
  | { unaryOp: [op: 'plus' | 'minus' | 'negation', expr: SpannedExpr] }
  | {
      methodCall: {
        callee: SpannedExpr;
        name: string;
        arguments: SpannedExpr[];
      };
    }
  | {
      binaryOp: {
        lhs: SpannedExpr;
        op:
          | 'add'
          | 'subtract'
          | 'multiply'
          | 'divide'
          | 'modulo'
          | 'eq'
          | 'neq'
          | 'gt'
          | 'gte'
          | 'lt'
          | 'lte'
          | 'and'
          | 'or';
        rhs: SpannedExpr;
      };
    }
  | 'invalid'
  | 'null';

export type Range = { start: number; end: number };

export type SpannedExpr = [Expr, Range];

///
/// Description notation start
/// 
export type DExpr =
  | { number: number }
  | { string: string }
  | { boolean: boolean }
  | { array: DSpannedExpr[] }
  | { object: { [key: string]: DSpannedExpr } }
  | { identifier: string }
  | { attribute: [value: DSpannedExpr, attribute: string] }
  | {
      methodCall: {
        callee: DSpannedExpr;
        name: string;
        arguments: DSpannedExpr[];
      };
    }
  | {
      binaryOp: {
        lhs: DSpannedExpr;
        op:
          | 'and'
          | 'or';
        rhs: DSpannedExpr;
      };
    }
  | {
      arrayOf: DSpannedExpr;
    }
  | 'invalid'
  | 'null';

export type DRange = { start: number; end: number };

export type DSpannedExpr = [DExpr, DRange];

///
/// Description notation end
/// 

export type ParseResult = {
  expr?: SpannedExpr;
  errors: TelParseError[];
};

export type DescriptionParseResult = {
  expr?: DSpannedExpr;
  errors: TelParseError[];
};

/**
 * @param input - TEL expression / selector
 */
export function parse(input: string): ParseResult;

/**
 * @param input - Description notation
 * @returns Description object
 */
export function parseDescription(input: string): DescriptionParseResult;

/**
 * @param description - Description
 * @returns Description notation
 */
export function getNotation(description: Description): string;

export type EvaluationResult =
  | {
      type: 'success';
      value: any;
    }
  | {
      type: 'error';
      error: TelError;
    };

/**
 * @param expression - TEL expression
 * @param storage - Storage object
 * @param environment - Environment object
 * @returns Result object
 */
export function evaluateValue(
  expression: string,
  storage: any,
  environment: any,
): EvaluationResult;

export type DescriptionEvaluationResult = {
  value: Description;
};

/**
 * @param value - Storage value
 */
export function describe(value: any): Description;

/**
 * @param expression - TEL selector expression
 * @param storage - Storage description object
 * @param environment - Environment description object
 */
export function predictDescription(
  expression: string,
  storage: Record<string, Description>,
  environment: Record<string, Description>,
): DescriptionEvaluationResult;


/**
 * @param expression - Description notation
 */
export function evaluateDescription(
  expression: string,
): DescriptionEvaluationResult;

export type DescriptionStoreBranch =
  | {
      type: 'ok';
      storage: Record<string, Description>;
    }
  | {
      type: 'error';
      error: TelError;
    };

export type DescriptionStoreResult = {
  branches: DescriptionStoreBranch[];
};

/**
 * @param expression - TEL selector expression
 * @param storage - Storage description object
 * @param environment - Environment description object
 * @returns - Result object with possible branches
 */
export function predictStore(
  expression: string,
  storage: Record<string, Description>,
  environment: Record<string, Description>,
  value: Description,
): DescriptionStoreResult;

/**
 * @param selector - selector
 * @param storage - Storage description object
 * @returns - New described storage object
 */
export function storeDescription(
  selector: any[],
  storage: Record<string, Description>,
  value: Description,
): Record<string, Description>;

/**
 * @param selector - selector
 * @param storage - Storage object
 * @param value - value that is being added
 * @returns - New storage object
 */
export function storeValue(selector: any[], storage: Record<string, any>, value: any): Record<string, any>;
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum EvaluationResult {
    Success { value: StorageValue },
    Error { error: TelError },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub struct DescriptionEvaluationResult {
    pub value: Description,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub struct DescriptionNotationEvaluationResult {
    pub value: Description,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum DescriptionStoreBranch {
    Ok { storage: ObjectDescription },
    Error { error: TelError },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescriptionStoreResult {
    pub branches: Vec<DescriptionStoreBranch>,
}

#[wasm_bindgen(skip_typescript, js_name = parse)]
pub fn export_parse(input: &str) -> JsValue {
    let result = tel::parse(input);
    serialize(&result).expect("Could not serialize ParseResult")
}

#[wasm_bindgen(skip_typescript, js_name = parseDescription)]
pub fn export_parse_description(input: &str) -> JsValue {
    let result = tel::parse_description(input);
    serialize(&result).expect("Could not serialize ParseResult")
}

#[wasm_bindgen(skip_typescript, js_name = getNotation)]
pub fn get_notation(description: JsValue) -> JsValue {
    let description: Description =
        serde_wasm_bindgen::from_value(description).expect("Could not deserialize Description");

    let result = description.to_notation();
    serialize(&result).expect("Could not serialize String")
}

#[wasm_bindgen(skip_typescript, js_name = evaluateDescription)]
pub fn evaluate_description(input: &str) -> JsValue {
    let parse_result = tel::parse_description(input);
    let result: DescriptionEvaluationResult = match parse_result.expr {
        Some(expr) => {
            let output = tel::evaluate_description_notation(expr);
            match output {
                Ok(value) => DescriptionEvaluationResult { value },
                Err(error) => DescriptionEvaluationResult {
                    value: Description::Error { error },
                },
            }
        }
        None => DescriptionEvaluationResult {
            value: Description::Error {
                error: TelError::ParseError {
                    errors: parse_result.errors,
                },
            },
        },
    };

    serialize(&result).expect("Could not serialize DescriptionEvaluationResult")
}

#[wasm_bindgen(skip_typescript, js_name = describe)]
pub fn describe(storage_value: JsValue) -> JsValue {
    let storage_value: StorageValue =
        serde_wasm_bindgen::from_value(storage_value).expect("Could not deserialize StorageValue");

    let result: Description = tel::describe(storage_value);
    serialize(&result).expect("Could not serialize Description")
}

#[wasm_bindgen(skip_typescript, js_name = predictDescription)]
pub fn predict_description(input: &str, storage: JsValue, environment: JsValue) -> JsValue {
    let storage: ObjectDescription =
        serde_wasm_bindgen::from_value(storage).expect("Could not deserialize described storage");
    let environment: ObjectDescription = serde_wasm_bindgen::from_value(environment)
        .expect("Could not deserialize described environment");

    let parse_result = tel::parse(input);
    let result: DescriptionEvaluationResult = match parse_result.expr {
        Some(expr) => {
            let output = tel::predict_description(expr, &storage, &environment);
            DescriptionEvaluationResult { value: output }
        }
        None => DescriptionEvaluationResult {
            value: Description::Error {
                error: TelError::ParseError {
                    errors: parse_result.errors,
                },
            },
        },
    };

    serialize(&result).expect("Could not serialize DescriptionEvaluationResult")
}

#[wasm_bindgen(skip_typescript, js_name = predictStore)]
pub fn predict_store(
    input: &str,
    storage: JsValue,
    environment: JsValue,
    value: JsValue,
) -> JsValue {
    let storage: ObjectDescription =
        serde_wasm_bindgen::from_value(storage).expect("Could not deserialize described storage");
    let environment: ObjectDescription = serde_wasm_bindgen::from_value(environment)
        .expect("Could not deserialize described environment");
    let value: Description =
        serde_wasm_bindgen::from_value(value).expect("Could not deserialize described value");

    let parse_result = tel::parse(input);
    let result: DescriptionStoreResult = match parse_result.expr {
        Some(expr) => {
            let selector = tel::evaluate_selector_description(expr, &storage, &environment);
            let mut branches: Vec<DescriptionStoreBranch> = vec![];
            for selector in selector.into_iter() {
                match selector {
                    SelectorDescription::Static { selector } => {
                        let mut storage = storage.clone();
                        match tel::store_description(&selector, &mut storage, value.clone()) {
                            Ok(()) => branches.push(DescriptionStoreBranch::Ok { storage }),
                            Err(error) => branches.push(DescriptionStoreBranch::Error { error }),
                        }
                    }
                    SelectorDescription::Error { error } => {
                        branches.push(DescriptionStoreBranch::Error { error })
                    }
                    SelectorDescription::Unknown => {}
                }
            }
            DescriptionStoreResult { branches }
        }
        None => DescriptionStoreResult {
            branches: vec![DescriptionStoreBranch::Error {
                error: TelError::ParseError {
                    errors: parse_result.errors,
                },
            }],
        },
    };

    serialize(&result).expect("Could not serialize DescriptionStoreResult")
}

#[wasm_bindgen(skip_typescript, js_name = evaluateValue)]
pub fn evaluate_value(input: &str, storage: JsValue, environment: JsValue) -> JsValue {
    let storage: ObjectBody =
        serde_wasm_bindgen::from_value(storage).expect("Could not deserialize storage");
    let environment: ObjectBody =
        serde_wasm_bindgen::from_value(environment).expect("Could not deserialize environment");

    let parse_result = tel::parse(input);

    if !parse_result.errors.is_empty() {
        return serialize(&EvaluationResult::Error {
            error: TelError::ParseError {
                errors: parse_result.errors,
            },
        })
        .expect("Could not serialize EvaluationResult");
    }

    let result: EvaluationResult = match parse_result.expr {
        Some(expr) => {
            let output = tel::evaluate_value(expr, &storage, &environment);
            match output {
                Ok(value) => EvaluationResult::Success { value },
                Err(error) => EvaluationResult::Error { error },
            }
        }
        None => EvaluationResult::Error {
            error: TelError::ParseError { errors: vec![] },
        },
    };

    serialize(&result).expect("Could not serialize EvaluationResult")
}

#[wasm_bindgen(skip_typescript, js_name = storeValue)]
pub fn store_value(selector: JsValue, storage: JsValue, value: JsValue) -> JsValue {
    let selector: Selector =
        serde_wasm_bindgen::from_value(selector).expect("Could not deserialize selector");
    let mut storage: ObjectBody =
        serde_wasm_bindgen::from_value(storage).expect("Could not deserialize storage");
    let value: StorageValue =
        serde_wasm_bindgen::from_value(value).expect("Could not deserialize value");

    match tel::store_value(&selector, &mut storage, value) {
        Ok(()) => serialize(&storage).expect("Could not serialize new storage"),
        Err(error) => serialize(&error).expect("Could not serialize error"),
    }
}

#[wasm_bindgen(skip_typescript, js_name = storeDescription)]
pub fn store_description(selector: JsValue, storage: JsValue, value: JsValue) -> JsValue {
    let selector: Selector =
        serde_wasm_bindgen::from_value(selector).expect("Could not deserialize selector");
    let mut storage: ObjectDescription =
        serde_wasm_bindgen::from_value(storage).expect("Could not deserialize storage");
    let value: Description =
        serde_wasm_bindgen::from_value(value).expect("Could not deserialize described value");

    match tel::store_description(&selector, &mut storage, value) {
        Ok(()) => serialize(&storage).expect("Could not serialize new storage"),
        Err(error) => serialize(&error).expect("Could not serialize error"),
    }
}

#[wasm_bindgen(skip_typescript, js_name = analyze)]
pub fn analyze(steps: JsValue, declarations: JsValue, steps_to_capture: JsValue) -> JsValue {
    let steps: Vec<Step> =
        serde_wasm_bindgen::from_value(steps).expect("Could not deserialize steps");
    let declarations: Vec<FunctionDeclaration> =
        serde_wasm_bindgen::from_value(declarations).expect("Could not deserialize declarations");
    let steps_to_capture: Vec<String> = serde_wasm_bindgen::from_value(steps_to_capture)
        .expect("Could not deserialize steps to capture");

    let result = analyze_instructions(&steps, declarations, steps_to_capture, None);

    result
        .serialize(&SERIALIZER)
        .expect("Could not serialize analysis")
}

fn serialize<T: SerdeSerialize>(value: &T) -> Result<JsValue, serde_wasm_bindgen::Error> {
    value.serialize(&SERIALIZER)
}

#[wasm_bindgen(start, skip_typescript)]
pub fn main() {
    set_panic_hook();
}
