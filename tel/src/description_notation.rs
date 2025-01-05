use chumsky::{prelude::*, Parser, Stream};
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, vec};

use crate::{Description, Spanned, TelError, TelParseError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DExpr {
    Null,
    Number(f64),
    String(String),
    Boolean(bool),
    Array(Vec<Spanned<Self>>),
    Object(HashMap<String, Spanned<DExpr>>),
    Identifier(String),
    Attribute(Box<Spanned<Self>>, String),
    ArrayOf(Box<Spanned<Self>>),
    BinaryOp {
        lhs: Box<Spanned<Self>>,
        op: DBinaryOpType,
        rhs: Box<Spanned<Self>>,
    },
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum DBinaryOpType {
    Or,
}

/// Op enum for Token parsing
///
/// We cannot divide ops into unary and binary because some ops are using the same symbols
/// and we can't know if it's unary or binary until we parse the expression
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash)]
#[serde(rename_all = "camelCase")]
enum DOp {
    Or,
}

impl std::fmt::Display for DOp {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DOp::Or => write!(f, "|"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum DToken {
    Null,
    Boolean(bool),
    Number(String),
    String(String),
    Op(DOp),
    Ctrl(char),
    Identifier(String),
}

impl std::fmt::Display for DToken {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DToken::Null => write!(f, "null"),
            DToken::Boolean(b) => write!(f, "{}", b),
            DToken::Number(n) => write!(f, "{}", n),
            DToken::String(s) => write!(f, "{}", s),
            DToken::Op(op) => write!(f, "{}", op),
            DToken::Ctrl(c) => write!(f, "{}", c),
            DToken::Identifier(s) => write!(f, "{}", s),
        }
    }
}

fn lexer() -> impl Parser<char, Vec<Spanned<DToken>>, Error = Simple<char>> {
    let frac = just('.').chain(text::digits(10));

    let exp = just('e')
        .or(just('E'))
        .chain(just('+').or(just('-')).or_not())
        .chain::<char, _, _>(text::digits(10));

    let number = text::int(10)
        .chain::<char, _, _>(frac.or_not().flatten())
        .chain::<char, _, _>(exp.or_not().flatten())
        .collect::<String>()
        .map(DToken::Number)
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
        .map(DToken::String)
        .labelled("string");

    let boolean = just("true")
        .to(DToken::Boolean(true))
        .or(just("false").to(DToken::Boolean(false)));

    let null = just("null").to(DToken::Null);

    let identifier = text::ident().map(DToken::Identifier).labelled("identifier");

    // A parser for operators
    let op = just("|").to(DOp::Or).map(DToken::Op);

    // A parser for control characters (delimiters, semicolons, etc.)
    let ctrl = one_of("()[]{};,?:.").map(DToken::Ctrl);

    let token = choice((number, string, op, ctrl, null, boolean, identifier))
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
    EmptySlice,
}

