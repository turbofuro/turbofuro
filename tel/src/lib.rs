use chumsky::{prelude::*, Parser, Stream};
use serde::Serializer;
use serde_derive::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt::Display;
use std::{collections::HashMap, vec};

mod description;
mod description_notation;
mod operators;

pub use description::describe;
pub use description::evaluate_selector_description;
pub use description::parse_value_by_description;
pub use description::predict_description;
pub use description::store_description;
pub use description::Description;
pub use description::ObjectDescription;
pub use description::SelectorDescription;
pub use description_notation::evaluate_description_notation;
pub use description_notation::parse_description;

pub type ObjectBody = HashMap<String, StorageValue>;

/// Layered storage for evaluating expressions with multiple storages
/// without having to duplicate the storages
#[derive(Debug)]
pub struct LayeredStorage<'a, 'b, T: Storage> {
    pub top: &'a T,
    pub down: &'b T,
}

impl Storage for LayeredStorage<'_, '_, ObjectBody> {
    fn get(&self, key: &str) -> Option<&StorageValue> {
        self.top.get(key).or_else(|| self.down.get(key))
    }
}

pub trait Storage {
    fn get(&self, key: &str) -> Option<&StorageValue>;
}

impl Storage for ObjectBody {
    fn get(&self, key: &str) -> Option<&StorageValue> {
        self.get(key)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StorageValue {
    String(String),
    #[serde(serialize_with = "serialize_number")]
    Number(f64),
    Boolean(bool),
    Array(Vec<StorageValue>),
    Object(ObjectBody),
    Null(Option<()>),
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

impl Default for StorageValue {
    fn default() -> Self {
        NULL
    }
}

impl From<String> for StorageValue {
    fn from(item: String) -> Self {
        StorageValue::String(item)
    }
}

impl From<f32> for StorageValue {
    fn from(item: f32) -> Self {
        StorageValue::Number(item.into())
    }
}

impl From<i32> for StorageValue {
    fn from(item: i32) -> Self {
        StorageValue::Number(item.into())
    }
}

impl From<u32> for StorageValue {
    fn from(item: u32) -> Self {
        StorageValue::Number(item.into())
    }
}

impl From<usize> for StorageValue {
    fn from(item: usize) -> Self {
        StorageValue::Number(item as f64)
    }
}

impl From<f64> for StorageValue {
    fn from(item: f64) -> Self {
        StorageValue::Number(item)
    }
}

impl From<bool> for StorageValue {
    fn from(item: bool) -> Self {
        StorageValue::Boolean(item)
    }
}

impl From<&str> for StorageValue {
    fn from(item: &str) -> Self {
        StorageValue::String(item.to_owned())
    }
}

#[macro_export]
macro_rules! storage_value {
    ($($json:tt)+) => {
        serde_json::from_value::<StorageValue>(serde_json::json!($($json)+)).unwrap()
    };
}

#[macro_export]
macro_rules! expr {
    ($($tts:tt)*) => {
        parse(stringify!($($tts)*))
    };
}

impl StorageValue {
    pub fn get_type(&self) -> &'static str {
        match self {
            StorageValue::String(_) => "string",
            StorageValue::Number(_) => "number",
            StorageValue::Boolean(_) => "boolean",
            StorageValue::Array(_) => "array",
            StorageValue::Object(_) => "object",
            StorageValue::Null(_) => "null",
        }
    }

    pub fn new_byte_array(bytes: &[u8]) -> StorageValue {
        StorageValue::Array(
            bytes
                .iter()
                .map(|b| StorageValue::Number(*b as f64))
                .collect(),
        )
    }

    pub fn as_index(&self) -> Result<usize, TelError> {
        match self {
            StorageValue::Number(f) => Ok(*f as usize),
            StorageValue::String(s) => match s.parse::<usize>() {
                Ok(i) => Ok(i),
                Err(_) => Err(TelError::InvalidIndex {
                    subject: "string".to_owned(),
                    message: "Can't use string as array index".to_owned(),
                }),
            },
            _ => Err(TelError::InvalidIndex {
                subject: self.get_type().to_owned(),
                message: format!("Can't use {} as array index", self.get_type()),
            }),
        }
    }

    pub fn to_string(&self) -> Result<String, TelError> {
        match self {
            StorageValue::String(s) => Ok(s.clone()),
            StorageValue::Number(f) => Ok(f.to_string()),
            StorageValue::Boolean(b) => Ok(b.to_string()),
            StorageValue::Array(_) => Err(TelError::UnsupportedOperation {
                operation: "to_string_array".to_owned(),
                message: "Can't convert array to string".to_owned(),
            }),
            StorageValue::Object(_) => Err(TelError::UnsupportedOperation {
                operation: "to_string_object".to_owned(),
                message: "Can't convert object to string".to_owned(),
            }),
            StorageValue::Null(_) => Ok("null".to_owned()),
        }
    }

    pub fn to_boolean(&self) -> Result<bool, TelError> {
        match self {
            StorageValue::String(s) => Ok(!s.is_empty()),
            StorageValue::Number(f) => Ok(*f != 0.0),
            StorageValue::Boolean(b) => Ok(*b),
            StorageValue::Array(_) => Ok(true),
            StorageValue::Object(_) => Ok(true),
            StorageValue::Null(_) => Ok(false),
        }
    }
}

impl PartialOrd for StorageValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self {
            StorageValue::String(a) => match other {
                StorageValue::String(b) => a.partial_cmp(b),
                _ => None,
            },
            StorageValue::Number(a) => match other {
                StorageValue::Number(b) => a.partial_cmp(b),
                _ => None,
            },
            StorageValue::Boolean(a) => match other {
                StorageValue::Boolean(b) => a.partial_cmp(b),
                _ => None,
            },
            StorageValue::Array(_) => None,
            StorageValue::Object(_) => None,
            StorageValue::Null(b) => match other {
                StorageValue::Null(_) => b.partial_cmp(&None),
                _ => None,
            },
        }
    }
}

