use crate::{Expr, Selector, SelectorPart, Spanned, StorageValue, TelError, NULL};
use once_cell::sync::Lazy;
use serde::Serializer;
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Display};
use url::Url;

pub type ObjectDescription = HashMap<String, Description>;

pub trait DescribedStorage {
    fn get(&self, key: &str) -> Option<&Description>;
}

impl DescribedStorage for ObjectDescription {
    fn get(&self, key: &str) -> Option<&Description> {
        self.get(key)
    }
}

pub trait DescribedEnvironment {
    fn get(&self, key: &str) -> Option<&Description>;
}

impl DescribedEnvironment for ObjectDescription {
    fn get(&self, key: &str) -> Option<&Description> {
        self.get(key)
    }
}
/// Layered storage for evaluating expressions with multiple storages
/// without having to duplicate the storages
#[derive(Debug)]
pub struct LayeredDescribedStorage<'a, 'b, T: DescribedStorage> {
    pub top: &'a T,
    pub down: &'b T,
}

impl DescribedStorage for LayeredDescribedStorage<'_, '_, ObjectDescription> {
    fn get(&self, key: &str) -> Option<&Description> {
        self.top.get(key).or_else(|| self.down.get(key))
    }
}

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
        #[serde(serialize_with = "serialize_number")]
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
        #[serde(skip_serializing_if = "Option::is_none")]
        length: Option<usize>,
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
    Any,
}

impl Description {
    pub fn error(&self) -> Option<TelError> {
        match self {
            Description::Error { error } => Some(error.clone()),
            _ => None,
        }
    }

    pub fn get_part(&self) -> Description {
        match self {
            Description::Array {
                item_type,
                length: _,
            } => *item_type.clone(),
            Description::StringValue { value: _ } => Description::new_base_type("string.char"),
            Description::BaseType { field_type } => {
                if field_type.starts_with("array") {
                    // TODO: Better array detection
                    Description::Any
                } else if field_type.starts_with("string") {
                    Description::new_base_type("string.char")
                } else {
                    Description::Any
                }
            }
            Description::ExactArray { value } => Description::Union { of: value.clone() },
            _ => Description::Any,
        }
    }

    pub fn is_compatible(&self, other: &Description) -> bool {
        if (self == other)
            || (other == &Description::Any
                || matches!(
                    other,
                    Description::BaseType {
                        field_type
                    } if field_type == "any"
                ))
        {
            return true;
        }

        if matches!(self, Description::Error { error: _ })
            || matches!(other, Description::Error { error: _ })
        {
            return false;
        }

        match self {
            Description::Null => match other {
                Description::Null => true,
                Description::BaseType { field_type } => field_type.starts_with("null"), // TODO: Emit warning if it doesn't exact match
                Description::Union { of } => of.iter().any(|d| self.is_compatible(d)),
                _ => false,
            },
            Description::StringValue { value } => match other {
                Description::StringValue { value: other } => value == other,
                Description::BaseType { field_type } => field_type.starts_with("string"), // TODO: Emit warning if it doesn't exact match
                Description::Union { of } => of.iter().any(|d| self.is_compatible(d)),
                _ => false,
            },
            Description::NumberValue { value } => match other {
                Description::NumberValue { value: other } => value == other,
                Description::BaseType { field_type } => field_type.starts_with("number"), // TODO: Emit warning if it doesn't exact match
                Description::Union { of } => of.iter().any(|d| self.is_compatible(d)),
                _ => false,
            },
            Description::BooleanValue { value } => match other {
                Description::BooleanValue { value: other } => value == other,
                Description::BaseType { field_type } => field_type.starts_with("boolean"), // TODO: Emit warning if it doesn't exact match
                Description::Union { of } => of.iter().any(|d| self.is_compatible(d)),
                _ => false,
            },
            Description::Object { value } => match other {
                Description::Object { value: other } => {
                    for (key, other_item) in other {
                        if let Some(item) = value.get(key) {
                            // Check if the item is compatible
                            if !item.is_compatible(other_item) {
                                return false;
                            }
                        } else {
                            // The key doesn't exist and the item is required
                            if !Description::Null.is_compatible(other_item) {
                                return false;
                            }
                        }
                    }
                    true
                }
                Description::BaseType { field_type } => field_type.starts_with("object"), // TODO: Emit warning if it doesn't exact match
                Description::Union { of } => of.iter().any(|d| self.is_compatible(d)),
                _ => false,
            },
            Description::ExactArray { value } => match other {
                Description::ExactArray { value: other } => {
                    for (i, item) in value.iter().enumerate() {
                        if !item.is_compatible(&other[i]) {
                            return false;
                        }
                    }
                    true
                }
                Description::BaseType { field_type } => field_type.starts_with("array"), // TODO: Emit warning if it doesn't exact match
                Description::Union { of } => of.iter().any(|d| self.is_compatible(d)),
                _ => false,
            },
            Description::Array { item_type, .. } => match other {
                Description::Array {
                    item_type: other_item_type,
                    ..
                } => item_type.is_compatible(other_item_type),
                Description::BaseType { field_type } => field_type.starts_with("array"), // TODO: Emit warning if it doesn't exact match
                Description::Union { of } => of.iter().any(|d| self.is_compatible(d)),
                _ => false,
            },
            Description::BaseType { field_type } => match other {
                Description::BaseType { field_type: other } => {
                    let t = field_type.split(".").collect::<Vec<&str>>();
                    let o = other.split(".").collect::<Vec<&str>>();
                    t.first() == o.first()
                }
                _ => false,
            },
            Description::Union { of } => {
                for item in of {
                    if !item.is_compatible(other) {
                        return false;
                    }
                }
                true
            }
            Description::Error { error: _ } => false,
            Description::Any => true,
        }
    }
}