fn parser() -> impl Parser<DToken, Spanned<DExpr>, Error = Simple<DToken>> + Clone {
    recursive(|expr: Recursive<'_, DToken, Spanned<DExpr>, _>| {
        let basic_value = select! {
            DToken::Null => DExpr::Null,
            DToken::Boolean(b) => DExpr::Boolean(b),
            DToken::Number(n) => {
                match n.parse() {
                    Ok(f) => DExpr::Number(f),
                    Err(_) => DExpr::Invalid,
                }
            },
        }
        .labelled("value");

        let string = select! {
            DToken::String(s) => s.clone(),
        }
        .labelled("string");

        let identifier =
            select! { DToken::Identifier(ident) => ident.clone() }.labelled("identifier");

        let array = expr
            .clone()
            .chain(just(DToken::Ctrl(',')).ignore_then(expr.clone()).repeated())
            .or_not()
            .flatten()
            .delimited_by(just(DToken::Ctrl('[')), just(DToken::Ctrl(']')))
            .map(DExpr::Array)
            .labelled("array");

        let object_key = string.or(identifier).labelled("object key");

        let field = object_key
            .then_ignore(just(DToken::Ctrl(':')))
            .then(expr.clone());

        let object = field
            .clone()
            .chain(just(DToken::Ctrl(',')).ignore_then(field).repeated())
            .or_not()
            .flatten()
            .delimited_by(just(DToken::Ctrl('{')), just(DToken::Ctrl('}')))
            .collect::<HashMap<String, Spanned<DExpr>>>()
            .map(DExpr::Object)
            .labelled("object");

        // Atom
        let atom: BoxedParser<'_, DToken, Spanned<DExpr>, Simple<DToken>> = choice((
            basic_value,
            array,
            object,
            string.map(DExpr::String),
            identifier.map(DExpr::Identifier),
        ))
        .map_with_span(|expr, span| (expr, span))
        .or(expr
            .clone()
            .delimited_by(just(DToken::Ctrl('(')), just(DToken::Ctrl(')')))
            .recover_with(nested_delimiters(
                DToken::Ctrl('('),
                DToken::Ctrl(')'),
                [
                    (DToken::Ctrl('{'), DToken::Ctrl('}')),
                    (DToken::Ctrl('['), DToken::Ctrl(']')),
                ],
                |span| (DExpr::Invalid, span),
            )))
        .boxed();

        // Accessor, basically method calls like .type(), attributes like .type and slices like [0]
        // These have the highest but equal precedence so we this enum to easily fold them later
        let accessor = choice((
            just(DToken::Ctrl('.'))
                .ignored()
                .then(identifier)
                .map_with_span(|(_, name), span| (Accessor::Attribute(name), span)),
            just(DToken::Ctrl('['))
                .then(just(DToken::Ctrl(']')).ignored())
                .map_with_span(|_, span| (Accessor::EmptySlice, span)),
        ));

        let primary = atom
            .clone()
            .then(accessor.repeated())
            .foldl(|a, b| match b.0 {
                Accessor::Attribute(name) => (DExpr::Attribute(Box::new(a), name), b.1),
                Accessor::EmptySlice => (DExpr::ArrayOf(Box::new(a)), b.1),
            });

        // Logical
        let op = choice((just(DToken::Op(DOp::Or)).to(DBinaryOpType::Or),));

        let logic = primary
            .clone()
            .then(op.then(primary).repeated())
            .foldl(|a, (op, b)| {
                let span = a.1.start..b.1.end;
                (
                    DExpr::BinaryOp {
                        lhs: Box::new(a),
                        op,
                        rhs: Box::new(b),
                    },
                    span,
                )
            });

        logic
            .recover_with(nested_delimiters(
                DToken::Ctrl('['),
                DToken::Ctrl(']'),
                [
                    (DToken::Ctrl('{'), DToken::Ctrl('}')),
                    (DToken::Ctrl('('), DToken::Ctrl(')')),
                ],
                |span| (DExpr::Invalid, span),
            ))
            .recover_with(nested_delimiters(
                DToken::Ctrl('('),
                DToken::Ctrl(')'),
                [
                    (DToken::Ctrl('{'), DToken::Ctrl('}')),
                    (DToken::Ctrl('['), DToken::Ctrl(']')),
                ],
                |span| (DExpr::Invalid, span),
            ))
            .recover_with(nested_delimiters(
                DToken::Ctrl('{'),
                DToken::Ctrl('}'),
                [
                    (DToken::Ctrl('['), DToken::Ctrl(']')),
                    (DToken::Ctrl('('), DToken::Ctrl(')')),
                ],
                |span| (DExpr::Invalid, span),
            ))
    })
    .then_ignore(end().recover_with(skip_then_retry_until([])))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    pub expr: Option<Spanned<DExpr>>,
    pub errors: Vec<TelParseError>,
}

