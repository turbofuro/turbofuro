use crate::{Expr, Selector, SelectorPart, Spanned, StorageValue, TelError};
use once_cell::sync::Lazy;
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, vec};

pub type ObjectDescription = HashMap<String, Description>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SelectorDescription {
    Static { selector: Selector },
    Error { error: TelError },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Description {
    Null,
    StringValue {
        value: String,
    },
    NumberValue {
        value: f64,
    },
    BooleanValue {
        value: bool,
    },
    Object {
        value: HashMap<String, Description>,
    },
    ExactArray {
        value: Vec<Description>,
    },
    Array {
        #[serde(rename = "itemType")]
        item_type: Box<Description>,
    },
    BaseType {
        #[serde(rename = "fieldType")]
        field_type: String,
    },
    Union {
        of: Vec<Description>,
    },
    Error {
        error: TelError,
    },
    Unknown,
    Any,
}

static OPERATORS: Lazy<HashMap<String, Description>> = Lazy::new(|| {
    let mut map: HashMap<String, Description> = HashMap::new();

    // Sum
    map.insert(
        "string_string_+".to_owned(),
        Description::new_base_type("string"),
    );
    for thing_summable_to_string in ["number", "boolean", "null", "any"] {
        map.insert(
            format!("{}_string_+", thing_summable_to_string),
            Description::new_base_type("string"),
        );
        map.insert(
            format!("string_{}_+", thing_summable_to_string),
            Description::new_base_type("string"),
        );
    }
    map.insert(
        "array_array_+".to_owned(),
        Description::new_base_type("array"),
    );
    map.insert(
        "array_any_+".to_owned(),
        Description::new_base_type("array"),
    );
    map.insert(
        "any_array_+".to_owned(),
        Description::new_base_type("array"),
    );
    map.insert(
        "object_object_+".to_owned(),
        Description::new_base_type("object"),
    );
    map.insert(
        "any_object_+".to_owned(),
        Description::new_base_type("object"),
    );
    map.insert(
        "object_any_+".to_owned(),
        Description::new_base_type("object"),
    );

    // Arithmetics
    map.insert(
        "number_number_+".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "number_number_-".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "number_number_*".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "number_number_/".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert("number_+".to_owned(), Description::new_base_type("number"));
    map.insert("number_-".to_owned(), Description::new_base_type("number"));

    // Arithmetics with any
    map.insert(
        "any_number_+".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "any_number_-".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "any_number_*".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "any_number_/".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "number_any_+".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "number_any_-".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "number_any_*".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert(
        "number_any_/".to_owned(),
        Description::new_base_type("number"),
    );
    map.insert("any_+".to_owned(), Description::new_base_type("number"));
    map.insert("any_-".to_owned(), Description::new_base_type("number"));

    map.insert(
        "boolean_!".to_owned(),
        Description::new_base_type("boolean"),
    );
    map.insert("any_!".to_owned(), Description::new_base_type("boolean"));

    // Equals
    for a in [
        "string", "number", "boolean", "object", "array", "null", "any",
    ] {
        for b in [
            "string", "number", "boolean", "object", "array", "null", "any",
        ] {
            map.insert(
                format!("{}_{}_==", a, b),
                Description::new_base_type("boolean"),
            );
            map.insert(
                format!("{}_{}_!=", a, b),
                Description::new_base_type("boolean"),
            );
            map.insert(
                format!("{}_{}_>", a, b),
                Description::new_base_type("boolean"),
            );
            map.insert(
                format!("{}_{}_>=", a, b),
                Description::new_base_type("boolean"),
            );
            map.insert(
                format!("{}_{}_<", a, b),
                Description::new_base_type("boolean"),
            );
            map.insert(
                format!("{}_{}_<=", a, b),
                Description::new_base_type("boolean"),
            );
        }
    }

    map.insert(
        "boolean_boolean_&&".to_owned(),
        Description::new_base_type("boolean"),
    );
    map.insert(
        "boolean_boolean_||".to_owned(),
        Description::new_base_type("boolean"),
    );
    map.insert(
        "any_boolean_&&".to_owned(),
        Description::new_base_type("boolean"),
    );
    map.insert(
        "any_boolean_||".to_owned(),
        Description::new_base_type("boolean"),
    );
    map.insert(
        "boolean_any_&&".to_owned(),
        Description::new_base_type("boolean"),
    );
    map.insert(
        "boolean_any_||".to_owned(),
        Description::new_base_type("boolean"),
    );

    map
});

static METHODS: Lazy<HashMap<&'static str, Description>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert("string_length", Description::new_base_type("number"));
    map.insert("string_type", Description::new_string("string".to_string()));
    map.insert("string_toString", Description::new_base_type("string"));
    map.insert("string_toNumber", Description::new_base_type("number"));
    map.insert("string_contains", Description::new_base_type("boolean"));
    map.insert("string_toUpperCase", Description::new_base_type("string"));
    map.insert("string_toLowerCase", Description::new_base_type("string"));
    map.insert("string_trim", Description::new_base_type("string"));
    map.insert("string_isEmpty", Description::new_base_type("boolean"));
    map.insert("string_startsWith", Description::new_base_type("boolean"));
    map.insert("number_toString", Description::new_base_type("string"));
    map.insert("number_type", Description::new_string("number".to_string()));
    map.insert("number_round", Description::new_base_type("number"));
    map.insert("number_floor", Description::new_base_type("number"));
    map.insert("number_ceil", Description::new_base_type("number"));
    map.insert("number_abs", Description::new_base_type("number"));
    map.insert("boolean_toString", Description::new_base_type("string"));
    map.insert(
        "boolean_type",
        Description::new_string("boolean".to_string()),
    );
    map.insert("array_type", Description::new_string("array".to_string()));
    map.insert("array_join", Description::new_base_type("string"));
    map.insert("array_contains", Description::new_base_type("boolean"));
    map.insert("object_type", Description::new_string("object".to_string()));
    map.insert("null_toString", Description::new_base_type("string"));
    map.insert("null_type", Description::new_string("null".to_string()));

    map
});

impl Description {
    pub fn new_string(value: String) -> Description {
        Description::StringValue { value }
    }

    pub fn new_number(value: f64) -> Description {
        Description::NumberValue { value }
    }

    pub fn new_error(error: TelError) -> Description {
        Description::Error { error }
    }

    pub fn new_base_type(field_type: &str) -> Description {
        Description::BaseType {
            field_type: field_type.to_owned(),
        }
    }

    pub fn to_form_string(&self) -> String {
        match self {
            Description::Null => "null".to_owned(),
            Description::StringValue { value } => "\"".to_owned() + value + "\"",
            Description::NumberValue { value } => value.to_string(),
            Description::BooleanValue { value } => value.to_string(),
            Description::Object { value } => {
                if value.is_empty() {
                    return "{}".to_owned();
                }

                let mut descriptions = Vec::new();
                for (k, v) in value {
                    descriptions.push(format!("{:?}: {}", k, v.to_form_string()));
                }
                format!("{{ {} }}", descriptions.join(", "))
            }
            Description::ExactArray { value } => {
                if value.is_empty() {
                    return "[]".to_owned();
                }

                let mut descriptions = Vec::new();
                for item in value {
                    descriptions.push(item.to_form_string());
                }
                format!("[{}]", descriptions.join(", "))
            }
            Description::Array { item_type } => {
                format!("{}[]", item_type.to_form_string())
            }
            Description::BaseType { field_type } => field_type.to_owned(),
            Description::Union { of } => {
                let mut descriptions = Vec::new();
                for item in of {
                    descriptions.push(item.to_form_string());
                }
                descriptions.join(" | ")
            }
            Description::Error { error: _ } => "error".to_owned(), // TODO
            Description::Unknown => "unknown".to_owned(),
            Description::Any => "any".to_owned(),
        }
    }

    pub fn optional(&self) -> Description {
        if matches!(self, Description::Null) {
            return self.clone();
        }
        Description::new_union(vec![self.clone(), Description::Null])
    }

    pub fn new_union(descriptions: Vec<Description>) -> Description {
        let description = descriptions.into_iter().reduce(merge);
        description.unwrap_or(Description::Unknown)
    }

    pub fn map<M>(&self, m: M) -> Description
    where
        M: Fn(&Description) -> Description,
    {
        match self {
            Description::Union { of } => {
                let mut descriptions = Vec::new();
                for item in of {
                    descriptions.push(m(item));
                }
                Description::new_union(descriptions)
            }
            Description::Error { error: _ } => self.clone(),
            _ => m(self),
        }
    }

    pub fn get_type(&self) -> String {
        match self {
            Description::StringValue { .. } => "string".to_owned(),
            Description::NumberValue { .. } => "number".to_owned(),
            Description::BooleanValue { .. } => "boolean".to_owned(),
            Description::Array { .. } => "array".to_owned(),
            Description::Object { .. } => "object".to_owned(),
            Description::Null => "null".to_owned(),
            Description::ExactArray { .. } => "array".to_owned(),
            Description::BaseType { field_type } => field_type.to_owned(),
            Description::Error { error: _ } => "error".to_owned(),
            Description::Any => "any".to_owned(),
            Description::Unknown => "unknown".to_owned(),
            Description::Union { of } => {
                let types = of.iter().map(|d| d.get_type()).collect::<Vec<String>>();

                types.join(" | ")
            }
        }
    }

    /**
     * Converts a description into potential index
     *
     * If the description is a valid index, it will return the index.
     * Otherwise it will return an error or an empty option if it could be potentially a valid index
     * but it could not be determined.
     */
    pub fn as_index(&self) -> Result<usize, Option<TelError>> {
        match self {
            Description::NumberValue { value: f } => Ok(*f as usize),
            Description::StringValue { value: s } => match s.parse::<usize>() {
                Ok(i) => Ok(i),
                Err(_) => Err(Some(TelError::InvalidIndex {
                    subject: "string".to_owned(),
                    message: "Can't use string as array index".to_owned(),
                })),
            },
            Description::BooleanValue { .. } => Err(Some(TelError::InvalidIndex {
                subject: "boolean".to_owned(),
                message: "Can't use boolean as array index".to_owned(),
            })),
            Description::Null => Err(Some(TelError::InvalidIndex {
                subject: "null".to_owned(),
                message: "Can't use null as array index".to_owned(),
            })),
            Description::Object { .. } => Err(Some(TelError::InvalidIndex {
                subject: "object".to_owned(),
                message: "Can't use object as array index".to_owned(),
            })),
            Description::ExactArray { .. } => Err(Some(TelError::InvalidIndex {
                subject: "array".to_owned(),
                message: "Can't use array as array index".to_owned(),
            })),
            Description::Array { .. } => Err(Some(TelError::InvalidIndex {
                subject: "array".to_owned(),
                message: "Can't use array as array index".to_owned(),
            })),
            Description::BaseType { field_type } => match field_type.as_str() {
                "string" => Err(None),
                "number" => Err(None),
                _ => Err(Some(TelError::InvalidIndex {
                    subject: field_type.to_owned(),
                    message: format!("Can't use {} as array index", field_type),
                })),
            },
            Description::Union { of: _ } => Err(None), // TODO: Improve prediction
            Description::Error { error } => Err(Some(error.to_owned())),
            Description::Unknown => Err(None),
            Description::Any => Err(None),
        }
    }

    /**
     * Converts a description into potential slice
     *
     * Like numbers[0] or headers["Content-Type"]
     *
     * If the description is a valid slice, it will return the slice part.
     * Otherwise it will return an error or unknown if it could be potentially a valid slice
     * but it could not be determined.
     */
    pub fn as_slice(&self) -> Vec<SelectorDescription> {
        match self {
            Description::Null => vec![SelectorDescription::Static {
                selector: vec![SelectorPart::Slice(StorageValue::Null(None))],
            }],
            Description::StringValue { value } => vec![SelectorDescription::Static {
                selector: vec![SelectorPart::Slice(StorageValue::String(value.clone()))],
            }],
            Description::NumberValue { value } => vec![SelectorDescription::Static {
                selector: vec![SelectorPart::Slice(StorageValue::Number(*value))],
            }],
            Description::BooleanValue { value } => vec![SelectorDescription::Static {
                selector: vec![SelectorPart::Slice(StorageValue::Boolean(*value))],
            }],
            Description::Object { .. } => vec![SelectorDescription::Error {
                error: TelError::InvalidSelector {
                    message: "Can't use object as slice".to_owned(),
                },
            }],
            Description::ExactArray { .. } => vec![SelectorDescription::Error {
                error: TelError::InvalidSelector {
                    message: "Can't use array as slice".to_owned(),
                },
            }],
            Description::Array { .. } => vec![SelectorDescription::Error {
                error: TelError::InvalidSelector {
                    message: "Can't use array as slice".to_owned(),
                },
            }],
            Description::BaseType { .. } => vec![SelectorDescription::Unknown],
            Description::Union { of } => {
                let mut descriptions = Vec::new();
                for item in of {
                    descriptions.append(&mut item.as_slice());
                }
                descriptions
            }
            Description::Error { error } => vec![SelectorDescription::Error {
                error: error.clone(),
            }],
            Description::Unknown => vec![SelectorDescription::Unknown],
            Description::Any => vec![SelectorDescription::Unknown],
        }
    }

    pub fn to_string(&self) -> Description {
        match self {
            Description::Null => Description::new_string("null".to_owned()),
            Description::StringValue { value } => Description::new_string(value.clone()),
            Description::NumberValue { value } => Description::new_string(value.to_string()),
            Description::BooleanValue { value } => Description::new_string(value.to_string()),
            Description::Object { value: _ } => Description::Error {
                error: TelError::UnsupportedOperation {
                    operation: "to_string_object".to_owned(),
                    message: "Can't convert object to string".to_owned(),
                },
            },
            Description::ExactArray { value: _ } => Description::Error {
                error: TelError::UnsupportedOperation {
                    operation: "to_string_array".to_owned(),
                    message: "Can't convert array to string".to_owned(),
                },
            },
            Description::Array { item_type: _ } => Description::Error {
                error: TelError::UnsupportedOperation {
                    operation: "to_string_array".to_owned(),
                    message: "Can't convert array to string".to_owned(),
                },
            },
            Description::BaseType { field_type } => match field_type.as_str() {
                "string" => Description::new_base_type("string"),
                "number" => Description::new_base_type("string"),
                "boolean" => Description::new_base_type("string"),
                "null" => Description::new_string("null".to_owned()),
                "object" => Description::Error {
                    error: TelError::UnsupportedOperation {
                        operation: "to_string_object".to_owned(),
                        message: "Can't convert object to string".to_owned(),
                    },
                },
                "array" => Description::Error {
                    error: TelError::UnsupportedOperation {
                        operation: "to_string_array".to_owned(),
                        message: "Can't convert array to string".to_owned(),
                    },
                },
                "string.uuid" => Description::new_base_type("string"),
                _ => Description::Unknown,
            },
            Description::Union { of } => {
                let mut descriptions = Vec::new();
                for item in of {
                    descriptions.push(item.to_string());
                }
                Description::new_union(descriptions)
            }
            Description::Any => Description::Any,
            Description::Unknown => Description::Unknown,
            Description::Error { error } => Description::Error {
                error: error.clone(),
            },
        }
    }

    pub fn to_boolean(&self) -> Description {
        match self {
            Description::Null => Description::BooleanValue { value: false },
            Description::StringValue { value } => Description::BooleanValue {
                value: !value.is_empty(),
            },
            Description::NumberValue { value } => Description::BooleanValue {
                value: *value != 0.0,
            },
            Description::BooleanValue { value } => Description::BooleanValue { value: *value },
            Description::Object { .. } => Description::Error {
                error: TelError::UnsupportedOperation {
                    operation: "to_boolean_object".to_owned(),
                    message: "Can't convert object to boolean".to_owned(),
                },
            },
            Description::ExactArray { .. } => Description::Error {
                error: TelError::UnsupportedOperation {
                    operation: "to_boolean_array".to_owned(),
                    message: "Can't convert array to boolean".to_owned(),
                },
            },
            Description::Array { .. } => Description::Error {
                error: TelError::UnsupportedOperation {
                    operation: "to_boolean_array".to_owned(),
                    message: "Can't convert array to boolean".to_owned(),
                },
            },
            Description::BaseType { field_type } => match field_type.as_str() {
                "string" => Description::new_base_type("boolean"),
                "number" => Description::new_base_type("boolean"),
                "boolean" => Description::new_base_type("boolean"),
                "object" => Description::Error {
                    error: TelError::UnsupportedOperation {
                        operation: "to_boolean_object".to_owned(),
                        message: "Can't convert object to boolean".to_owned(),
                    },
                },
                "array" => Description::Error {
                    error: TelError::UnsupportedOperation {
                        operation: "to_boolean_array".to_owned(),
                        message: "Can't convert array to boolean".to_owned(),
                    },
                },
                "string.uuid" => Description::BooleanValue { value: false },
                _ => Description::Unknown,
            },
            Description::Union { of } => {
                let mut descriptions = Vec::new();
                for item in of {
                    descriptions.push(item.to_boolean());
                }
                Description::new_union(descriptions)
            }
            Description::Error { error } => Description::Error {
                error: error.clone(),
            },
            Description::Unknown => Description::Unknown,
            Description::Any => Description::Any,
        }
    }

    /**
     * Converts a description to its base type.
     *
     * Base types are the most generic types that can be used to describe a value without an actual value.
     */
    pub fn to_base(&self) -> Description {
        match self {
            Description::Null => self.clone(),
            Description::StringValue { value } => {
                if value.starts_with("https://") || value.starts_with("http://") {
                    return Description::new_base_type("string.url");
                }
                Description::new_base_type("string")
            }
            Description::BooleanValue { value: _ } => Description::new_base_type("boolean"),
            Description::Object { value } => {
                let mut descriptions = HashMap::new();
                // TODO: Limit and pick at certain levels
                for (k, v) in value {
                    descriptions.insert(k.clone(), v.to_base());
                }
                Description::Object {
                    value: descriptions,
                }
            }
            Description::ExactArray { value } => {
                let mut descriptions = Vec::new();
                // TODO: Limit and pick at certain levels
                for item in value {
                    descriptions.push(item.to_base());
                }

                let unified = value
                    .clone()
                    .into_iter()
                    .reduce(|a, b| merge(a.to_base(), b.to_base()));

                if let Some(item_type) = unified {
                    Description::Array {
                        item_type: Box::new(item_type),
                    }
                } else {
                    Description::new_base_type("array.empty")
                }
            }
            Description::NumberValue { value: _ } => Description::new_base_type("number"),
            d => d.clone(),
        }
    }
}

/**
 * Provides literal description of a value.
 *
 * This is the entry to further type inference.
 */
pub fn describe(value: StorageValue) -> Description {
    match value {
        StorageValue::String(s) => Description::StringValue { value: s },
        StorageValue::Number(f) => Description::NumberValue { value: f },
        StorageValue::Boolean(b) => Description::BooleanValue { value: b },
        StorageValue::Array(array) => {
            let mut descriptions = Vec::new();
            // TODO: Smart limit so the resulting
            for item in array {
                descriptions.push(describe(item));
            }
            Description::ExactArray {
                value: descriptions,
            }
        }
        StorageValue::Object(object) => {
            let mut descriptions = HashMap::new();
            for (key, value) in object {
                descriptions.insert(key, describe(value));
            }
            Description::Object {
                value: descriptions,
            }
        }
        StorageValue::Null(_) => Description::Null,
    }
}

pub fn merge(a: Description, b: Description) -> Description {
    match (a, b) {
        (Description::Union { of: a }, Description::Union { of: b }) => {
            let mut descriptions = Vec::new();
            for item in a {
                descriptions.push(item);
            }
            for item in b {
                descriptions.push(item);
            }
            descriptions.dedup();

            // TODO: Compression

            Description::Union { of: descriptions }
        }
        (Description::Union { of: a }, b) => {
            let mut descriptions = Vec::new();
            for item in a {
                descriptions.push(item);
            }
            descriptions.push(b);
            descriptions.dedup();
            Description::Union { of: descriptions }
        }
        (a, Description::Union { of: b }) => {
            let mut descriptions = Vec::new();
            descriptions.push(a);
            for item in b {
                descriptions.push(item);
            }
            descriptions.dedup();
            Description::Union { of: descriptions }
        }
        (a, b) => {
            if a == b {
                return a;
            }
            Description::Union { of: vec![a, b] }
        }
    }
}

pub fn evaluate_description(
    expr: Spanned<Expr>,
    storage: &HashMap<String, Description>,
    environment: &HashMap<String, Description>,
) -> Description {
    match expr.0 {
        Expr::Null => Description::Null,
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            let value = evaluate_description(*condition, storage, environment);

            match value {
                Description::BooleanValue { value } => {
                    if value {
                        evaluate_description(*then, storage, environment)
                    } else {
                        evaluate_description(*otherwise, storage, environment)
                    }
                }
                Description::Any => Description::Union {
                    of: vec![
                        evaluate_description(*then, storage, environment),
                        evaluate_description(*otherwise, storage, environment),
                    ],
                },
                Description::BaseType { field_type } if field_type == "boolean" => {
                    Description::Union {
                        of: vec![
                            evaluate_description(*then, storage, environment),
                            evaluate_description(*otherwise, storage, environment),
                        ],
                    }
                }
                _ => Description::Error {
                    error: TelError::UnsupportedOperation {
                        operation: "if".to_owned(),
                        message: "Condition must be boolean".to_owned(),
                    },
                },
            }
        }
        Expr::Number(f) => Description::NumberValue { value: f },
        Expr::String(s) => Description::StringValue { value: s },
        Expr::MultilineString {
            value: data,
            tag: _,
        } => Description::StringValue { value: data },
        Expr::Boolean(b) => Description::BooleanValue { value: b },
        Expr::Array(n) => {
            let data: Vec<Description> = n
                .into_iter()
                .map(|e| evaluate_description(e, storage, environment))
                .collect();

            Description::ExactArray { value: data }
        }
        Expr::Object(n) => {
            let data: HashMap<String, Description> = n
                .into_iter()
                .map(|(k, v)| {
                    let v = evaluate_description(v, storage, environment);
                    (k, v)
                })
                .collect();

            Description::Object { value: data }
        }
        Expr::Identifier(iden) => match storage.get(&iden) {
            Some(v) => v.clone(),
            None => Description::Null,
        },
        Expr::Environment(iden) => match environment.get(&iden) {
            Some(v) => v.clone(),
            None => Description::Null,
        },
        Expr::Attribute(expr, attr) => {
            let expr = evaluate_description(*expr, storage, environment);

            expr.map(|expr| match expr {
                Description::Object { value } => match value.get(&attr) {
                    Some(v) => v.clone(),
                    None => Description::Null,
                },
                Description::ExactArray { value: vec } => match attr.parse::<usize>() {
                    Ok(i) => match vec.get(i) {
                        Some(v) => v.clone(),
                        None => Description::Null,
                    },
                    Err(_) => Description::Null,
                },
                Description::StringValue { value: s } => match attr.as_str() {
                    "length" => Description::NumberValue {
                        value: s.len() as f64,
                    },
                    _ => Description::Null,
                },
                Description::NumberValue { value: f } => match attr.as_str() {
                    "isInteger" => Description::BooleanValue {
                        value: f.fract() == 0.0,
                    },
                    _ => Description::Null,
                },
                Description::Array { item_type } => match attr.parse::<usize>() {
                    Ok(_i) => {
                        // TODO
                        Description::new_union(vec![*item_type.clone(), Description::Null])
                    }
                    Err(_) => Description::Null,
                },
                Description::Any => Description::Any,
                _ => Description::Null,
            })
        }
        Expr::Slice(expr, slice_expr) => {
            let expr = evaluate_description(*expr, storage, environment);
            let slice = evaluate_description(*slice_expr, storage, environment);

            expr.map(|expr| {
                slice.map(|slice| match expr {
                    Description::StringValue { value } => match slice.as_index() {
                        Ok(i) => {
                            let c = value.chars().nth(i);
                            match c {
                                Some(c) => Description::StringValue {
                                    value: c.to_string(),
                                },
                                None => Description::Null,
                            }
                        }
                        Err(error) => match error {
                            Some(e) => Description::new_error(e),
                            None => Description::Unknown,
                        },
                    },
                    Description::Object { value } => match slice.to_string() {
                        Description::StringValue { value: slice } => {
                            value.get(&slice).cloned().unwrap_or(Description::Null)
                        }
                        Description::BaseType { field_type } if field_type == "string" => {
                            Description::Any
                        }
                        Description::Any => Description::Any,
                        _ => Description::Unknown,
                    },
                    Description::ExactArray { value } => match slice.as_index() {
                        Ok(i) => value.get(i).cloned().unwrap_or(Description::Null),
                        Err(error) => match error {
                            Some(e) => Description::new_error(e),
                            None => Description::Unknown,
                        },
                    },
                    Description::Array { item_type } => item_type.optional(),
                    Description::BaseType { field_type } => match field_type.as_str() {
                        "string" => Description::new_base_type("string").optional(),
                        "object" => Description::Any,
                        "array" => Description::Any,
                        _ => Description::Error {
                            error: TelError::InvalidIndex {
                                subject: expr.get_type(),
                                message: format!("Can't use {} as array index", expr.get_type()),
                            },
                        },
                    },
                    Description::Union { of: _ } => {
                        unreachable!("Union should be handled above")
                    }
                    Description::Error { error: _ } => {
                        unreachable!("Error should be handled above")
                    }
                    Description::Unknown => Description::Unknown,
                    Description::Any => Description::Any,
                    _ => Description::Error {
                        error: TelError::InvalidIndex {
                            subject: expr.get_type(),
                            message: format!("Can't use {} as array index", expr.get_type()),
                        },
                    },
                })
            })
        }
        Expr::UnaryOp(op, expr) => {
            let expr = evaluate_description(*expr, storage, environment);

            expr.map(|expr| {
                let key = expr.get_type() + "_" + op.get_operator();

                OPERATORS
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or(Description::Error {
                        error: TelError::new_unary_unsupported(op, expr.to_owned()),
                    })
            })
        }
        Expr::BinaryOp { lhs, op, rhs } => {
            let l = evaluate_description(*lhs, storage, environment);
            let r = evaluate_description(*rhs, storage, environment);

            r.map(|r| {
                l.map(|l| {
                    let key = l.get_type() + "_" + r.get_type().as_str() + "_" + op.get_operator();

                    OPERATORS
                        .get(key.as_str())
                        .cloned()
                        .unwrap_or(Description::Error {
                            error: TelError::new_binary_unsupported(op, l.to_owned(), r.to_owned()),
                        })
                })
            })
        }
        Expr::MethodCall {
            callee,
            name,
            arguments: _,
        } => {
            let value = evaluate_description(*callee, storage, environment);

            value.map(|v| {
                let key = v.get_type() + "_" + name.as_str();
                METHODS
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or(Description::Null)
            })
        }
        Expr::Invalid => unreachable!(),
    }
}