impl PartialEq for StorageValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::String(l0), Self::String(r0)) => l0 == r0,
            (Self::Number(l0), Self::Number(r0)) => l0 == r0,
            (Self::Boolean(l0), Self::Boolean(r0)) => l0 == r0,
            (Self::Array(l0), Self::Array(r0)) => l0 == r0,
            (Self::Object(l0), Self::Object(r0)) => l0 == r0,
            (Self::Null(_), Self::Null(_)) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Expr {
    Null,
    If {
        condition: Box<Spanned<Self>>,
        then: Box<Spanned<Self>>,
        otherwise: Box<Spanned<Self>>,
    },
    Number(f64),
    String(String),
    MultilineString {
        value: String,
        tag: String,
    },
    Boolean(bool),
    Array(Vec<Spanned<Self>>),
    Object(HashMap<String, Spanned<Expr>>),
    Identifier(String),
    Environment(String),
    Attribute(Box<Spanned<Self>>, String),
    Slice(Box<Spanned<Self>>, Box<Spanned<Self>>),
    UnaryOp(UnaryOpType, Box<Spanned<Self>>),
    MethodCall {
        callee: Box<Spanned<Self>>,
        name: String,
        arguments: Vec<Spanned<Self>>,
    },
    BinaryOp {
        lhs: Box<Spanned<Self>>,
        op: BinaryOpType,
        rhs: Box<Spanned<Self>>,
    },
    Invalid,
}

pub type Span = std::ops::Range<usize>;

pub type Spanned<T> = (T, Span);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum UnaryOpType {
    Plus,
    Minus,
    Negation,
}

impl UnaryOpType {
    pub fn get_operator(&self) -> &str {
        match self {
            UnaryOpType::Plus => "+",
            UnaryOpType::Minus => "-",
            UnaryOpType::Negation => "!",
        }
    }