impl Default for Description {
    fn default() -> Self {
        Self::Any
    }
}

impl Display for Description {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_notation())
    }
}

impl From<StorageValue> for Description {
    fn from(item: StorageValue) -> Self {
        describe(item)
    }
}

/// Custom serializer for StorageValue::Number
///
/// Serializes as i64 if no fractional part, otherwise as f64
/// mimics the behavior of ECMAScript Number type
pub fn serialize_number<S>(num: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if num.fract() == 0.0 {
        // Serialize as i64 if no fractional part
        serializer.serialize_i64(*num as i64)
    } else {
        // Serialize as f64 otherwise
        serializer.serialize_f64(*num)
    }
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

    // Equals and logical
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

            // The || operator returns a union of positive types from left and the last type
            let mut possibilities = vec![Description::new_base_type(b)];
            if let Some(only_positive) = Description::new_base_type(a).only_positive() {
                possibilities.insert(0, only_positive);
            }

            map.insert(
                format!("{}_{}_||", a, b),
                Description::new_union(possibilities),
            );
            map.insert(format!("{}_{}_&&", a, b), Description::new_base_type(b));

            // Only implemented for string, number, boolean, but any will work (just no ordering)
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
    map.insert("string_endsWith", Description::new_base_type("boolean"));
    map.insert("string_stripPrefix", Description::new_base_type("string"));
    map.insert("string_stripSuffix", Description::new_base_type("string"));
    map.insert("string_replace", Description::new_base_type("string"));
    map.insert("number_toString", Description::new_base_type("string"));
    map.insert("number_type", Description::new_string("number".to_string()));
    map.insert("number_round", Description::new_base_type("number"));
    map.insert("number_floor", Description::new_base_type("number"));
    map.insert("number_ceil", Description::new_base_type("number"));
    map.insert("number_abs", Description::new_base_type("number"));
    map.insert("number_pow", Description::new_base_type("number"));
    map.insert("number_cos", Description::new_base_type("number"));
    map.insert("number_sin", Description::new_base_type("number"));
    map.insert("number_tan", Description::new_base_type("number"));
    map.insert("number_acos", Description::new_base_type("number"));
    map.insert("number_asin", Description::new_base_type("number"));
    map.insert("number_atan", Description::new_base_type("number"));
    map.insert("number_sqrt", Description::new_base_type("number"));
    map.insert("number_exp", Description::new_base_type("number"));
    map.insert("number_ln", Description::new_base_type("number"));
    map.insert("number_log", Description::new_base_type("number"));
    map.insert("number_log10", Description::new_base_type("number"));
    map.insert("number_sign", Description::new_base_type("number"));
    map.insert("number_isInteger", Description::new_base_type("boolean"));
    map.insert("number_isNaN", Description::new_base_type("boolean"));
    map.insert("number_isFinite", Description::new_base_type("boolean"));
    map.insert("number_isInfinite", Description::new_base_type("boolean"));
    map.insert("number_isEven", Description::new_base_type("boolean"));
    map.insert("number_isOdd", Description::new_base_type("boolean"));
    map.insert("number_toNumber", Description::new_base_type("number"));
    map.insert("number_truncate", Description::new_base_type("number"));
    map.insert("boolean_toString", Description::new_base_type("string"));
    map.insert(
        "boolean_type",
        Description::new_string("boolean".to_string()),
    );
    map.insert("array_type", Description::new_string("array".to_string()));
    map.insert("array_join", Description::new_base_type("string"));
    map.insert("array_contains", Description::new_base_type("boolean"));
    map.insert("object_type", Description::new_string("object".to_string()));
    map.insert("object_values", Description::new_base_type("array"));
    map.insert("object_keys", Description::new_base_type("array"));
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

    pub fn only_positive(&self) -> Option<Description> {
        match self {
            Description::Null => None,
            Description::StringValue { value } => {
                if value.is_empty() {
                    None
                } else {
                    Some(Description::new_base_type("string"))
                }
            }
            Description::NumberValue { value } => {
                if *value == 0.0 {
                    None
                } else {
                    Some(Description::new_base_type("number"))
                }
            }
            Description::BooleanValue { value } => {
                if *value {
                    Some(Description::new_base_type("boolean"))
                } else {
                    None
                }
            }
            Description::BaseType { field_type } => {
                if field_type == "null" {
                    return None;
                }
                Some(Description::new_base_type(field_type))
            }
            Description::Union { of } => {
                let mut result = Vec::new();
                for item in of {
                    if let Some(item) = item.only_positive() {
                        result.push(item);
                    }
                }
                Some(Description::Union { of: result })
            }
            desc => Some(desc.clone()),
        }
    }

    pub fn to_notation(&self) -> String {
        self.to_notation_with_indent(0)
    }

    pub fn to_notation_with_indent(&self, indent: usize) -> String {
        match self {
            Description::Null => "null".to_owned(),
            Description::StringValue { value } => "\"".to_owned() + value + "\"",
            Description::NumberValue { value } => value.to_string(),
            Description::BooleanValue { value } => value.to_string(),
            Description::Object { value } => {
                if value.is_empty() {
                    return "{}".to_owned();
                }

                let indent_str = " ".repeat(indent + 2);

                // Get entries and sort them
                let mut entries = value.iter().collect::<Vec<_>>();
                entries.sort_by(|a, b| a.0.cmp(b.0));

                // Write notation
                let mut notation: String = "{\n".to_owned();
                for (k, v) in entries {
                    // If key is alphanumeric, we don't need to quote it
                    let k = if k.chars().all(|c| c.is_alphanumeric()) {
                        k.clone()
                    } else {
                        format!("\"{}\"", k)
                    };

                    notation.push_str(&format!(
                        "{}{}: {},\n",
                        indent_str,
                        k,
                        v.to_notation_with_indent(indent + 2)
                    ));
                }

                // Remove trailing comma
                if notation.ends_with(",\n") {
                    notation.pop();
                    notation.pop();
                }
                notation.push('\n');
                notation.push_str(" ".repeat(indent).as_str());
                notation.push('}');
                notation
            }
            Description::ExactArray { value } => {
                if value.is_empty() {
                    return "[]".to_owned();
                }

                let mut descriptions = Vec::new();
                for item in value {
                    descriptions.push(item.to_notation());
                }
                format!("[{}]", descriptions.join(", "))
            }
            Description::Array {
                item_type,
                length: _,
            } => {
                // If item type is a union we need to wrap it in parentheses
                let item_type = match *item_type.clone() {
                    Description::Union { of } => {
                        if of.len() == 1 {
                            of[0].to_notation()
                        } else {
                            format!("({})", item_type.to_notation())
                        }
                    }
                    _ => item_type.to_notation(),
                };
                format!("{}[]", item_type)
            }
            Description::BaseType { field_type } => field_type.to_owned(),
            Description::Union { of } => {
                let mut descriptions = Vec::new();
                for item in of {
                    descriptions.push(item.to_notation());
                }
                descriptions.join(" | ")
            }
            Description::Error { error: _ } => "error".to_owned(), // TODO
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
        description.unwrap_or(Description::Any)
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
            Description::Array {
                item_type: _,
                length: _,
            } => Description::Error {
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
                _ => Description::Any,
            },
            Description::Union { of } => {
                let mut descriptions = Vec::new();
                for item in of {
                    descriptions.push(item.to_string());
                }
                Description::new_union(descriptions)
            }
            Description::Any => Description::Any,
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
            Description::Object { .. } => Description::BooleanValue { value: true },
            Description::ExactArray { .. } => Description::BooleanValue { value: true },
            Description::Array { .. } => Description::BooleanValue { value: true },
            Description::BaseType { field_type } => {
                if field_type.starts_with("string") {
                    return Description::new_base_type("boolean");
                }

                if field_type.starts_with("number") {
                    return Description::new_base_type("boolean");
                }

                if field_type.starts_with("object") {
                    return Description::BooleanValue { value: true };
                }

                if field_type.starts_with("array") {
                    return Description::BooleanValue { value: true };
                }

                if field_type.starts_with("boolean") {
                    return Description::new_base_type("boolean");
                }
                Description::Any
            }
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
                // Check if it's a UUID
                if value.len() == 36
                    && value.chars().nth(8) == Some('-')
                    && value.chars().nth(13) == Some('-')
                    && value.chars().nth(18) == Some('-')
                    && value.chars().nth(23) == Some('-')
                {
                    return Description::new_base_type("string.uuid");
                }

                // Check if it's a date
                if value.len() == 10
                    && value.chars().nth(4) == Some('-')
                    && value.chars().nth(7) == Some('-')
                {
                    return Description::new_base_type("string.date");
                }

                // Check if it's a time
                if value.len() == 8
                    && value.chars().nth(2) == Some(':')
                    && value.chars().nth(5) == Some(':')
                {
                    return Description::new_base_type("string.time");
                }

                // Check if it's a datetime
                if value.len() == 19
                    && value.chars().nth(4) == Some('-')
                    && value.chars().nth(7) == Some('-')
                    && value.chars().nth(10) == Some('T')
                    && value.chars().nth(13) == Some(':')
                    && value.chars().nth(16) == Some(':')
                {
                    return Description::new_base_type("string.datetime");
                }

                // Check if it's a duration
                if value.len() > 2
                    && value.ends_with('s')
                    && value.chars().nth(value.len() - 2) == Some('m')
                {
                    return Description::new_base_type("string.duration");
                }

                // Check if it's a number
                if value.parse::<f64>().is_ok() {
                    return Description::new_base_type("string.number");
                }

                // Check if it's a boolean
                if value == "true" || value == "false" {
                    return Description::new_base_type("string.boolean");
                }

                if value.chars().all(|c| c.is_alphanumeric()) {
                    return Description::new_base_type("string.alphanumeric");
                }

                if value.parse::<Url>().is_ok() {
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
                // let mut descriptions = Vec::new();
                // // TODO: Limit and pick at certain levels
                // for item in value {
                //     descriptions.push(item.to_base());
                // }

                let unified = value
                    .clone()
                    .into_iter()
                    .reduce(|a, b| merge(a.to_base(), b.to_base()));

                if let Some(item_type) = unified {
                    Description::Array {
                        length: Some(value.len()),
                        item_type: Box::new(item_type),
                    }
                } else {
                    Description::new_base_type("array.empty")
                }
            }
            Description::NumberValue { value } => {
                if value.fract() == 0.0 {
                    Description::new_base_type("number.integer")
                } else {
                    Description::new_base_type("number")
                }
            }
            d => d.clone(),
        }
    }
}

