mod utils;

use serde::Serialize as SerdeSerialize;
use serde_derive::{Deserialize, Serialize};
use serde_wasm_bindgen::Serializer;
use tel::{
    Description, ObjectBody, ObjectDescription, Selector, SelectorDescription, StorageValue,
    TelError,
};
use utils::set_panic_hook;
use wasm_bindgen::prelude::*;

const SERIALIZER: Serializer = Serializer::new().serialize_maps_as_objects(true);

/**
 * We are skipping TypeScript generation to make our own super-cool type definitions
 */
#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT: &'static str = r#"
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
      type: 'unknown';
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

export type ParseResult = {
  expr?: SpannedExpr;
  errors: TelParseError[];
};

/**
 * @param input - TEL expression
 */
export function parse(input: string): ParseResult;

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
export function evaluateDescription(
  expression: string,
  storage: Record<string, Description>,
  environment: Record<string, Description>,
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
pub fn parse(input: &str) -> JsValue {
    let result = tel::parse(input);
    serialize(&result).expect("Could not serialize ParseResult")
}

#[wasm_bindgen(skip_typescript, js_name = describe)]
pub fn describe(storage_value: JsValue) -> JsValue {
    let storage_value: StorageValue =
        serde_wasm_bindgen::from_value(storage_value).expect("Could not deserialize StorageValue");

    let result: Description = tel::describe(storage_value);
    serialize(&result).expect("Could not serialize Description")
}

#[wasm_bindgen(skip_typescript, js_name = evaluateDescription)]
pub fn evaluate_description(input: &str, storage: JsValue, environment: JsValue) -> JsValue {
    let storage: ObjectDescription =
        serde_wasm_bindgen::from_value(storage).expect("Could not deserialize described storage");
    let environment: ObjectDescription = serde_wasm_bindgen::from_value(environment)
        .expect("Could not deserialize described environment");

    let parse_result = tel::parse(input);
    let result: DescriptionEvaluationResult = match parse_result.expr {
        Some(expr) => {
            let output = tel::evaluate_description(expr, &storage, &environment);
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

fn serialize<T: SerdeSerialize>(value: &T) -> Result<JsValue, serde_wasm_bindgen::Error> {
    value.serialize(&SERIALIZER)
}

#[wasm_bindgen(start, skip_typescript)]
pub fn main() {
    set_panic_hook();
}