    pub fn get_label(&self) -> &str {
        match self {
            UnaryOpType::Plus => "unary plus",
            UnaryOpType::Minus => "unary minus",
            UnaryOpType::Negation => "negation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum BinaryOpType {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
    And,
    Or,
}

impl BinaryOpType {
    pub fn get_operator(&self) -> &str {
        match self {
            BinaryOpType::Add => "+",
            BinaryOpType::Subtract => "-",
            BinaryOpType::Multiply => "*",
            BinaryOpType::Divide => "/",
            BinaryOpType::Modulo => "%",
            BinaryOpType::Gt => ">",
            BinaryOpType::Gte => ">=",
            BinaryOpType::Lt => "<",
            BinaryOpType::Lte => "<=",
            BinaryOpType::Eq => "==",
            BinaryOpType::Neq => "!=",
            BinaryOpType::And => "&&",
            BinaryOpType::Or => "||",
        }
    }

    pub fn get_label(&self) -> &str {
        match self {
            BinaryOpType::Add => "add",
            BinaryOpType::Subtract => "sub",
            BinaryOpType::Multiply => "mul",
            BinaryOpType::Divide => "div",
            BinaryOpType::Modulo => "mod",
            BinaryOpType::Gt => "gt",
            BinaryOpType::Gte => "gte",
            BinaryOpType::Lt => "lt",
            BinaryOpType::Lte => "lte",
            BinaryOpType::Eq => "eq",
            BinaryOpType::Neq => "neq",
            BinaryOpType::And => "and",
            BinaryOpType::Or => "or",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TelParseAction {
    pub name: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TelParseError {
    from: usize,
    to: usize,
    severity: String,
    message: String,
    actions: Vec<TelParseAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "code", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TelError {
    ParseError {
        errors: Vec<TelParseError>,
    },
    ConversionError {
        message: String,
        from: String,
        to: String,
    },
    NotIndexable {
        message: String,
        subject: String,
    },
    NoAttribute {
        message: String,
        subject: String,
        attribute: String,
    },
    InvalidSelector {
        message: String,
    },
    UnsupportedOperation {
        operation: String,
        message: String,
    },
    FunctionNotFound(String),
    IndexOutOfBounds {
        index: usize,
        max: usize,
    },
    InvalidIndex {
        subject: String,
        message: String,
    },
    InvalidArgument {
        index: usize,
        method_name: String,
        expected: Option<String>,
        actual: Option<String>,
    },
    MissingArgument {
        index: usize,
        method_name: String,
    },
}

impl TelError {
    pub fn new_binary_unsupported(op: BinaryOpType, a: Description, b: Description) -> TelError {
        TelError::UnsupportedOperation {
            operation: op.get_label().to_owned(),
            message: format!(
                "Can't {} {} and {}",
                op.get_label().to_owned(),
                a.get_type(),
                b.get_type()
            ),
        }
    }

    pub fn new_unary_unsupported(op: UnaryOpType, a: Description) -> TelError {
        TelError::UnsupportedOperation {
            operation: op.get_label().to_owned(),
            message: format!("Can't use {} on {}", op.get_label(), a.get_type(),),
        }
    }

    pub fn get_code(&self) -> u32 {
        match self {
            TelError::ParseError { .. } => 1,
            TelError::ConversionError { .. } => 2,
            TelError::NotIndexable { .. } => 3,
            TelError::NoAttribute { .. } => 4,
            TelError::InvalidSelector { .. } => 5,
            TelError::UnsupportedOperation { .. } => 6,
            TelError::FunctionNotFound(_) => 7,
            TelError::IndexOutOfBounds { .. } => 8,
            TelError::InvalidIndex { .. } => 9,
            TelError::InvalidArgument { .. } => 10,
            TelError::MissingArgument { .. } => 11,
        }
    }
}

impl Display for TelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TelError::ParseError { errors } => {
                for error in errors {
                    writeln!(f, "{}", error.message)?;
                }
                Ok(())
            }
            TelError::ConversionError { message, from, to } => {
                write!(f, "{}: Can't convert {} to {}", message, from, to)
            }
            TelError::NotIndexable { message, subject } => {
                write!(f, "{}: {} is not indexable", message, subject)
            }
            TelError::NoAttribute {
                message,
                subject,
                attribute,
            } => {
                write!(f, "{}: {} has no attribute {}", message, subject, attribute)
            }
            TelError::InvalidSelector { message } => {
                write!(f, "Invalid selector: {}", message)
            }
            TelError::UnsupportedOperation { operation, message } => {
                write!(f, "Unsupported operation: {}: {}", operation, message)
            }
            TelError::FunctionNotFound(_) => {
                write!(f, "Function not found")
            }
            TelError::IndexOutOfBounds { index, max } => {
                write!(f, "Index out of bounds: {} > {}", index, max)
            }
            TelError::InvalidIndex { subject, message } => {
                write!(f, "Invalid index: {}: {}", subject, message)
            }
            TelError::InvalidArgument {
                index,
                method_name,
                expected,
                actual,
            } => {
                if let Some(expected) = expected {
                    if let Some(actual) = actual {
                        write!(
                            f,
                            "Invalid argument {} of {}, expected {} but got {}",
                            index, method_name, expected, actual
                        )
                    } else {
                        write!(
                            f,
                            "Invalid argument {} of {}, expected {}",
                            index, method_name, expected
                        )
                    }
                } else {
                    write!(f, "Invalid argument {} of {}", index, method_name)
                }
            }
            TelError::MissingArgument { index, method_name } => {
                write!(f, "Missing argument {} of {}", index, method_name)
            }
        }
    }
}

pub const NULL: StorageValue = StorageValue::Null(None);

/// Op enum for Token parsing
///
/// We cannot divide ops into unary and binary because some ops are using the same symbols
/// and we can't know if it's unary or binary until we parse the expression
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    Multiply,
    Divide,
    Modulo,
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
    And,
    Or,
    Plus,
    Minus,
    Negation,
}

impl std::fmt::Display for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Op::Multiply => write!(f, "*"),
            Op::Divide => write!(f, "/"),
            Op::Modulo => write!(f, "%"),
            Op::Gt => write!(f, ">"),
            Op::Gte => write!(f, ">="),
            Op::Lt => write!(f, "<"),
            Op::Lte => write!(f, "<="),
            Op::Eq => write!(f, "=="),
            Op::Neq => write!(f, "!="),
            Op::And => write!(f, "&&"),
            Op::Or => write!(f, "||"),
            Op::Plus => write!(f, "+"),
            Op::Minus => write!(f, "-"),
            Op::Negation => write!(f, "!"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Token {
    Null,
    Boolean(bool),
    Number(String),
    String(String),
    MultilineString { value: String, tag: String },
    EnvironmentRef(String),
    Op(Op),
    Ctrl(char),
    Identifier(String),
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Token::Null => write!(f, "null"),
            Token::Boolean(b) => write!(f, "{}", b),
            Token::Number(n) => write!(f, "{}", n),
            Token::String(s) => write!(f, "{}", s),
            Token::Op(op) => write!(f, "{}", op),
            Token::Ctrl(c) => write!(f, "{}", c),
            Token::Identifier(s) => write!(f, "{}", s),
            Token::EnvironmentRef(name) => write!(f, "${}", name),
            Token::MultilineString { value, tag } => write!(f, "```{}\n{}```", tag, value),
        }
    }
}

fn lexer() -> impl Parser<char, Vec<Spanned<Token>>, Error = Simple<char>> {
    let frac = just('.').chain(text::digits(10));

    let exp = just('e')
        .or(just('E'))
        .chain(just('+').or(just('-')).or_not())
        .chain::<char, _, _>(text::digits(10));

    let number = text::int(10)
        .chain::<char, _, _>(frac.or_not().flatten())
        .chain::<char, _, _>(exp.or_not().flatten())
        .collect::<String>()
        .map(Token::Number)
        .labelled("number");

    let escape = just('\\').ignore_then(
        just('\\')
            .or(just('/'))
            .or(just('"'))
            .or(just('b').to('\x08'))
            .or(just('f').to('\x0C'))
            .or(just('n').to('\n'))
            .or(just('r').to('\r'))
            .or(just('t').to('\t'))
            .or(just('u').ignore_then(
                filter(|c: &char| c.is_ascii_hexdigit())
                    .repeated()
                    .exactly(4)
                    .collect::<String>()
                    .validate(|digits, span, emit| {
                        char::from_u32(u32::from_str_radix(&digits, 16).unwrap()).unwrap_or_else(
                            || {
                                emit(Simple::custom(span, "invalid unicode character"));
                                '\u{FFFD}' // unicode replacement character
                            },
                        )
                    }),
            )),
    );

    let string = just('"')
        .ignore_then(filter(|c| *c != '\\' && *c != '"').or(escape).repeated())
        .then_ignore(just('"'))
        .collect::<String>()
        .map(Token::String)
        .labelled("string");

    let multiline_string = just("```")
        .ignore_then(take_until(just("\n").ignored()))
        .then(take_until(just("```").ignored()))
        .map(|((tag, _), (content, _))| Token::MultilineString {
            tag: tag.into_iter().collect(),
            value: content.into_iter().collect(),
        });

    let boolean = just("true")
        .to(Token::Boolean(true))
        .or(just("false").to(Token::Boolean(false)));

    let null = just("null").to(Token::Null);

    let identifier = text::ident().map(Token::Identifier).labelled("identifier");

    let environment_ref = just("$")
        .then(text::ident())
        .map(|(_, name)| Token::EnvironmentRef(name));

    // A parser for operators
    let op = choice((
        just("==").to(Op::Eq),
        just("!=").to(Op::Neq),
        just("<=").to(Op::Lte),
        just(">=").to(Op::Gte),
        just("<").to(Op::Lt),
        just(">").to(Op::Gt),
        just("*").to(Op::Multiply),
        just("/").to(Op::Divide),
        just("%").to(Op::Modulo),
        just("&&").to(Op::And),
        just("||").to(Op::Or),
        just("+").to(Op::Plus),
        just("-").to(Op::Minus),
        just("!").to(Op::Negation),
    ))
    .map(Token::Op);

    // A parser for control characters (delimiters, semicolons, etc.)
    let ctrl = one_of("()[]{};,?:.").map(Token::Ctrl);

    let token = choice((
        number,
        string,
        multiline_string,
        environment_ref,
        op,
        ctrl,
        null,
        boolean,
        identifier,
    ))
    .recover_with(skip_then_retry_until([]));

    let single_line = just::<_, _, Simple<char>>("//")
        .then(take_until(text::newline()))
        .ignored()
        .padded();

    let multi_line = just::<_, _, Simple<char>>("/*")
        .then(take_until(just("*/")))
        .ignored()
        .padded();

    let comment = single_line.or(multi_line);

    token
        .map_with_span(|tok, span| (tok, span))
        .padded_by(comment.repeated())
        .padded()
        .repeated()
}

enum Accessor {
    Attribute(String),
    Slice(Box<Spanned<Expr>>),
    MethodCall(String, Vec<Spanned<Expr>>),
}

fn parser() -> impl Parser<Token, Spanned<Expr>, Error = Simple<Token>> + Clone {
    recursive(|expr: Recursive<'_, Token, Spanned<Expr>, _>| {
        let basic_value = select! {
            Token::Null => Expr::Null,
            Token::Boolean(b) => Expr::Boolean(b),
            Token::Number(n) => {
                match n.parse() {
                    Ok(f) => Expr::Number(f),
                    Err(_) => Expr::Invalid,
                }
            },
            Token::EnvironmentRef(s) => Expr::Environment(s),
        }
        .labelled("value");

        let string = select! {
            Token::String(s) => s.clone(),
        }
        .labelled("string");

        let multiline_string = select! {
            Token::MultilineString { value, tag } => Expr::MultilineString { value, tag },
        }
        .labelled("multiline string");

        let identifier =
            select! { Token::Identifier(ident) => ident.clone() }.labelled("identifier");

        let array = expr
            .clone()
            .chain(just(Token::Ctrl(',')).ignore_then(expr.clone()).repeated())
            .or_not()
            .flatten()
            .delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')))
            .map(Expr::Array)
            .labelled("array");

        let object_key = string.or(identifier).labelled("object key");

        let field = object_key
            .then_ignore(just(Token::Ctrl(':')))
            .then(expr.clone());

        let object = field
            .clone()
            .chain(just(Token::Ctrl(',')).ignore_then(field).repeated())
            .or_not()
            .flatten()
            .delimited_by(just(Token::Ctrl('{')), just(Token::Ctrl('}')))
            .collect::<HashMap<String, Spanned<Expr>>>()
            .map(Expr::Object)
            .labelled("object");

        // Atom
        let atom: BoxedParser<'_, Token, Spanned<Expr>, Simple<Token>> = choice((
            basic_value,
            array,
            object,
            string.map(Expr::String),
            multiline_string,
            identifier.map(Expr::Identifier),
        ))
        .map_with_span(|expr, span| (expr, span))
        .or(expr
            .clone()
            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
            .recover_with(nested_delimiters(
                Token::Ctrl('('),
                Token::Ctrl(')'),
                [
                    (Token::Ctrl('{'), Token::Ctrl('}')),
                    (Token::Ctrl('['), Token::Ctrl(']')),
                ],
                |span| (Expr::Invalid, span),
            )))
        .boxed();

        // Accessor, basically method calls like .type(), attributes like .type and slices like [0]
        // These have the highest but equal precedence so we this enum to easily fold them later
        let accessor = choice((
            just(Token::Ctrl('.'))
                .ignore_then(identifier)
                .then(
                    expr.clone()
                        .separated_by(just(Token::Ctrl(',')))
                        .allow_trailing()
                        .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))),
                )
                .map_with_span(|(name, arguments), span| {
                    (Accessor::MethodCall(name, arguments), span)
                }),
            just(Token::Ctrl('.'))
                .ignored()
                .then(identifier)
                .map_with_span(|(_, name), span| (Accessor::Attribute(name), span)),
            expr.clone()
                .delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')))
                .recover_with(nested_delimiters(
                    Token::Ctrl('['),
                    Token::Ctrl(']'),
                    [
                        (Token::Ctrl('{'), Token::Ctrl('}')),
                        (Token::Ctrl('('), Token::Ctrl(')')),
                    ],
                    |span| (Expr::Invalid, span),
                ))
                .map_with_span(|expr, span| (Accessor::Slice(Box::new(expr)), span)),
        ));

        let primary = atom
            .clone()
            .then(accessor.repeated())
            .foldl(|a, b| match b.0 {
                Accessor::Attribute(name) => (Expr::Attribute(Box::new(a), name), b.1),
                Accessor::Slice(expr) => (Expr::Slice(Box::new(a), expr), b.1),
                Accessor::MethodCall(name, arguments) => (
                    Expr::MethodCall {
                        callee: Box::new(a),
                        name,
                        arguments,
                    },
                    b.1,
                ),
            });

        let op = choice((
            just(Token::Op(Op::Minus)).to(UnaryOpType::Minus),
            just(Token::Op(Op::Plus)).to(UnaryOpType::Plus),
            just(Token::Op(Op::Negation)).to(UnaryOpType::Negation),
        ))
        .map_with_span(|op, span| (op, span));

        // Unary operators
        let unary = op
            .repeated()
            .then(primary.clone())
            .foldr(|(op, span), expr| {
                let span = span.start..expr.1.end;
                (Expr::UnaryOp(op, Box::new(expr)), span)
            });

        // Multiply and divide
        let op = choice((
            just(Token::Op(Op::Multiply)).to(BinaryOpType::Multiply),
            just(Token::Op(Op::Divide)).to(BinaryOpType::Divide),
            just(Token::Op(Op::Modulo)).to(BinaryOpType::Modulo),
        ));

        let mul = unary
            .clone()
            .then(op.then(unary).repeated())
            .foldl(|a, (op, b)| {
                let span = a.1.start..b.1.end;
                (
                    Expr::BinaryOp {
                        lhs: Box::new(a),
                        op,
                        rhs: Box::new(b),
                    },
                    span,
                )
            });

        // Add and subtract
        let op = choice((
            just(Token::Op(Op::Plus)).to(BinaryOpType::Add),
            just(Token::Op(Op::Minus)).to(BinaryOpType::Subtract),
        ));

        let sum = mul
            .clone()
            .then(op.then(mul).repeated())
            .foldl(|a, (op, b)| {
                let span = a.1.start..b.1.end;
                (
                    Expr::BinaryOp {
                        lhs: Box::new(a),
                        op,
                        rhs: Box::new(b),
                    },
                    span,
                )
            });

        // Comparison
        let op = choice((
            just(Token::Op(Op::Eq)).to(BinaryOpType::Eq),
            just(Token::Op(Op::Neq)).to(BinaryOpType::Neq),
            just(Token::Op(Op::Lte)).to(BinaryOpType::Lte),
            just(Token::Op(Op::Gte)).to(BinaryOpType::Gte),
            just(Token::Op(Op::Lt)).to(BinaryOpType::Lt),
            just(Token::Op(Op::Gt)).to(BinaryOpType::Gt),
        ));

        let compare = sum
            .clone()
            .then(op.then(sum).repeated())
            .foldl(|a, (op, b)| {
                let span = a.1.start..b.1.end;
                (
                    Expr::BinaryOp {
                        lhs: Box::new(a),
                        op,
                        rhs: Box::new(b),
                    },
                    span,
                )
            });

        // Logical
        let op = choice((
            just(Token::Op(Op::And)).to(BinaryOpType::And),
            just(Token::Op(Op::Or)).to(BinaryOpType::Or),
        ));

        let logic = compare
            .clone()
            .then(op.then(compare).repeated())
            .foldl(|a, (op, b)| {
                let span = a.1.start..b.1.end;
                (
                    Expr::BinaryOp {
                        lhs: Box::new(a),
                        op,
                        rhs: Box::new(b),
                    },
                    span,
                )
            });

        // Inline conditional
        let if_ = logic
            .clone()
            .then(
                just(Token::Ctrl('?'))
                    .ignore_then(expr.clone())
                    .then(just(Token::Ctrl(':')).ignore_then(expr.clone()))
                    .repeated(),
            )
            .foldl(|a, (b, c)| {
                let span = a.1.start..c.1.end;

                (
                    Expr::If {
                        condition: Box::new(a),
                        then: Box::new(b),
                        otherwise: Box::new(c),
                    },
                    span,
                )
            });

        if_.recover_with(nested_delimiters(
            Token::Ctrl('['),
            Token::Ctrl(']'),
            [
                (Token::Ctrl('{'), Token::Ctrl('}')),
                (Token::Ctrl('('), Token::Ctrl(')')),
            ],
            |span| (Expr::Invalid, span),
        ))
        .recover_with(nested_delimiters(
            Token::Ctrl('('),
            Token::Ctrl(')'),
            [
                (Token::Ctrl('{'), Token::Ctrl('}')),
                (Token::Ctrl('['), Token::Ctrl(']')),
            ],
            |span| (Expr::Invalid, span),
        ))
        .recover_with(nested_delimiters(
            Token::Ctrl('{'),
            Token::Ctrl('}'),
            [
                (Token::Ctrl('['), Token::Ctrl(']')),
                (Token::Ctrl('('), Token::Ctrl(')')),
            ],
            |span| (Expr::Invalid, span),
        ))
    })
    .then_ignore(end().recover_with(skip_then_retry_until([])))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    pub expr: Option<Spanned<Expr>>,
    pub errors: Vec<TelParseError>,
}