pub fn evaluate_selector_description(
    expr: Spanned<Expr>,
    storage: &HashMap<String, Description>,
    environment: &HashMap<String, Description>,
) -> Vec<SelectorDescription> {
    match expr.0 {
        Expr::Null => vec![SelectorDescription::Static {
            selector: vec![SelectorPart::Null],
        }],
        Expr::Identifier(iden) => vec![SelectorDescription::Static {
            selector: vec![SelectorPart::Identifier(iden)],
        }],
        Expr::Attribute(expr, attr) => {
            let mut selectors = evaluate_selector_description(*expr, storage, environment);
            for selector in selectors.iter_mut() {
                if let SelectorDescription::Static { selector } = selector {
                    selector.push(SelectorPart::Attribute(attr.clone()))
                }
            }
            selectors
        }
        Expr::Slice(expr, slice_expr) => {
            let selectors = evaluate_selector_description(*expr, storage, environment);
            let value = evaluate_description(*slice_expr, storage, environment);

            let mut combined_selectors = Vec::new();
            let mut value_selector_branches = value.as_slice();

            for branch in value_selector_branches.iter_mut() {
                match branch {
                    SelectorDescription::Static {
                        selector: added_selector,
                    } => {
                        for selector in selectors.iter() {
                            if let SelectorDescription::Static { selector } = selector {
                                let mut combined_selector = selector.clone();
                                combined_selector.append(&mut added_selector.clone());
                                combined_selectors.push(SelectorDescription::Static {
                                    selector: combined_selector,
                                });
                            }
                        }
                    }
                    SelectorDescription::Error { error } => {
                        combined_selectors.push(SelectorDescription::Error {
                            error: error.clone(),
                        });
                    }
                    SelectorDescription::Unknown => {
                        combined_selectors.push(SelectorDescription::Unknown);
                    }
                }
            }
            combined_selectors
        }
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            let value = evaluate_description(*condition, storage, environment);

            match value {
                Description::BooleanValue { value } => {
                    if value {
                        evaluate_selector_description(*then, storage, environment)
                    } else {
                        evaluate_selector_description(*otherwise, storage, environment)
                    }
                }
                Description::Any => {
                    let mut if_then = evaluate_selector_description(*then, storage, environment);
                    let mut if_else =
                        evaluate_selector_description(*otherwise, storage, environment);
                    if_then.append(&mut if_else);
                    if_then
                }
                Description::BaseType { field_type } if field_type == "boolean" => {
                    let mut if_then = evaluate_selector_description(*then, storage, environment);
                    let mut if_else =
                        evaluate_selector_description(*otherwise, storage, environment);
                    if_then.append(&mut if_else);
                    if_then
                }
                Description::Unknown => vec![SelectorDescription::Unknown],
                e => {
                    vec![SelectorDescription::Error {
                        error: TelError::InvalidSelector {
                            message: format!("Invalid selector containing: {:?}", e),
                        },
                    }]
                }
            }
        }
        e => vec![SelectorDescription::Error {
            error: TelError::InvalidSelector {
                message: format!("Invalid selector containing: {:?}", e),
            },
        }],
    }
}

