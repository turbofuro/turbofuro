use std::{borrow::Cow, collections::HashMap, fmt::Display, vec};

use axum::{
    body::{Body, Bytes},
    extract::{FromRequest, FromRequestParts, Path, Query, Request},
    http::request::Parts,
    RequestExt,
};
use axum_extra::extract::CookieJar;
use encoding_rs::{Encoding, UTF_8};
use futures_util::StreamExt;
use hyper::{header, HeaderMap, Method};
use tel::{ObjectBody, StorageValue};

use crate::{
    errors::ExecutionError,
    modules::http_server::form_data::{
        FormDataReaderCommand, FormDataReaderEvent, PendingFormData,
    },
    resources::ActorResources,
};

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
        && (mime.subtype() == "json" || mime.suffix().is_some_and(|name| name == "json"))
    {
        return DetectedContentType::Json;
    }

    if mime.type_() == "application"
        && (mime.subtype() == "x-www-form-urlencoded"
            || mime
                .suffix()
                .is_some_and(|name| name == "x-www-form-urlencoded"))
    {
        return DetectedContentType::Form;
    }

    if mime.type_() == "multipart" {
        return DetectedContentType::Multipart {
            boundary: mime.get_param("boundary").map(|s| s.to_string()),
        };
    }

    if mime.type_() == "text" {
        return DetectedContentType::Text;
    }

    if mime.type_() == "application"
        && (mime.subtype() == "octet-stream"
            || mime.suffix().is_some_and(|name| name == "octet-stream"))
    {
        return DetectedContentType::Bytes;
    }

    DetectedContentType::Unknown
}

#[derive(Debug, PartialEq)]
pub enum DetectedContentType {
    Json,
    Form,
    Text,
    Bytes,
    None,
    Unknown,
    Multipart { boundary: Option<String> },
}

impl Display for DetectedContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = match self {
            DetectedContentType::Json => "json".to_string(),
            DetectedContentType::Form => "form".to_string(),
            DetectedContentType::Text => "text".to_string(),
            DetectedContentType::Bytes => "bytes".to_string(),
            DetectedContentType::None => "none".to_string(),
            DetectedContentType::Unknown => "unknown".to_string(),
            DetectedContentType::Multipart { boundary } => {
                format!(
                    "multipart/{}",
                    boundary.clone().unwrap_or_else(|| "".to_owned())
                )
            }
        };

        write!(f, "{string}")
    }
}

pub async fn build_metadata_from_parts(parts: &mut Parts) -> (ObjectBody, DetectedContentType) {
    let mut obj: HashMap<String, StorageValue> = HashMap::new();
    obj.insert("version".to_owned(), format!("{:?}", parts.version).into());
    let content_type: DetectedContentType;
    {
        let path: Option<Path<HashMap<String, String>>> =
            Path::from_request_parts(parts, &()).await.ok();
        let query: Option<Query<HashMap<String, String>>> =
            Query::from_request_parts(parts, &()).await.ok();
        let method: Method = Method::from_request_parts(parts, &()).await.unwrap();

        obj.insert(
            "method".to_string(),
            StorageValue::String(method.to_string()),
        );

        if let Some(query) = query {
            obj.insert(
                "query".to_string(),
                StorageValue::Object(
                    query
                        .0
                        .into_iter()
                        .map(|(k, v)| (k, StorageValue::String(v)))
                        .collect(),
                ),
            );
        }
        if let Some(path) = path {
            obj.insert(
                "params".to_string(),
                StorageValue::Object(
                    path.0
                        .into_iter()
                        .map(|(k, v)| (k, StorageValue::String(v)))
                        .collect(),
                ),
            );
        }
        obj.insert(
            "path".to_string(),
            StorageValue::String(parts.uri.path().to_string()),
        );

        let cookies = CookieJar::from_request_parts(parts, &()).await.ok();
        let mut cookies_object = ObjectBody::new();
        if let Some(cookies) = cookies {
            for cookie in cookies.iter() {
                let mut cookie_object: ObjectBody = HashMap::new();
                cookie_object.insert("value".to_string(), cookie.value().into());
                if let Some(expires) = cookie.expires() {
                    match expires {
                        axum_extra::extract::cookie::Expiration::DateTime(datetime) => {
                            cookie_object.insert(
                                "expires".to_string(),
                                (datetime.unix_timestamp() as f64).into(),
                            );
                        }
                        axum_extra::extract::cookie::Expiration::Session => {
                            cookie_object.insert("expires".to_string(), "session".into());
                        }
                    }
                }
                if let Some(max_age) = cookie.max_age() {
                    cookie_object.insert("maxAge".to_string(), max_age.as_seconds_f64().into());
                }
                if let Some(secure) = cookie.secure() {
                    cookie_object.insert("secure".to_string(), secure.into());
                }
                if let Some(http_only) = cookie.http_only() {
                    cookie_object.insert("httpOnly".to_string(), http_only.into());
                }
                if let Some(same_site) = cookie.same_site() {
                    cookie_object.insert(
                        "sameSite".to_string(),
                        StorageValue::String(same_site.to_string()),
                    );
                }
                cookies_object.insert(
                    cookie.name().to_string(),
                    StorageValue::Object(cookie_object),
                );
            }
        }
        obj.insert("cookies".to_string(), StorageValue::Object(cookies_object));

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
        obj.insert("headers".to_string(), StorageValue::Object(headers));
        content_type = retrieve_content_type(&parts.headers);
    }
    (obj, content_type)
}

