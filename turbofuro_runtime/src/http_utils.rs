use std::{borrow::Cow, collections::HashMap};

use axum::{
    body::Bytes,
    extract::{FromRequest, FromRequestParts, Path, Query},
};
use encoding_rs::{Encoding, UTF_8};
use http::{header, HeaderMap, Method, Request};
use hyper::Body;
use tel::StorageValue;

fn retrieve_content_type(headers: &HeaderMap) -> DetectedContentType {
    let content_type = match headers.get(header::CONTENT_TYPE) {
        Some(v) => v,
        None => return DetectedContentType::None,
    };
    let content_type = match content_type.to_str() {
        Ok(content_type) => content_type,
        Err(_) => return DetectedContentType::None,
    };
    let mime = match content_type.parse::<mime::Mime>() {
        Ok(mime) => mime,
        Err(_) => return DetectedContentType::Unknown,
    };

    if mime.type_() == "application"
        && (mime.subtype() == "json" || mime.suffix().map_or(false, |name| name == "json"))
    {
        return DetectedContentType::Json;
    }

    if mime.type_() == "application"
        && (mime.subtype() == "x-www-form-urlencoded"
            || mime
                .suffix()
                .map_or(false, |name| name == "x-www-form-urlencoded"))
    {
        return DetectedContentType::Form;
    }

    if mime.type_() == "text" {
        return DetectedContentType::Text;
    }

    if mime.type_() == "application"
        && (mime.subtype() == "octet-stream"
            || mime.suffix().map_or(false, |name| name == "octet-stream"))
    {
        return DetectedContentType::Bytes;
    }

    DetectedContentType::Unknown
}

#[derive(Debug, PartialEq)]
enum DetectedContentType {
    Json,
    Form,
    Text,
    Bytes,
    None,
    Unknown,
}

pub async fn build_initial_storage_from_request(
    req: Request<Body>,
) -> HashMap<String, StorageValue> {
    let mut request_obj = HashMap::new();

    let (mut parts, body) = req.into_parts();
    let content_type: DetectedContentType;

    {
        let path: Path<HashMap<String, String>> =
            Path::from_request_parts(&mut parts, &()).await.unwrap();
        let query: Query<HashMap<String, String>> =
            Query::from_request_parts(&mut parts, &()).await.unwrap();
        let method: Method = Method::from_request_parts(&mut parts, &()).await.unwrap();

        request_obj.insert(
            "method".to_string(),
            StorageValue::String(method.to_string()),
        );
        request_obj.insert(
            "query".to_string(),
            StorageValue::Object(
                query
                    .0
                    .into_iter()
                    .map(|(k, v)| (k, StorageValue::String(v)))
                    .collect(),
            ),
        );
        request_obj.insert(
            "params".to_string(),
            StorageValue::Object(
                path.0
                    .into_iter()
                    .map(|(k, v)| (k, StorageValue::String(v)))
                    .collect(),
            ),
        );

        request_obj.insert(
            "path".to_string(),
            StorageValue::String(parts.uri.path().to_string()),
        );

        // Insert headers
        let mut headers = HashMap::new();
        for (k, v) in &parts.headers {
            let value = match v.to_str() {
                Ok(v) => v,
                Err(_) => continue,
            };

            headers.insert(
                k.as_str().to_string(),
                StorageValue::String(value.to_string()),
            );
        }
        request_obj.insert("headers".to_string(), StorageValue::Object(headers));
        content_type = retrieve_content_type(&parts.headers);
    }

    let request = Request::from_parts(parts, body);

    // TODO: Add support for not parsing the body
    let bytes = Bytes::from_request(request, &()).await.ok();
    if let Some(bytes) = bytes {
        match content_type {
            DetectedContentType::Json => {
                let body = serde_json::from_slice(&bytes).ok();
                if let Some(body) = body {
                    request_obj.insert("body".to_string(), body);
                }
            }
            DetectedContentType::Form => {
                let body = serde_urlencoded::from_bytes(&bytes).ok();
                if let Some(body) = body {
                    request_obj.insert("form".to_string(), body);
                }
            }
            DetectedContentType::Text => {
                let (text, _) = decode_text_with_encoding("utf-8", &bytes);
                request_obj.insert("body".to_string(), StorageValue::String(text));
            }
            DetectedContentType::Bytes => {
                let vec = bytes
                    .to_vec()
                    .iter_mut()
                    .map(|f| StorageValue::Number(*f as f64))
                    .collect();
                request_obj.insert("body".to_string(), StorageValue::Array(vec));
            }
            DetectedContentType::None => {}
            DetectedContentType::Unknown => {}
        }
    }

    let mut storage = HashMap::new();
    storage.insert("request".to_string(), StorageValue::Object(request_obj));
    storage
}