pub fn parse(input: &str) -> ParseResult {
    let (tokens, errors) = lexer().parse_recovery(input);

    let mut token_errors: Vec<TelParseError> = errors
        .into_iter()
        .map(|e| match e.reason() {
            chumsky::error::SimpleReason::Unexpected => {
                let span = e.span();

                TelParseError {
                    from: span.start(),
                    to: span.end(),
                    severity: "error".to_owned(),
                    message: "Unexpected token".to_owned(),
                    actions: vec![],
                }
            }
            chumsky::error::SimpleReason::Unclosed { span, delimiter } => TelParseError {
                from: span.start(),
                to: span.end(),
                severity: "error".to_owned(),
                message: format!("Unclosed {}", delimiter),
                actions: vec![],
            },
            chumsky::error::SimpleReason::Custom(e) => TelParseError {
                from: 0,
                to: 0,
                severity: "error".to_owned(),
                message: e.to_owned(),
                actions: vec![],
            },
        })
        .collect();

    if let Some(tokens) = tokens {
        let len = input.chars().count();
        let (expr, errors) =
            parser().parse_recovery(Stream::from_iter(len..len + 1, tokens.into_iter()));

        let mut expr_errors: Vec<TelParseError> = errors
            .into_iter()
            .map(|e| match e.reason() {
                chumsky::error::SimpleReason::Unexpected => {
                    let span = e.span();

                    TelParseError {
                        from: span.start(),
                        to: span.end(),
                        severity: "error".to_owned(),
                        message: "Unexpected token".to_owned(),
                        actions: vec![],
                    }
                }
                chumsky::error::SimpleReason::Unclosed { span, delimiter } => TelParseError {
                    from: span.start(),
                    to: span.end(),
                    severity: "error".to_owned(),
                    message: format!("Unclosed {}", delimiter),
                    actions: vec![],
                },
                chumsky::error::SimpleReason::Custom(e) => TelParseError {
                    from: 0,
                    to: 0,
                    severity: "error".to_owned(),
                    message: e.to_owned(),
                    actions: vec![],
                },
            })
            .collect();

        // Gather all errors
        expr_errors.append(&mut token_errors);

        return ParseResult {
            expr,
            errors: expr_errors,
        };
    }

    ParseResult {
        expr: None,
        errors: token_errors,
    }
}