pub async fn build_request_object(
    request: Request<Body>,
) -> (HashMap<String, StorageValue>, ActorResources) {
    let mut resources = ActorResources::default();
    let (mut parts, body) = request.into_parts();
    let (mut request_object, content_type) = build_metadata_from_parts(&mut parts).await;

    let request = Request::from_parts(parts, body);
    match content_type {
        DetectedContentType::Json => {
            let bytes = Bytes::from_request(request, &()).await.ok();
            if let Some(bytes) = bytes {
                let body = serde_json::from_slice(&bytes).ok();
                if let Some(body) = body {
                    request_object.insert("body".to_string(), body);
                }
            }
        }
        DetectedContentType::Form => {
            let bytes = Bytes::from_request(request, &()).await.ok();
            if let Some(bytes) = bytes {
                let body: Option<Vec<(String, StorageValue)>> =
                    serde_html_form::from_bytes(&bytes).ok();
                if let Some(body) = body {
                    let mut form: HashMap<String, Vec<StorageValue>> = HashMap::new();
                    for (key, value) in body {
                        let removed = form.remove(&key);
                        if let Some(mut removed) = removed {
                            removed.push(value);
                            form.insert(key, removed);
                        } else {
                            form.insert(key, vec![value]);
                        }
                    }

                    request_object.insert(
                        "form".to_string(),
                        StorageValue::Object(
                            form.into_iter()
                                .map(|(k, v)| {
                                    if v.len() == 1 {
                                        (k, v[0].clone())
                                    } else {
                                        (k, StorageValue::Array(v))
                                    }
                                })
                                .collect(),
                        ),
                    );
                }
            }
        }
        DetectedContentType::Text => {
            let bytes = Bytes::from_request(request, &()).await.ok();
            if let Some(bytes) = bytes {
                let (text, _) = decode_text_with_encoding("utf-8", &bytes);
                request_object.insert("body".to_string(), StorageValue::String(text));
            }
        }
        DetectedContentType::Bytes => {
            let bytes = Bytes::from_request(request, &()).await.ok();
            if let Some(bytes) = bytes {
                let vec = bytes
                    .to_vec()
                    .iter_mut()
                    .map(|f| StorageValue::Number(*f as f64))
                    .collect();
                request_object.insert("body".to_string(), StorageValue::Array(vec));
            }
        }
        DetectedContentType::None => {}
        DetectedContentType::Unknown => {}
        DetectedContentType::Multipart { boundary } => {
            let stream = request.with_limited_body().into_body();
            if let Some(boundary) = boundary {
                let mut multipart = multer::Multipart::new(stream.into_data_stream(), boundary);
                request_object.insert("multipart".to_string(), StorageValue::Boolean(true));

                let (sender, mut receiver) = tokio::sync::mpsc::channel::<FormDataReaderCommand>(4);
                resources.add_pending_form_data(PendingFormData::new(sender));
                tokio::spawn(async move {
                    while let Some(command) = receiver.recv().await {
                        match command {
                            FormDataReaderCommand::GetNext {
                                sender: field_sender,
                            } => {
                                let field = multipart.next_field().await;

                                match field {
                                    Ok(field) => match field {
                                        Some(mut field) => {
                                            let (sender, receiver) =
                                                tokio::sync::mpsc::channel::<
                                                    Result<Bytes, ExecutionError>,
                                                >(4);

                                            // Insert headers
                                            let mut headers: HashMap<String, String> =
                                                HashMap::new();
                                            for (k, v) in field.headers() {
                                                let value = match v.to_str() {
                                                    Ok(v) => v,
                                                    Err(_) => continue,
                                                };
                                                headers.insert(
                                                    k.as_str().to_string(),
                                                    value.to_string(),
                                                );
                                            }

                                            match field_sender.send(FormDataReaderEvent::File {
                                                name: field.name().map(|s| s.to_owned()),
                                                filename: field.file_name().map(|s| s.to_owned()),
                                                receiver,
                                                headers,
                                                index: field.index(),
                                            }) {
                                                Ok(_) => {}
                                                Err(_) => {
                                                    break;
                                                }
                                            }

                                            while let Some(chunk) = field.next().await {
                                                match sender
                                                    .send(chunk.map_err(|e| {
                                                        ExecutionError::IoError {
                                                            message: e.to_string(),
                                                            os_code: None,
                                                        }
                                                    }))
                                                    .await
                                                {
                                                    Ok(_) => {}
                                                    Err(_) => {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        None => {
                                            match field_sender.send(FormDataReaderEvent::Empty) {
                                                Ok(_) => {}
                                                Err(_) => break,
                                            }
                                        }
                                    },
                                    Err(_e) => {
                                        match field_sender.send(FormDataReaderEvent::Error) {
                                            Ok(_) => {}
                                            Err(_) => {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                });
            } else {
                let mut body = ObjectBody::new();
                body.insert("error".to_owned(), "Missing boundary".into());
                request_object.insert("multipart".to_string(), StorageValue::Object(body));
            }
        }
    }
    (request_object, resources)
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