pub fn parse_description(input: &str) -> ParseResult {
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

pub fn evaluate_description_notation(expr: Spanned<DExpr>) -> Result<Description, TelError> {
    let out = match expr.0 {
        DExpr::Null => Description::Null,
        DExpr::Number(f) => Description::NumberValue { value: f },
        DExpr::String(s) => Description::StringValue { value: s },
        DExpr::Boolean(b) => Description::BooleanValue { value: b },
        DExpr::Array(n) => {
            let data: Result<Vec<Description>, TelError> =
                n.into_iter().map(evaluate_description_notation).collect();

            Description::ExactArray { value: data? }
        }
        DExpr::Object(n) => {
            let data: Result<HashMap<String, Description>, TelError> = n
                .into_iter()
                .map(|(k, v)| {
                    let v = evaluate_description_notation(v);
                    v.map(|v| (k, v))
                })
                .collect();

            Description::Object { value: data? }
        }
        DExpr::Identifier(iden) => match iden.as_str() {
            "any" => Description::Any,
            iden => Description::new_base_type(iden),
        },
        DExpr::Attribute(expr, attr) => {
            let expr = evaluate_description_notation(*expr)?;

            match expr {
                Description::BaseType { field_type } => {
                    Description::new_base_type(&format!("{}.{}", field_type, attr))
                }
                _ => {
                    return Err(TelError::NoAttribute {
                        message: "No attribute".to_owned(),
                        subject: expr.get_type(),
                        attribute: attr,
                    })
                }
            }
        }
        DExpr::BinaryOp { lhs, op, rhs } => match op {
            DBinaryOpType::Or => {
                let l = evaluate_description_notation(*lhs)?;
                let r = evaluate_description_notation(*rhs)?;

                return Ok(Description::new_union(vec![l, r]));
            }
        },
        DExpr::Invalid => {
            return Err(TelError::UnsupportedOperation {
                operation: "invalid".to_owned(),
                message: "Invalid expression".to_owned(),
            })
        }
        DExpr::ArrayOf(expr) => {
            let expr = evaluate_description_notation(*expr)?;
            Description::Array {
                item_type: Box::new(expr),
                length: None,
            }
        }
    };

    Ok(out)
}

#[cfg(test)]
mod test_descr {
    use super::*;

    fn parse_and_evaluate_description_notation(input: &str) -> Result<Description, TelError> {
        let result: ParseResult = parse_description(input);
        if let Some(expr) = result.expr {
            evaluate_description_notation(expr)
        } else {
            Err(TelError::ParseError {
                errors: result.errors,
            })
        }
    }

    #[test]
    fn test_simple_type() {
        let result = parse_and_evaluate_description_notation("string").unwrap();
        assert_eq!(
            result,
            Description::BaseType {
                field_type: "string".to_owned()
            }
        );
    }

    #[test]
    fn test_simple_type2() {
        let result = parse_and_evaluate_description_notation("number").unwrap();
        assert_eq!(result, Description::new_base_type("number"));
    }

    #[test]
    fn test_object() {
        let result = parse_and_evaluate_description_notation("{ a: 4, b: string }").unwrap();
        let mut map = HashMap::new();
        map.insert("a".to_owned(), Description::NumberValue { value: 4.0 });
        map.insert(
            "b".to_owned(),
            Description::BaseType {
                field_type: "string".to_owned(),
            },
        );
        assert_eq!(result, Description::Object { value: map });
    }

    #[test]
    fn test_simple_array() {
        let result = parse_and_evaluate_description_notation("string[]").unwrap();
        assert_eq!(
            result,
            Description::Array {
                item_type: Box::new(Description::new_base_type("string")),
                length: None
            }
        );
    }

    #[test]
    fn test_array_with_union() {
        let result = parse_and_evaluate_description_notation("(string | boolean)[]").unwrap();
        assert_eq!(
            result,
            Description::Array {
                item_type: Box::new(Description::new_union(vec![
                    Description::new_base_type("string"),
                    Description::new_base_type("boolean")
                ])),
                length: None
            }
        );
    }

    #[test]
    fn test_exact_array() {
        let result = parse_and_evaluate_description_notation("[string, boolean]").unwrap();
        assert_eq!(
            result,
            Description::ExactArray {
                value: vec![
                    Description::new_base_type("string"),
                    Description::new_base_type("boolean")
                ]
            }
        );
    }

    #[test]
    fn test_literal_union() {
        let result = parse_and_evaluate_description_notation("\"hello\" | \"world\"").unwrap();
        assert_eq!(
            result,
            Description::Union {
                of: vec![
                    Description::StringValue {
                        value: "hello".to_owned()
                    },
                    Description::StringValue {
                        value: "world".to_owned()
                    }
                ]
            }
        );
    }

    #[test]
    fn test_complex_type() {
        let result = parse_and_evaluate_description_notation("string.uuid").unwrap();
        assert_eq!(result, Description::new_base_type("string.uuid"));
    }

    #[test]
    fn test_nullable_union() {
        let result = parse_and_evaluate_description_notation("boolean | null").unwrap();
        assert_eq!(
            result,
            Description::Union {
                of: vec![Description::new_base_type("boolean"), Description::Null]
            }
        );
    }
}