pub fn evaluate_value<T: Storage>(
    expr: Spanned<Expr>,
    storage: &T,
    environment: &HashMap<String, StorageValue>,
) -> Result<StorageValue, TelError> {
    let out = match expr.0 {
        Expr::Null => StorageValue::Null(None),
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            let value = evaluate_value(*condition, storage, environment)?;

            match value {
                StorageValue::Boolean(b) => {
                    if b {
                        evaluate_value(*then, storage, environment)?
                    } else {
                        evaluate_value(*otherwise, storage, environment)?
                    }
                }
                _ => {
                    return Err(TelError::UnsupportedOperation {
                        operation: "if".to_owned(),
                        message: "Condition must be boolean".to_owned(),
                    })
                }
            }
        }
        Expr::Number(f) => StorageValue::Number(f),
        Expr::String(s) => StorageValue::String(s),
        Expr::Boolean(b) => StorageValue::Boolean(b),
        Expr::Array(n) => {
            let data: Result<Vec<StorageValue>, TelError> = n
                .into_iter()
                .map(|e| evaluate_value(e, storage, environment))
                .collect();

            StorageValue::Array(data?)
        }
        Expr::Object(n) => {
            let data: Result<HashMap<String, StorageValue>, TelError> = n
                .into_iter()
                .map(|(k, v)| {
                    let v = evaluate_value(v, storage, environment);
                    v.map(|v| (k, v))
                })
                .collect();

            StorageValue::Object(data?)
        }
        Expr::Identifier(iden) => match storage.get(&iden) {
            Some(v) => v.clone(),
            None => NULL,
        },
        Expr::Environment(iden) => match environment.get(&iden) {
            Some(v) => v.clone(),
            None => NULL,
        },
        Expr::Attribute(expr, attr) => {
            let expr = evaluate_value(*expr, storage, environment)?;

            match expr {
                StorageValue::Object(map) => match map.get(&attr) {
                    Some(v) => v.clone(),
                    None => NULL,
                },
                StorageValue::Array(vec) => match attr.as_str() {
                    "length" => vec.len().into(),
                    _ => NULL,
                },
                StorageValue::String(s) => match attr.as_str() {
                    "length" => s.len().into(),
                    _ => NULL,
                },
                StorageValue::Number(f) => match attr.as_str() {
                    "isInteger" => StorageValue::Boolean(f.fract() == 0.0),
                    _ => NULL,
                },
                _ => NULL,
            }
        }
        Expr::Slice(expr, slice_expr) => {
            let expr = evaluate_value(*expr, storage, environment)?;
            let slice = evaluate_value(*slice_expr, storage, environment)?;

            match expr {
                StorageValue::String(s) => {
                    let index = slice.as_index()?;
                    match s.chars().nth(index) {
                        Some(c) => StorageValue::String(c.to_string()),
                        None => NULL,
                    }
                }
                StorageValue::Array(arr) => {
                    let index = slice.as_index()?;
                    match arr.get(index) {
                        Some(v) => v.clone(),
                        None => NULL,
                    }
                }
                StorageValue::Object(obj) => {
                    let key = slice.to_string()?;
                    match obj.get(&key) {
                        Some(v) => v.clone(),
                        None => NULL,
                    }
                }
                _ => NULL,
            }
        }
        Expr::UnaryOp(op, expr) => {
            let expr = evaluate_value(*expr, storage, environment)?;
            match op {
                UnaryOpType::Negation => operators::neg(expr)?,
                UnaryOpType::Minus => operators::minus(expr)?,
                UnaryOpType::Plus => operators::plus(expr)?,
            }
        }
        Expr::BinaryOp { lhs, op, rhs } => {
            if matches!(op, BinaryOpType::And) {
                let l = evaluate_value(*lhs, storage, environment)?;
                let left = l.to_boolean()?;
                if left {
                    let r = evaluate_value(*rhs, storage, environment)?;
                    return Ok(r);
                }
                return Ok(StorageValue::Boolean(false));
            }

            if matches!(op, BinaryOpType::Or) {
                let l = evaluate_value(*lhs, storage, environment)?;
                let left = l.to_boolean()?;
                if left {
                    return Ok(l.clone());
                }
                let r = evaluate_value(*rhs, storage, environment)?;
                return Ok(r);
            }

            let l = evaluate_value(*lhs, storage, environment)?;
            let r = evaluate_value(*rhs, storage, environment)?;

            match op {
                BinaryOpType::Add => operators::add(l, r)?,
                BinaryOpType::Subtract => operators::sub(l, r)?,
                BinaryOpType::Multiply => operators::mul(l, r)?,
                BinaryOpType::Divide => operators::div(l, r)?,
                BinaryOpType::Modulo => operators::rem(l, r)?,
                BinaryOpType::Gt => StorageValue::Boolean(l > r),
                BinaryOpType::Gte => StorageValue::Boolean(l >= r),
                BinaryOpType::Lt => StorageValue::Boolean(l < r),
                BinaryOpType::Lte => StorageValue::Boolean(l <= r),
                BinaryOpType::Eq => StorageValue::Boolean(l == r),
                BinaryOpType::Neq => StorageValue::Boolean(l != r),
                BinaryOpType::And => panic!("AND operation should be handled above"),
                BinaryOpType::Or => panic!("OR operation should be handled above"),
            }
        }
        Expr::MethodCall {
            callee,
            name,
            mut arguments,
        } => {
            let value = evaluate_value(*callee, storage, environment)?;

            match value {
                StorageValue::String(s) => match name.as_str() {
                    "length" => s.len().into(),
                    "type" => StorageValue::String("string".to_string()),
                    "toString" => StorageValue::String(s),
                    "toNumber" => StorageValue::Number(s.parse::<f64>().map_err(|_| {
                        TelError::ConversionError {
                            message: "Failed to convert string to number".to_owned(),
                            from: "string".to_string(),
                            to: "number".to_string(),
                        }
                    })?),
                    "contains" => {
                        let arg = arguments.pop();
                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?.to_string()?;
                            StorageValue::Boolean(s.contains(&arg))
                        } else {
                            StorageValue::Boolean(false)
                        }
                    }
                    "toUpperCase" => StorageValue::String(s.to_uppercase()),
                    "toLowerCase" => StorageValue::String(s.to_lowercase()),
                    "trim" => StorageValue::String(s.trim().to_string()),
                    "isEmpty" => StorageValue::Boolean(s.is_empty()),
                    "replace" => {
                        let mut arguments = VecDeque::from(arguments);
                        let from = arguments.pop_front();
                        let to = arguments.pop_front();
                        if let Some(from) = from {
                            let from = evaluate_value(from, storage, environment)?;
                            let from = from.to_string()?;

                            if let Some(to) = to {
                                let to = evaluate_value(to, storage, environment)?;
                                let to = to.to_string()?;

                                println!("Replacing {} with {}", from, to);

                                StorageValue::String(s.replace(from.as_str(), to.as_str()))
                            } else {
                                StorageValue::String(s.replace(from.as_str(), ""))
                            }
                        } else {
                            return Err(TelError::MissingArgument {
                                index: 0,
                                method_name: "replace".to_owned(),
                            });
                        }
                    }
                    "stripPrefix" => {
                        let arg = arguments.pop();
                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?;
                            let arg = arg.to_string()?;

                            if let Some(removed) = s.strip_prefix(arg.as_str()) {
                                StorageValue::String(removed.to_string())
                            } else {
                                s.into()
                            }
                        } else {
                            s.trim_start().into()
                        }
                    }
                    "stripSuffix" => {
                        let arg = arguments.pop();
                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?;
                            let arg = arg.to_string()?;

                            if let Some(removed) = s.strip_suffix(arg.as_str()) {
                                StorageValue::String(removed.to_string())
                            } else {
                                s.into()
                            }
                        } else {
                            s.trim_end().into()
                        }
                    }
                    "startsWith" => {
                        let arg = arguments.pop();
                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?;
                            let arg = arg.to_string()?;
                            StorageValue::Boolean(s.starts_with(&arg))
                        } else {
                            StorageValue::Boolean(false)
                        }
                    }
                    "endsWith" => {
                        let arg = arguments.pop();
                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?;
                            let arg = arg.to_string()?;
                            StorageValue::Boolean(s.ends_with(&arg))
                        } else {
                            StorageValue::Boolean(false)
                        }
                    }
                    _ => NULL,
                },
                StorageValue::Number(f) => match name.as_str() {
                    "toString" => StorageValue::String(f.to_string()),
                    "type" => StorageValue::String("number".to_string()),
                    "round" => StorageValue::Number(f.round()),
                    "floor" => StorageValue::Number(f.floor()),
                    "ceil" => StorageValue::Number(f.ceil()),
                    "abs" => StorageValue::Number(f.abs()),
                    "pow" => {
                        let arg = arguments.pop();
                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?;
                            if let StorageValue::Number(arg) = arg {
                                return Ok(StorageValue::Number(f.powf(arg)));
                            } else {
                                return Err(TelError::InvalidArgument {
                                    index: 0,
                                    method_name: "pow".to_owned(),
                                    expected: Some("number".to_owned()),
                                    actual: Some(arg.get_type().to_owned()),
                                });
                            }
                        } else {
                            return Err(TelError::MissingArgument {
                                index: 0,
                                method_name: "pow".to_owned(),
                            });
                        }
                    }
                    "cos" => StorageValue::Number(f.cos()),
                    "sin" => StorageValue::Number(f.sin()),
                    "tan" => StorageValue::Number(f.tan()),
                    "acos" => StorageValue::Number(f.acos()),
                    "asin" => StorageValue::Number(f.asin()),
                    "atan" => StorageValue::Number(f.atan()),
                    "sqrt" => StorageValue::Number(f.sqrt()),
                    "exp" => StorageValue::Number(f.exp()),
                    "ln" => StorageValue::Number(f.ln()),
                    "log" => {
                        let arg = arguments.pop();
                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?;
                            if let StorageValue::Number(arg) = arg {
                                return Ok(StorageValue::Number(f.log(arg)));
                            } else {
                                return Err(TelError::InvalidArgument {
                                    index: 0,
                                    method_name: "log".to_owned(),
                                    expected: Some("number".to_owned()),
                                    actual: Some(arg.get_type().to_owned()),
                                });
                            }
                        }
                        return Err(TelError::MissingArgument {
                            index: 0,
                            method_name: "log".to_owned(),
                        });
                    }
                    "log10" => StorageValue::Number(f.log10()),
                    "sign" => StorageValue::Number(f.signum()),
                    "isInteger" => StorageValue::Boolean(f.fract() == 0.0),
                    "isNaN" => StorageValue::Boolean(f.is_nan()),
                    "isFinite" => StorageValue::Boolean(f.is_finite()),
                    "isInfinite" => StorageValue::Boolean(f.is_infinite()),
                    "isEven" => StorageValue::Boolean(f.fract() == 0.0 && f % 2.0 == 0.0),
                    "isOdd" => StorageValue::Boolean(f.fract() == 0.0 && f % 2.0 != 0.0),
                    "toNumber" => StorageValue::Number(f),
                    "truncate" => StorageValue::Number(f.trunc()),
                    _ => NULL,
                },
                StorageValue::Boolean(b) => match name.as_str() {
                    "toString" => StorageValue::String(b.to_string()),
                    "type" => StorageValue::String("boolean".to_string()),
                    _ => NULL,
                },
                StorageValue::Array(arr) => match name.as_str() {
                    "type" => StorageValue::String("array".to_string()),
                    "join" => {
                        let arg = arguments.pop();
                        let strings = arr
                            .iter()
                            .map(|v| v.to_string())
                            .collect::<Result<Vec<String>, _>>()?;

                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?;
                            let arg = arg.to_string()?;
                            StorageValue::String(strings.join(&arg))
                        } else {
                            StorageValue::String(strings.join(", "))
                        }
                    }
                    "contains" => {
                        let arg = arguments.pop();
                        if let Some(arg) = arg {
                            let arg = evaluate_value(arg, storage, environment)?;
                            StorageValue::Boolean(arr.contains(&arg))
                        } else {
                            StorageValue::Boolean(false)
                        }
                    }
                    "length" => StorageValue::Number(arr.len() as f64),
                    _ => NULL,
                },
                StorageValue::Object(object) => match name.as_str() {
                    "type" => StorageValue::String("object".to_string()),
                    "values" => {
                        StorageValue::Array(object.values().cloned().collect::<Vec<StorageValue>>())
                    }
                    "keys" => StorageValue::Array(
                        object
                            .keys()
                            .map(|k| StorageValue::String(k.clone()))
                            .collect(),
                    ),
                    _ => NULL,
                },
                StorageValue::Null(_) => match name.as_str() {
                    "toString" => StorageValue::String("null".to_string()),
                    "type" => StorageValue::String("null".to_string()),
                    _ => NULL,
                },
            }
        }
        Expr::Invalid => {
            return Err(TelError::UnsupportedOperation {
                operation: "invalid".to_owned(),
                message: "Invalid expression".to_owned(),
            })
        }
        Expr::MultilineString {
            value: data,
            tag: _,
        } => StorageValue::String(data),
    };

    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SelectorPart {
    Identifier(String),
    Attribute(String),
    Slice(StorageValue),
    Null,
}

