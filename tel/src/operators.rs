use crate::{StorageValue, TelError};

pub(crate) fn add(a: StorageValue, b: StorageValue) -> Result<StorageValue, TelError> {
    match a {
        StorageValue::Number(a) => match b {
            StorageValue::Number(b) => Ok(StorageValue::Number(a + b)),
            StorageValue::String(b) => Ok(StorageValue::String(a.to_string() + &b)),
            b => Err(TelError::UnsupportedOperation {
                message: format!("Can't sum (+ operator) float and {}", b.get_type()),
                operation: format!("sum_float_{}", b.get_type()),
            }),
        },
        StorageValue::String(a) => match b {
            StorageValue::Number(b) => Ok(StorageValue::String(a + &b.to_string())),
            StorageValue::String(b) => Ok(StorageValue::String(a + &b)),
            StorageValue::Boolean(b) => Ok(StorageValue::String(a + &b.to_string())),
            StorageValue::Null(_) => Ok(StorageValue::String(a + "null")),
            b => Err(TelError::UnsupportedOperation {
                message: format!("Can't sum (+ operator) string and {}", b.get_type()),
                operation: format!("sum_string_{}", b.get_type()),
            }),
        },
        StorageValue::Array(a) => match b {
            StorageValue::Array(b) => {
                let mut result = a;
                result.extend(b);
                Ok(StorageValue::Array(result))
            }
            b => Err(TelError::UnsupportedOperation {
                message: format!("Can't sum (+ operator) array and {}", b.get_type()),
                operation: format!("sum_array_{}", b.get_type()),
            }),
        },
        StorageValue::Object(a) => match b {
            StorageValue::Object(b) => {
                let mut result = a;
                for (key, value) in b {
                    result.insert(key, value);
                }
                Ok(StorageValue::Object(result))
            }
            b => Err(TelError::UnsupportedOperation {
                message: format!("Can't sum (+ operator) object and {}", b.get_type()),
                operation: format!("sum_object_{}", b.get_type()),
            }),
        },
        a => Err(TelError::UnsupportedOperation {
            message: format!(
                "Can't sum (+ operator) {} and {}",
                a.get_type(),
                b.get_type()
            ),
            operation: format!("sum_{}_{}", a.get_type(), b.get_type()),
        }),
    }
}

pub(crate) fn sub(a: StorageValue, b: StorageValue) -> Result<StorageValue, TelError> {
    match a {
        StorageValue::Number(a) => match b {
            StorageValue::Number(b) => Ok(StorageValue::Number(a - b)),
            b => Err(TelError::UnsupportedOperation {
                message: format!("Can't subtract (- operator) float and {}", b.get_type()),
                operation: format!("subtract_float_{}", b.get_type()),
            }),
        },
        a => Err(TelError::UnsupportedOperation {
            message: format!(
                "Can't subtract (- operator) {} and {}",
                a.get_type(),
                b.get_type()
            ),
            operation: format!("subtract_{}_{}", a.get_type(), b.get_type()),
        }),
    }
}

pub(crate) fn mul(a: StorageValue, b: StorageValue) -> Result<StorageValue, TelError> {
    match a {
        StorageValue::Number(a) => match b {
            StorageValue::Number(b) => Ok(StorageValue::Number(a * b)),
            b => Err(TelError::UnsupportedOperation {
                message: format!("Can't multiply (* operator) float by {}", b.get_type()),
                operation: format!("multiply_float_{}", b.get_type()),
            }),
        },
        a => Err(TelError::UnsupportedOperation {
            message: format!(
                "Can't multiply (* operator) {} by {}",
                a.get_type(),
                b.get_type()
            ),
            operation: format!("multiply_{}_{}", a.get_type(), b.get_type()),
        }),
    }
}

pub(crate) fn div(a: StorageValue, b: StorageValue) -> Result<StorageValue, TelError> {
    match a {
        StorageValue::Number(a) => match b {
            StorageValue::Number(b) => Ok(StorageValue::Number(a / b)),
            b => Err(TelError::UnsupportedOperation {
                message: format!("Can't divide (/ operator) float by {}", b.get_type()),
                operation: format!("divide_float_{}", b.get_type()),
            }),
        },
        a => Err(TelError::UnsupportedOperation {
            message: format!(
                "Can't divide (/ operator) {} by {}",
                a.get_type(),
                b.get_type()
            ),
            operation: format!("divide_{}_{}", a.get_type(), b.get_type()),
        }),
    }
}

pub(crate) fn rem(a: StorageValue, b: StorageValue) -> Result<StorageValue, TelError> {
    match a {
        StorageValue::Number(a) => match b {
            StorageValue::Number(b) => Ok(StorageValue::Number(a % b)),
            b => Err(TelError::UnsupportedOperation {
                message: format!(
                    "Can't get remainder (% operator) of float divided by {}",
                    b.get_type()
                ),
                operation: format!("remainder_float_{}", b.get_type()),
            }),
        },
        a => Err(TelError::UnsupportedOperation {
            message: format!(
                "Can't get remainder (% operator) of {} divided by {}",
                a.get_type(),
                b.get_type()
            ),
            operation: format!("remainder_{}_{}", a.get_type(), b.get_type()),
        }),
    }
}

pub(crate) fn neg(first: StorageValue) -> Result<StorageValue, TelError> {
    match first {
        StorageValue::Boolean(a) => Ok(StorageValue::Boolean(!a)),
        a => Err(TelError::UnsupportedOperation {
            message: format!("Can't negate (! operator) {}", a.get_type()),
            operation: format!("negate_{}", a.get_type()),
        }),
    }
}

pub(crate) fn plus(first: StorageValue) -> Result<StorageValue, TelError> {
    match first {
        StorageValue::Number(a) => Ok(StorageValue::Number(a)),
        a => Err(TelError::UnsupportedOperation {
            message: format!("Can't use unary plus (+ operator) on {}", a.get_type()),
            operation: format!("plus_{}", a.get_type()),
        }),
    }
}

pub(crate) fn minus(first: StorageValue) -> Result<StorageValue, TelError> {
    match first {
        StorageValue::Number(a) => Ok(StorageValue::Number(-a)),
        a => Err(TelError::UnsupportedOperation {
            message: format!("Can't use unary minus (- operator) on {}", a.get_type()),
            operation: format!("minus_{}", a.get_type()),
        }),
    }
}