/// Describes a storage value
///
/// This function will return a description of the value.
pub fn describe(value: StorageValue) -> Description {
    // TODO: Shall we take a reference instead?
    match value {
        StorageValue::String(s) => {
            if s.len() < 5000 {
                Description::StringValue {
                    value: s.to_string(),
                }
            } else {
                Description::StringValue {
                    value: s[..5000].to_string(),
                }
            }
        }
        StorageValue::Number(f) => Description::NumberValue { value: f },
        StorageValue::Boolean(b) => Description::BooleanValue { value: b },
        StorageValue::Array(array) => {
            let length = array.len();

            // TODO: Make this limit smarter
            if length > 30 {
                let mut item_description = Description::Union { of: vec![] };
                // Gather descriptions which for many items will be much smaller
                for item in array {
                    item_description = merge(item_description, describe(item));
                }

                // Further compression
                // We will get rid of individual values in favor of base types
                item_description = match item_description {
                    Description::Union { of } => {
                        if of.len() > 12 {
                            let mut item_description = Description::Union { of: vec![] };
                            for item in of {
                                item_description = merge(item_description, item.to_base());
                            }
                            item_description
                        } else {
                            Description::Union { of }
                        }
                    }
                    b => b,
                };

                Description::Array {
                    length: Some(length),
                    item_type: Box::new(item_description),
                }
            } else {
                let mut descriptions = Vec::new();
                for item in array {
                    descriptions.push(describe(item));
                }
                Description::ExactArray {
                    value: descriptions,
                }
            }
        }
        StorageValue::Object(object) => {
            let mut descriptions: HashMap<String, Description> = HashMap::new();
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

fn add_to_union(mut descriptions: Vec<Description>, description: Description) -> Description {
    for item in descriptions.iter_mut() {
        if item == &description {
            return Description::Union { of: descriptions };
        }
    }
    descriptions.push(description);
    Description::Union { of: descriptions }
}

fn add_to_union_mut(descriptions: &mut Vec<Description>, description: Description) {
    for item in descriptions.iter_mut() {
        if item == &description {
            return;
        }
    }
    descriptions.push(description);
}

pub fn merge(a: Description, b: Description) -> Description {
    match (a, b) {
        (Description::Union { of: a }, Description::Union { of: mut b }) => {
            for item_a in a.into_iter() {
                add_to_union_mut(&mut b, item_a);
            }
            Description::Union { of: b }
        }
        (Description::Union { of: a }, b) => add_to_union(a, b),
        (a, Description::Union { of: b }) => add_to_union(b, a),
        (a, b) => {
            if a == b {
                return a;
            }
            Description::Union { of: vec![a, b] }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DescriptionParseError;

pub fn parse_value_by_description(
    value: StorageValue,
    description: Description,
) -> Result<StorageValue, DescriptionParseError> {
    match description {
        Description::Null => match value {
            StorageValue::Null(_) => Ok(value),
            _ => Err(DescriptionParseError),
        },
        Description::StringValue { value: expected } => match value {
            StorageValue::String(s) => {
                if s == expected {
                    Ok(s.into())
                } else {
                    Err(DescriptionParseError)
                }
            }
            _ => Err(DescriptionParseError),
        },
        Description::NumberValue { value: expected } => match value {
            StorageValue::Number(n) => {
                if n == expected {
                    Ok(value)
                } else {
                    Err(DescriptionParseError)
                }
            }
            _ => Err(DescriptionParseError),
        },
        Description::BooleanValue { value: expected } => match value {
            StorageValue::Boolean(b) => {
                if b == expected {
                    Ok(value)
                } else {
                    Err(DescriptionParseError)
                }
            }
            _ => Err(DescriptionParseError),
        },
        Description::Array { item_type, length } => match value {
            StorageValue::Array(array) => {
                if let Some(length) = length {
                    if array.len() != length {
                        return Err(DescriptionParseError);
                    }
                }

                let mut result = Vec::new();
                for item in array {
                    result.push(parse_value_by_description(item, *item_type.clone())?);
                }
                Ok(StorageValue::Array(result))
            }
            _ => Err(DescriptionParseError),
        },
        Description::Object { value: expected } => match value {
            StorageValue::Object(object) => {
                let mut result = HashMap::new();
                for (key, expected) in expected {
                    result.insert(
                        key.clone(),
                        parse_value_by_description(
                            object.get(&key).unwrap_or(&NULL).clone(),
                            expected.clone(),
                        )?,
                    );
                }
                Ok(StorageValue::Object(result))
            }
            _ => Err(DescriptionParseError),
        },
        Description::ExactArray { value: expected } => match value {
            StorageValue::Array(array) => {
                if array.len() != expected.len() {
                    return Err(DescriptionParseError);
                }

                let mut result = Vec::new();
                for (index, expected) in expected.iter().enumerate() {
                    result.push(parse_value_by_description(
                        array[index].clone(),
                        expected.clone(),
                    )?);
                }
                Ok(StorageValue::Array(result))
            }
            _ => Err(DescriptionParseError),
        },
        Description::Union { of } => {
            let mut result = Vec::new();
            for variant in of {
                match parse_value_by_description(value.clone(), variant) {
                    Ok(variant) => {
                        result.push(variant);
                    }
                    Err(_e) => {
                        // TODO: Collect errors for better error message
                    }
                }
            }

            if let Some(result) = result.pop() {
                Ok(result)
            } else {
                Err(DescriptionParseError)
            }
        }
        Description::Error { .. } => unreachable!(),
        Description::Any => Ok(value),
        Description::BaseType { field_type } => {
            if field_type.starts_with("string") {
                match value {
                    StorageValue::String(_) => return Ok(value),
                    _ => return Err(DescriptionParseError),
                }
            }

            if field_type.starts_with("number") {
                match value {
                    StorageValue::Number(_) => return Ok(value),
                    _ => return Err(DescriptionParseError),
                }
            }

            if field_type.starts_with("object") {
                match value {
                    StorageValue::Object(_) => return Ok(value),
                    _ => return Err(DescriptionParseError),
                }
            }

            if field_type.starts_with("array") {
                match value {
                    StorageValue::Array(_) => return Ok(value),
                    _ => return Err(DescriptionParseError),
                }
            }

            if field_type.starts_with("boolean") {
                match value {
                    StorageValue::Boolean(_) => return Ok(value),
                    _ => return Err(DescriptionParseError),
                }
            }

            Err(DescriptionParseError)
        }
    }
}

pub fn predict_description<S: DescribedStorage, E: DescribedEnvironment>(
    expr: Spanned<Expr>,
    storage: &S,
    environment: &E,
) -> Description {
    match expr.0 {
        Expr::Null => Description::Null,
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            let value = predict_description(*condition, storage, environment);

            match value {
                Description::BooleanValue { value } => {
                    if value {
                        predict_description(*then, storage, environment)
                    } else {
                        predict_description(*otherwise, storage, environment)
                    }
                }
                Description::Any => Description::Union {
                    of: vec![
                        predict_description(*then, storage, environment),
                        predict_description(*otherwise, storage, environment),
                    ],
                },
                Description::BaseType { field_type } if field_type == "boolean" => {
                    Description::Union {
                        of: vec![
                            predict_description(*then, storage, environment),
                            predict_description(*otherwise, storage, environment),
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
                .map(|e| predict_description(e, storage, environment))
                .collect();

            Description::ExactArray { value: data }
        }
        Expr::Object(n) => {
            let data: HashMap<String, Description> = n
                .into_iter()
                .map(|(k, v)| {
                    let v = predict_description(v, storage, environment);
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
            let expr = predict_description(*expr, storage, environment);

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
                Description::Array { item_type, length } => match attr.parse::<usize>() {
                    Ok(i) => {
                        if let Some(length) = length {
                            if i >= *length {
                                Description::Null
                            } else {
                                *item_type.clone()
                            }
                        } else {
                            Description::new_union(vec![*item_type.clone(), Description::Null])
                        }
                    }
                    Err(_) => Description::Null,
                },
                Description::Any => Description::Any,
                _ => Description::Null,
            })
        }
        Expr::Slice(expr, slice_expr) => {
            let expr = predict_description(*expr, storage, environment);
            let slice = predict_description(*slice_expr, storage, environment);

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
                            None => Description::Any,
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
                        _ => Description::Any,
                    },
                    Description::ExactArray { value } => match slice.as_index() {
                        Ok(i) => value.get(i).cloned().unwrap_or(Description::Null),
                        Err(error) => match error {
                            Some(e) => Description::new_error(e),
                            None => Description::Any,
                        },
                    },
                    Description::Array {
                        item_type,
                        length: _,
                    } => item_type.optional(),
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
            let expr = predict_description(*expr, storage, environment);

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
            let l = predict_description(*lhs, storage, environment);
            let r = predict_description(*rhs, storage, environment);

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
            let value = predict_description(*callee, storage, environment);

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

pub fn evaluate_selector_description<S: DescribedStorage, E: DescribedEnvironment>(
    expr: Spanned<Expr>,
    storage: &S,
    environment: &E,
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
            let value = predict_description(*slice_expr, storage, environment);

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
            let value = predict_description(*condition, storage, environment);

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
    Any,
}

pub fn store_description(
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
                    ContextStorage::Any => {
                        return Ok(());
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
                    ContextStorage::Any => return Ok(()),
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
                    ContextStorage::Any => return Ok(()),
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
                            Description::Any => {
                                // TODO: Emit warning
                                traversed = ContextStorage::Any;
                            }
                            Description::Array {
                                item_type,
                                length: _,
                            } => {
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
                    ContextStorage::Any => {
                        traversed = ContextStorage::Any;
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
                            Description::Any => {
                                // TODO: Emit warning
                                traversed = ContextStorage::Any;
                            }
                            Description::Array {
                                item_type,
                                length: _,
                            } => {
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
                    ContextStorage::Any => {
                        traversed = ContextStorage::Any;
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
                                Description::Array {
                                    item_type,
                                    length: _,
                                } => {
                                    traversed = ContextStorage::SimpleArray(item_type);
                                }
                                Description::Any => {
                                    // TODO: Emit warning
                                    traversed = ContextStorage::Any;
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
                                Description::Array { item_type, length } => {
                                    if let Some(length) = length {
                                        if index >= *length {
                                            return Err(TelError::IndexOutOfBounds {
                                                index,
                                                max: *length - 1,
                                            });
                                        }
                                        traversed = ContextStorage::SimpleArray(item_type);
                                    } else {
                                        traversed = ContextStorage::SimpleArray(item_type);
                                    }
                                }
                                Description::Any => {
                                    // TODO: Emit warning
                                    traversed = ContextStorage::Any;
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
                        let index = slice.as_index()?; // TODO: Check if index could be used for better data

                        match item_type.as_mut() {
                            Description::Object { value } => {
                                traversed = ContextStorage::Object(value);
                            }
                            Description::ExactArray { value } => {
                                traversed = ContextStorage::Array(value);
                            }
                            Description::Array { item_type, length } => {
                                if let Some(length) = length {
                                    if index >= *length {
                                        return Err(TelError::IndexOutOfBounds {
                                            index,
                                            max: *length - 1,
                                        });
                                    }
                                    traversed = ContextStorage::SimpleArray(item_type);
                                } else {
                                    traversed = ContextStorage::SimpleArray(item_type);
                                }
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
                    ContextStorage::Any => {
                        traversed = ContextStorage::Any;
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

    use crate::{evaluate_description_notation, parse, parse_description, storage_value};

    use super::*;

    fn parse_and_evaluate_description_notation(input: &str) -> Result<Description, TelError> {
        let result = parse_description(input);
        if let Some(expr) = result.expr {
            evaluate_description_notation(expr)
        } else {
            Err(TelError::ParseError {
                errors: result.errors,
            })
        }
    }

    #[test]
    fn test_parse_value_by_description_simple_string() {
        let description = describe(StorageValue::String("Hello World".to_owned()));
        let value =
            parse_value_by_description(StorageValue::String("Hello World".to_owned()), description)
                .unwrap();
        assert_eq!(value, StorageValue::String("Hello World".to_owned()));
    }

    #[test]
    fn test_parse_value_by_description_object() {
        let object = StorageValue::Object({
            let mut map = HashMap::new();
            map.insert(
                "a".to_owned(),
                StorageValue::String("Hello World".to_owned()),
            );
            map
        });

        let value = parse_value_by_description(
            object.clone(),
            parse_and_evaluate_description_notation("{ a: string | null }").unwrap(),
        )
        .unwrap();
        assert_eq!(value, object);
    }

    #[test]
    fn test_parse_value_by_description_object_with_optional_property() {
        let object = StorageValue::Object(HashMap::new());

        let value = parse_value_by_description(
            object.clone(),
            parse_and_evaluate_description_notation("{ a: string | null }").unwrap(),
        )
        .unwrap();
        assert_eq!(
            value,
            StorageValue::Object({
                let mut map = HashMap::new();
                map.insert("a".to_owned(), NULL);
                map
            })
        );
    }

    #[test]
    fn test_parse_value_by_description_simple_union() {
        let a: StorageValue = 500.into();
        let b: StorageValue = "Hello World".into();

        let description = parse_and_evaluate_description_notation("number | string").unwrap();

        let value = parse_value_by_description(a.clone(), description.clone()).unwrap();
        assert_eq!(value, a);
        let value = parse_value_by_description(b.clone(), description.clone()).unwrap();
        assert_eq!(value, b);
    }

    #[test]
    fn test_parse_value_by_description_union_of_objects() {
        let a = StorageValue::Object({
            let mut map = HashMap::new();
            map.insert("type".to_owned(), "A".into());
            map.insert("id".to_owned(), "Hello".into());
            map
        });

        let b = StorageValue::Object({
            let mut map = HashMap::new();
            map.insert("type".to_owned(), "B".into());
            map.insert("id".to_owned(), 500.into());
            map
        });

        let description = parse_and_evaluate_description_notation(
            r#"{ type: "A", id: string } | { type: "B", id: number }"#,
        )
        .unwrap();

        let value = parse_value_by_description(a.clone(), description.clone()).unwrap();
        assert_eq!(value, a);

        let value = parse_value_by_description(b.clone(), description.clone()).unwrap();
        assert_eq!(value, b);
    }

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
                    field_type: "string.alphanumeric".to_owned()
                }),
                length: Some(3)
            }
        )
    }

    #[test]
    fn test_detects_url() {
        let description = describe("https://turbofuro.com".into()).to_base();

        assert_eq!(
            description,
            Description::BaseType {
                field_type: "string.url".to_owned()
            }
        )
    }

    #[test]
    fn test_detects_uuid() {
        let description = describe("e65ebda2-aa4a-4eab-8f6e-de1fe87d98ba".into()).to_base();

        assert_eq!(
            description,
            Description::BaseType {
                field_type: "string.uuid".to_owned()
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

    fn evaluate_and_store(
        selector_expression: &str,
        value: Description,
        storage: &mut ObjectDescription,
        environment: &HashMap<String, Description>,
    ) {
        let result = parse(selector_expression);

        if let Some(expr) = result.expr {
            let mut selector = evaluate_selector_description(expr, storage, environment);
            assert_eq!(selector.len(), 1);

            let selector = selector.remove(0);
            match selector {
                SelectorDescription::Static { selector } => {
                    store_description(&selector, storage, value).unwrap();
                }
                _ => {
                    panic!("Should not happen");
                }
            }
        }
    }

    #[test]
    fn test_store_description() {
        let mut storage = HashMap::new();
        let environment = HashMap::new();

        evaluate_and_store(
            "test",
            describe(storage_value!({ "a": 4 })),
            &mut storage,
            &environment,
        );

        let current_test = storage.get("test").unwrap().clone();
        assert_eq!(current_test, describe(storage_value!({ "a": 4 })))
    }

    #[test]
    fn test_store_description2() {
        let mut storage = HashMap::new();
        let environment = HashMap::new();

        evaluate_and_store(
            "test",
            describe(storage_value!({ "a": 4 })),
            &mut storage,
            &environment,
        );

        evaluate_and_store(
            "test.a",
            describe(StorageValue::Number(6.0)),
            &mut storage,
            &environment,
        );

        let current_test = storage.get("test").unwrap().clone();
        assert_eq!(current_test, describe(storage_value!({ "a": 6.0 })))
    }

    #[test]
    fn test_notation() {
        assert_eq!(Description::new_base_type("string").to_notation(), "string");
        assert_eq!(
            Description::new_string("GET".into()).to_notation(),
            "\"GET\""
        );
        assert_eq!(
            Description::new_union(vec![
                Description::new_base_type("string"),
                Description::new_base_type("number")
            ])
            .to_notation(),
            "string | number"
        );

        let mut object = HashMap::new();
        object.insert("a".to_owned(), Description::new_base_type("string"));
        object.insert("b".to_owned(), Description::new_base_type("number"));
        assert_eq!(
            Description::Object {
                value: object.clone()
            }
            .to_notation(),
            "{\n  a: string,\n  b: number\n}"
        );

        let mut bigger_object = HashMap::new();
        bigger_object.insert("a".to_owned(), Description::new_base_type("string"));
        bigger_object.insert("b".to_owned(), Description::new_base_type("number"));
        bigger_object.insert("c".to_owned(), Description::Object { value: object });
        assert_eq!(
            Description::Object {
                value: bigger_object.clone()
            }
            .to_notation(),
            "{\n  a: string,\n  b: number,\n  c: {\n    a: string,\n    b: number\n  }\n}"
        );

        assert_eq!(
            Description::Object {
                value: HashMap::new()
            }
            .to_notation(),
            "{}"
        );
        assert_eq!(
            Description::ExactArray { value: vec![] }.to_notation(),
            "[]"
        );
        assert_eq!(
            Description::ExactArray {
                value: vec![
                    Description::new_base_type("string"),
                    Description::new_base_type("number"),
                ]
            }
            .to_notation(),
            "[string, number]"
        );
        assert_eq!(
            Description::Array {
                item_type: Box::new(Description::new_base_type("string")),
                length: None
            }
            .to_notation(),
            "string[]"
        );

        assert_eq!(
            Description::Array {
                item_type: Box::new(Description::new_union(vec![
                    Description::new_base_type("string"),
                    Description::new_base_type("number")
                ])),
                length: None
            }
            .to_notation(),
            "(string | number)[]"
        );
    }
}
