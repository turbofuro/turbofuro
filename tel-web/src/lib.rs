mod utils;

use serde::Serialize as SerdeSerialize;
use serde_derive::{Deserialize, Serialize};
use serde_wasm_bindgen::Serializer;
use tel::{ObjectBody, Selector, StorageValue, TelError};
use utils::set_panic_hook;
use wasm_bindgen::prelude::*;

const SERIALIZER: Serializer = Serializer::new().serialize_maps_as_objects(true);

/**
 * We are skipping TypeScript generation to make our own super-cool type definitions
 */
#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT: &'static str = r#"

export interface BinaryOp {
  binaryOp: {
    lhs: Spanned<Expr>;
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
    rhs: Spanned<Expr>;
  };
}

export interface UnaryOp {
  unaryOp: {
    op: 'plus' | 'minus' | 'negation';
    expr: Spanned<Expr>;
  };
}

export interface MethodCall {
  methodCall: {
    callee: Spanned<Expr>;
    name: string;
    arguments: Spanned<Expr>[];
  };
}

export interface Attribute {
  attribute: {
    value: Spanned<Expr>;
    attribute: string;
  };
}

export interface Slice {
  slice: [Spanned<Expr>, Spanned<Expr>];
}

export interface Identifier {
  identifier: string;
}

export interface Environment {
  environment: string;
}

export interface Int {
  int: number;
}

export interface Float {
  float: number;
}

export interface String {
  string: string;
}

export interface Boolean {
  boolean: boolean;
}

export interface Array {
  array: Spanned<Expr>[];
}

export interface Object {
  object: { [key: string]: Spanned<Expr> };
}

export interface Null {
  null: null;
}

export interface Invalid {
  invalid: null;
}

export interface If {
  if: {
    condition: Spanned<Expr>;
    then: Spanned<Expr>;
    otherwise: Spanned<Expr>;
  };
}

export type Expr =
  | { null: null }
  | {
      if: {
        condition: Spanned<Expr>;
        then: Spanned<Expr>;
        otherwise: Spanned<Expr>;
      };
    }
  | { int: number }
  | { float: number }
  | { string: string }
  | {
      multilineString: {
        value: string;
        tag: string;
      };
    }
  | { boolean: boolean }
  | { array: Spanned<Expr>[] }
  | { object: { [key: string]: Spanned<Expr> } }
  | { identifier: string }
  | { environment: string }
  | { attribute: [value: Spanned<Expr>, attribute: string] }
  | { slice: [Spanned<Expr>, Spanned<Expr>] }
  | { unaryOp: [op: 'plus' | 'minus' | 'negation', expr: Spanned<Expr>] }
  | {
      methodCall: {
        callee: Spanned<Expr>;
        name: string;
        arguments: Spanned<Expr>[];
      };
    }
  | {
      binaryOp: {
        lhs: Spanned<Expr>;
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
        rhs: Spanned<Expr>;
      };
    }
  | 'invalid';

export type Range = { start: number; end: number };

export type SpannedExpr = [Expr, Range];

export type ParseError = {
  message: string;
  from: number;
  to: number;
  actions: string[];
};

export type ParseResult = {
  expr?: SpannedExpr;
  errors: ParseError[];
};

/**
 * @param {string} input
 */
export function parseWithMetadata(input: string): ParseResult;

export type EvaluationResult =
  | {
      type: 'success';
      value: any;
    }
  | {
      type: 'error';
      message: string;
    };

/**
 * @param {string} input
 * @param {any} storage
 * @param {any} environment
 * @returns {any}
 */
export function evaluateValue(
  input: string,
  storage: any,
  environment: any,
): EvaluationResult;

/**
 * @param {array} TEL selector
 * @param {any} storage - storage object
 * @param {any} value - value that is being added
 * @returns {any}
 */
export function saveToStorage(
  selector: any,
  storage: any,
  value: any,
): EvaluationResult;
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum EvaluationResult {
    Success { value: StorageValue },
    Error { error: TelError },
}

#[wasm_bindgen(skip_typescript, js_name = parseWithMetadata)]
pub fn parse(input: &str) -> JsValue {
    let result = tel::parse(input);
    serialize(&result).expect("Could not serialize ParseResult")
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

#[wasm_bindgen(skip_typescript, js_name = saveToStorage)]
pub fn save_to_storage(selector: JsValue, storage: JsValue, value: JsValue) -> JsValue {
    let selector: Selector =
        serde_wasm_bindgen::from_value(selector).expect("Could not deserialize selector");
    let mut storage: ObjectBody =
        serde_wasm_bindgen::from_value(storage).expect("Could not deserialize storage");
    let value: StorageValue =
        serde_wasm_bindgen::from_value(value).expect("Could not deserialize value");

    match tel::save_to_storage(&selector, &mut storage, value) {
        Ok(()) => serialize(&storage).expect("Could not serialize new storage"),
        Err(error) => serialize(&error).expect("Could not save to storage"),
    }
}

fn serialize<T: SerdeSerialize>(value: &T) -> Result<JsValue, serde_wasm_bindgen::Error> {
    value.serialize(&SERIALIZER)
}

#[wasm_bindgen(start, skip_typescript)]
pub fn main() {
    set_panic_hook();
}