/// A selector is a list of parts that can be used to point to a place in a storage
/// It can be a simple identifier, an object's attribute, a array slice or a null value (write to nothing like in /dev/null)
pub type Selector = Vec<SelectorPart>;

pub fn evaluate_selector(
    expr: Spanned<Expr>,
    storage: &HashMap<String, StorageValue>,
    environment: &HashMap<String, StorageValue>,
) -> Result<Selector, TelError> {
    match expr.0 {
        Expr::Null => Ok(vec![SelectorPart::Null]),
        Expr::Identifier(iden) => Ok(vec![SelectorPart::Identifier(iden)]),
        Expr::Attribute(expr, attr) => {
            let mut selectors = evaluate_selector(*expr, storage, environment)?;
            selectors.push(SelectorPart::Attribute(attr));
            Ok(selectors)
        }
        Expr::Slice(expr, slice_expr) => {
            let mut selectors = evaluate_selector(*expr, storage, environment)?;
            let value = evaluate_value(*slice_expr, storage, environment)?;
            selectors.push(SelectorPart::Slice(value));
            Ok(selectors)
        }
        Expr::If {
            condition,
            then,
            otherwise,
        } => {
            let value = evaluate_value(*condition, storage, environment)?;

            match value {
                StorageValue::Boolean(b) => {
                    if b {
                        Ok(evaluate_selector(*then, storage, environment)?)
                    } else {
                        Ok(evaluate_selector(*otherwise, storage, environment)?)
                    }
                }
                _ => Err(TelError::UnsupportedOperation {
                    message: "Condition must be a boolean".to_owned(),
                    operation: "if".to_owned(),
                }),
            }
        }
        e => Err(TelError::InvalidSelector {
            message: format!("Invalid selector containing: {:?}", e), // TODO: Better Expr visualization
        })?,
    }
}