pub fn decode_text_with_encoding(encoding_name: &str, full: &Bytes) -> (String, bool) {
    let encoding = Encoding::for_label(encoding_name.as_bytes()).unwrap_or(UTF_8);
    let (text, _, replaced) = encoding.decode(full);
    if let Cow::Owned(s) = text {
        return (s, replaced);
    }
    unsafe {
        // decoding returned Cow::Borrowed, meaning these bytes
        // are already valid utf8
        (String::from_utf8_unchecked(full.to_vec()), replaced)
    }
}

#[cfg(test)]
mod test_http_utils {
    use super::*;

    #[test]
    fn test_decode_text_with_encoding_utf8() {
        let encoding_name = "utf-8";
        let bytes = Bytes::from("Hello, world!".as_bytes());

        let (text, replaced) = decode_text_with_encoding(encoding_name, &bytes);

        assert_eq!(text, "Hello, world!");
        assert!(!replaced);
    }

    #[test]
    fn test_decode_text_with_encoding_latin1() {
        let encoding_name = "latin1";
        let bytes = Bytes::from(vec![
            72, 101, 108, 108, 111, 44, 32, 119, 111, 114, 108, 100, 33, // Hello, world!
        ]);

        let (text, replaced) = decode_text_with_encoding(encoding_name, &bytes);

        assert_eq!(text, "Hello, world!");
        assert!(!replaced);
    }

    #[test]
    fn test_decode_text_with_utf8_emojis() {
        let encoding_name = "utf-8";
        let bytes = Bytes::from(vec![
            0xF0, 0x9F, 0x92, 0x96, 0xF0, 0x9F, 0x92, 0x96, 0xF0, 0x9F, 0x92, 0x96,
        ]);

        let (text, replaced) = decode_text_with_encoding(encoding_name, &bytes);

        assert_eq!(text, "💖💖💖");
        assert!(!replaced);
    }

    #[test]
    fn test_decode_text_with_incorrect_utf8() {
        let encoding_name = "utf-8";
        let bytes = Bytes::from(vec![
            0xF0, 0x9F, 0x92, 0x96, 0xF0, 0x9F, 0x92, 0x96, 0xF0, 0x9F, 0x92, 0x96, 0x80,
        ]);

        let (text, replaced) = decode_text_with_encoding(encoding_name, &bytes);

        assert_eq!(text, "💖💖💖�");
        assert!(replaced);
    }

    #[test]
    fn test_detects_textplain_content_type() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            "text/plain; charset=utf-8".parse().unwrap(),
        );

        let content_type = retrieve_content_type(&headers);
        assert_eq!(content_type, DetectedContentType::Text);
    }

    #[test]
    fn test_retrieve_content_type_json() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());

        let content_type = retrieve_content_type(&headers);
        assert_eq!(content_type, DetectedContentType::Json);
    }

    #[test]
    fn test_retrieve_content_type_form() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );

        let content_type = retrieve_content_type(&headers);
        assert_eq!(content_type, DetectedContentType::Form);
    }

    #[test]
    fn test_retrieve_content_type_text() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            "text/plain; charset=utf-8".parse().unwrap(),
        );

        let content_type = retrieve_content_type(&headers);
        assert_eq!(content_type, DetectedContentType::Text);
    }

    #[test]
    fn test_retrieve_content_type_bytes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            "application/octet-stream".parse().unwrap(),
        );

        let content_type = retrieve_content_type(&headers);
        assert_eq!(content_type, DetectedContentType::Bytes);
    }

    #[test]
    fn test_retrieve_content_type_none() {
        let headers = HeaderMap::new();

        let content_type = retrieve_content_type(&headers);
        assert_eq!(content_type, DetectedContentType::None);
    }

    #[test]
    fn test_retrieve_content_type_unknown() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/custom".parse().unwrap());

        let content_type = retrieve_content_type(&headers);
        assert_eq!(content_type, DetectedContentType::Unknown);
    }
}