enum ContextStorage<'a> {
    Object(&'a mut HashMap<String, Description>),
    Array(&'a mut Vec<Description>),
    SimpleArray(&'a mut Box<Description>),
}

pub fn save_to_storage_description(
    selectors: &Vec<SelectorPart>,
    storage: &mut HashMap<String, Description>,
    value: Description,
) -> Result<(), TelError> {
    let remaining = selectors.len();
    let mut traversed = ContextStorage::Object(storage);

    for (index, selector) in selectors.iter().enumerate() {
        let last = index == remaining - 1;
        match (selector, last) {
            // This is last element so we need to apply the modification
            (part, true) => match part {
                SelectorPart::Identifier(p) => match traversed {
                    ContextStorage::Object(obj) => {
                        obj.insert(p.to_owned(), value);
                        return Ok(());
                    }
                    ContextStorage::Array(_) | ContextStorage::SimpleArray(_) => {
                        unreachable!()
                    }
                },
                SelectorPart::Attribute(attr) => match traversed {
                    ContextStorage::Object(obj) => {
                        obj.insert(attr.to_owned(), value);
                        return Ok(());
                    }
                    ContextStorage::Array(_) | ContextStorage::SimpleArray(_) => {
                        return Err(TelError::NoAttribute {
                            message: format!("array has no attribute {}", attr),
                            subject: "array".to_owned(),
                            attribute: attr.to_string(),
                        });
                    }
                },
                SelectorPart::Slice(slice) => match traversed {
                    ContextStorage::Object(obj) => {
                        obj.insert(slice.to_string()?, value);
                        return Ok(());
                    }
                    ContextStorage::Array(arr) => {
                        let index = slice.as_index()?;
                        let length = arr.len();
                        if index >= length {
                            return Err(TelError::IndexOutOfBounds {
                                index,
                                max: length - 1,
                            });
                        }
                        arr[index] = value;
                        return Ok(());
                    }
                    ContextStorage::SimpleArray(item_type) => {
                        *item_type = Box::new(merge(value, *item_type.clone()));
                        return Ok(());
                    }
                },
                SelectorPart::Null => return Ok(()),
            },
            // This is not the last element so we need to traverse further
            (part, false) => match part {
                SelectorPart::Identifier(identifier) => match traversed {
                    ContextStorage::Object(obj) => match obj.get_mut(identifier) {
                        Some(value) => match value {
                            Description::Object { value } => {
                                traversed = ContextStorage::Object(value);
                            }
                            Description::ExactArray { value } => {
                                traversed = ContextStorage::Array(value);
                            }
                            Description::Array { item_type } => {
                                traversed = ContextStorage::SimpleArray(item_type);
                            }
                            sv => {
                                return Err(TelError::InvalidSelector {
                                    message: format!(
                                        "{} is not a writeable storage",
                                        sv.get_type()
                                    ),
                                })
                            }
                        },
                        None => {
                            return Err(TelError::NotIndexable {
                                message: "null is not indexable".to_owned(),
                                subject: "null".to_owned(),
                            })
                        }
                    },
                    ContextStorage::Array(_) | ContextStorage::SimpleArray(_) => {
                        unreachable!()
                    }
                },
                SelectorPart::Attribute(attr) => match traversed {
                    ContextStorage::Object(obj) => match obj.get_mut(attr) {
                        Some(value) => match value {
                            Description::Object { value } => {
                                traversed = ContextStorage::Object(value);
                            }
                            Description::ExactArray { value } => {
                                traversed = ContextStorage::Array(value);
                            }
                            Description::Array { item_type } => {
                                traversed = ContextStorage::SimpleArray(item_type);
                            }
                            sv => {
                                return Err(TelError::InvalidSelector {
                                    message: format!(
                                        "{} is not a writeable storage",
                                        sv.get_type()
                                    ),
                                })
                            }
                        },
                        None => {
                            return Err(TelError::NoAttribute {
                                attribute: attr.to_string(),
                                subject: "object".to_owned(),
                                message: format!("object has no attribute {}", attr),
                            })
                        }
                    },
                    ContextStorage::Array(_) | ContextStorage::SimpleArray(_) => {
                        return Err(TelError::NoAttribute {
                            attribute: attr.to_string(),
                            subject: "array".to_owned(),
                            message: format!("array has no attribute {}", attr),
                        })
                    }
                },
                SelectorPart::Slice(slice) => match traversed {
                    ContextStorage::Object(obj) => {
                        let key = slice.to_string()?;
                        match obj.get_mut(&key) {
                            Some(value) => match value {
                                Description::Object { value } => {
                                    traversed = ContextStorage::Object(value);
                                }
                                Description::ExactArray { value } => {
                                    traversed = ContextStorage::Array(value);
                                }
                                Description::Array { item_type } => {
                                    traversed = ContextStorage::SimpleArray(item_type);
                                }
                                sv => {
                                    return Err(TelError::InvalidSelector {
                                        message: format!(
                                            "{} is not a writeable storage",
                                            sv.get_type()
                                        ),
                                    })
                                }
                            },
                            None => {
                                return Err(TelError::NoAttribute {
                                    attribute: key.to_string(),
                                    subject: "object".to_owned(),
                                    message: format!("object has no attribute {}", key),
                                })
                            }
                        }
                    }
                    ContextStorage::Array(arr) => {
                        let index = slice.as_index()?;
                        match arr.get_mut(index) {
                            Some(arr_item) => match arr_item {
                                Description::Object { value } => {
                                    traversed = ContextStorage::Object(value);
                                }
                                Description::ExactArray { value } => {
                                    traversed = ContextStorage::Array(value);
                                }
                                Description::Array { item_type } => {
                                    traversed = ContextStorage::SimpleArray(item_type);
                                }
                                sv => {
                                    return Err(TelError::InvalidSelector {
                                        message: format!(
                                            "{} is not a writeable storage",
                                            sv.get_type()
                                        ),
                                    })
                                }
                            },
                            None => {
                                return Err(TelError::NoAttribute {
                                    attribute: index.to_string(),
                                    subject: "array".to_owned(),
                                    message: format!("array has no element at index {}", index),
                                })
                            }
                        }
                    }
                    ContextStorage::SimpleArray(item_type) => {
                        let _index = slice.as_index()?; // TODO: Check if index could be used for better data

                        match item_type.as_mut() {
                            Description::Object { value } => {
                                traversed = ContextStorage::Object(value);
                            }
                            Description::ExactArray { value } => {
                                traversed = ContextStorage::Array(value);
                            }
                            Description::Array { item_type } => {
                                traversed = ContextStorage::SimpleArray(item_type);
                            }
                            sv => {
                                return Err(TelError::InvalidSelector {
                                    message: format!(
                                        "{} is not a writeable storage",
                                        sv.get_type()
                                    ),
                                })
                            }
                        }
                    }
                },
                SelectorPart::Null => return Ok(()),
            },
        }
    }
    panic!("Should not happen, right?")
}

#[cfg(test)]
mod test_description {
    use std::vec;

    use crate::{parse, storage_value};

    use super::*;

    #[test]
    fn test_deduplicates() {
        let description = describe(StorageValue::Array(vec![
            StorageValue::String("a".to_owned()),
            StorageValue::String("b".to_owned()),
            StorageValue::String("c".to_owned()),
        ]));

        assert_eq!(
            description.to_base(),
            Description::Array {
                item_type: Box::new(Description::BaseType {
                    field_type: "string".to_owned()
                })
            }
        )
    }

    #[test]
    fn test_serialization() {
        let description = serde_json::to_string(&Description::new_base_type("number")).unwrap();
        assert_eq!(description, r#"{"type":"baseType","fieldType":"number"}"#);

        let description = serde_json::to_string(&Description::new_union(vec![
            Description::new_base_type("number"),
            Description::new_base_type("string"),
        ]))
        .unwrap();
        assert_eq!(
            description,
            r#"{"type":"union","of":[{"type":"baseType","fieldType":"number"},{"type":"baseType","fieldType":"string"}]}"#
        )
    }

    fn apply(
        input: &str,
        value: Description,
        storage: &mut ObjectDescription,
        environment: &HashMap<String, Description>,
    ) {
        let result = parse(input);

        if let Some(expr) = result.expr {
            let mut selector = evaluate_selector_description(expr, storage, environment);
            assert_eq!(selector.len(), 1);

            let selector = selector.remove(0);
            match selector {
                SelectorDescription::Static { selector } => {
                    save_to_storage_description(&selector, storage, value).unwrap();
                }
                _ => {
                    panic!("Should not happen");
                }
            }
        }
    }

    #[test]
    fn test_save_description() {
        let mut storage = HashMap::new();
        let environment = HashMap::new();

        apply(
            "test",
            describe(storage_value!({ "a": 4 })),
            &mut storage,
            &environment,
        );

        let current_test = storage.get("test").unwrap().clone();
        assert_eq!(current_test, describe(storage_value!({ "a": 4 })))
    }

    #[test]
    fn test_save_description2() {
        let mut storage = HashMap::new();
        let environment = HashMap::new();

        apply(
            "test",
            describe(storage_value!({ "a": 4 })),
            &mut storage,
            &environment,
        );

        apply(
            "test.a",
            describe(StorageValue::Number(6.0)),
            &mut storage,
            &environment,
        );

        let current_test = storage.get("test").unwrap().clone();
        assert_eq!(current_test, describe(storage_value!({ "a": 6.0 })))
    }
}