enum ContextStorage<'a> {
    Object(&'a mut HashMap<String, StorageValue>),
    Array(&'a mut Vec<StorageValue>),
}

pub fn store_value(
    selectors: &Vec<SelectorPart>,
    storage: &mut HashMap<String, StorageValue>,
    value: StorageValue,
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
                    ContextStorage::Array(_) => {
                        unreachable!()
                    }
                },
                SelectorPart::Attribute(attr) => match traversed {
                    ContextStorage::Object(obj) => {
                        obj.insert(attr.to_owned(), value);
                        return Ok(());
                    }
                    ContextStorage::Array(_) => {
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
                },
                SelectorPart::Null => return Ok(()),
            },
            // This is not the last element so we need to traverse further
            (part, false) => match part {
                SelectorPart::Identifier(identifier) => match traversed {
                    ContextStorage::Object(obj) => match obj.get_mut(identifier) {
                        Some(value) => match value {
                            StorageValue::Object(o) => {
                                traversed = ContextStorage::Object(o);
                            }
                            StorageValue::Array(arr) => {
                                traversed = ContextStorage::Array(arr);
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
                    ContextStorage::Array(_) => {
                        unreachable!()
                    }
                },
                SelectorPart::Attribute(attr) => match traversed {
                    ContextStorage::Object(obj) => match obj.get_mut(attr) {
                        Some(value) => match value {
                            StorageValue::Object(o) => {
                                traversed = ContextStorage::Object(o);
                            }
                            StorageValue::Array(arr) => {
                                traversed = ContextStorage::Array(arr);
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
                    ContextStorage::Array(_) => {
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
                                StorageValue::Object(o) => {
                                    traversed = ContextStorage::Object(o);
                                }
                                StorageValue::Array(arr) => {
                                    traversed = ContextStorage::Array(arr);
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
                            Some(value) => match value {
                                StorageValue::Object(o) => {
                                    traversed = ContextStorage::Object(o);
                                }
                                StorageValue::Array(arr) => {
                                    traversed = ContextStorage::Array(arr);
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
                },
                SelectorPart::Null => return Ok(()),
            },
        }
    }
    panic!("Should not happen, right?")
}

#[cfg(test)]
mod test_tel {
    use super::*;

    #[test]
    fn test_string_replace() {
        let result = evaluate_value(
            parse("(\"hello world\").replace(\"world\", \"there\")")
                .expr
                .unwrap(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(result, StorageValue::String("hello there".to_owned()));
    }

    #[test]
    fn test_serialize_number_as_integer() {
        assert_eq!(
            serde_json::to_string(&StorageValue::Number(1.1)).unwrap(),
            "1.1"
        );
        assert_eq!(
            serde_json::to_string(&StorageValue::Number(1.0)).unwrap(),
            "1"
        );
    }

    #[test]
    fn test_recognizes_multiline_string() {
        let input = r#"```
foo: bar
```"#;

        let result = parse(input);
        assert_eq!(
            result.expr.unwrap(),
            Spanned::new(
                Expr::MultilineString {
                    value: "foo: bar\n".to_owned(),
                    tag: "".to_owned()
                },
                0..input.len()
            )
        );
    }

    #[test]
    fn test_recognizes_multiline_string_with_tag() {
        let input = r#"```yaml
foo: bar
```"#;

        let result = parse(input);
        assert_eq!(
            result.expr.unwrap(),
            Spanned::new(
                Expr::MultilineString {
                    value: "foo: bar\n".to_owned(),
                    tag: "yaml".to_owned()
                },
                0..input.len()
            )
        );
    }
}
