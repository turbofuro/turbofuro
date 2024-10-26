use std::fs;

use image::codecs::{bmp::BmpEncoder, jpeg::JpegEncoder, png::PngEncoder, webp::WebPEncoder};
use serde::{Deserialize, Serialize};
use tel::{describe, Description, StorageValue};
use tokio::task;
use tracing::instrument;

use crate::{
    errors::ExecutionError,
    evaluations::eval_param,
    executor::{ExecutionContext, Parameter},
};

use super::as_string;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterType {
    Nearest,
    Triangle,
    CatmullRom,
    Gaussian,
    Lanczos3,
}

impl From<FilterType> for image::imageops::FilterType {
    fn from(value: FilterType) -> Self {
        match value {
            FilterType::Nearest => image::imageops::FilterType::Nearest,
            FilterType::Triangle => image::imageops::FilterType::Triangle,
            FilterType::CatmullRom => image::imageops::FilterType::CatmullRom,
            FilterType::Gaussian => image::imageops::FilterType::Gaussian,
            FilterType::Lanczos3 => image::imageops::FilterType::Lanczos3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
enum Action {
    Resize {
        width: u32,
        height: u32,
        filter: FilterType,
    },
    ResizeExact {
        width: u32,
        height: u32,
        filter: FilterType,
    },
    ResizeToFill {
        width: u32,
        height: u32,
        filter: FilterType,
    },
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Brighten {
        value: i32,
    },
    AdjustContrast {
        value: f32,
    },
    Blur {
        sigma: f32,
    },
    FastBlur {
        sigma: f32,
    },
    Grayscale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
enum Output {
    Jpeg { quality: Option<u8> },
    Png,
    Webp,
    Bmp,
}

fn process(
    source: &str,
    destination: &str,
    actions: Vec<Action>,
    output: Output,
) -> Result<(), ExecutionError> {
    let mut img = image::open(source).map_err(|e| ExecutionError::StateInvalid {
        message: "Could not open image".to_owned(),
        subject: "image".to_owned(),
        inner: e.to_string(),
    })?;
    for action in actions {
        match action {
            Action::Resize {
                width,
                height,
                filter,
            } => {
                img = img.resize(width, height, filter.into());
            }
            Action::Crop {
                x,
                y,
                width,
                height,
            } => {
                img = img.crop(x, y, width, height);
            }
            Action::Brighten { value } => {
                img = img.brighten(value);
            }
            Action::AdjustContrast { value } => {
                img = img.adjust_contrast(value);
            }
            Action::Blur { sigma } => {
                img = img.blur(sigma);
            }
            Action::Grayscale => {
                img = img.grayscale();
            }
            Action::ResizeExact {
                width,
                height,
                filter,
            } => {
                img = img.resize_exact(width, height, filter.into());
            }
            Action::ResizeToFill {
                width,
                height,
                filter,
            } => {
                img = img.resize_to_fill(width, height, filter.into());
            }
            Action::FastBlur { sigma } => {
                img = img.fast_blur(sigma);
            }
        }
    }

    let file = fs::File::create(destination).map_err(ExecutionError::from)?;
    match output {
        Output::Jpeg { quality } => {
            let mut writer = std::io::BufWriter::new(file);
            let encoder = JpegEncoder::new_with_quality(&mut writer, quality.unwrap_or(90));
            img.write_with_encoder(encoder)?;
        }
        Output::Png => {
            let mut writer = std::io::BufWriter::new(file);
            let encoder = PngEncoder::new_with_quality(
                &mut writer,
                image::codecs::png::CompressionType::default(),
                image::codecs::png::FilterType::default(),
            );
            img.write_with_encoder(encoder)?;
        }
        Output::Webp => {
            let mut writer = std::io::BufWriter::new(file);
            let encoder = WebPEncoder::new_lossless(&mut writer);
            img.write_with_encoder(encoder)?;
        }
        Output::Bmp => {
            let mut writer = std::io::BufWriter::new(file);
            let encoder = BmpEncoder::new(&mut writer);
            img.write_with_encoder(encoder)?;
        }
    }

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn convert(
    context: &mut ExecutionContext<'_>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let source = as_string(
        eval_param("source", parameters, &context.storage, &context.environment)?,
        "source",
    )?;

    let destination = as_string(
        eval_param(
            "destination",
            parameters,
            &context.storage,
            &context.environment,
        )?,
        "path",
    )?;

    let actions = match eval_param(
        "actions",
        parameters,
        &context.storage,
        &context.environment,
    )? {
        StorageValue::Array(arr) => {
            serde_json::from_value::<Vec<Action>>(serde_json::to_value(arr).unwrap()).map_err(|e| {
                ExecutionError::ParameterInvalid {
                    name: "actions".to_owned(),
                    message: e.to_string(),
                }
            })
        }
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "actions".to_owned(),
            expected: Description::new_base_type("array"),
            actual: describe(s),
        }),
    }?;

    let output = match eval_param("output", parameters, &context.storage, &context.environment)? {
        StorageValue::Object(obj) => {
            serde_json::from_value::<Output>(serde_json::to_value(obj).unwrap()).map_err(|e| {
                ExecutionError::ParameterInvalid {
                    name: "output".to_owned(),
                    message: e.to_string(),
                }
            })
        }
        s => Err(ExecutionError::ParameterTypeMismatch {
            name: "output".to_owned(),
            expected: Description::new_base_type("object"),
            actual: describe(s),
        }),
    }?;

    let result =
        task::spawn_blocking(move || process(&source, &destination, actions, output)).await;

    match result {
        Ok(_) => {}
        Err(e) => {
            return Err(ExecutionError::StateInvalid {
                message: "Could not spawn blocking imgage processing task".to_owned(),
                subject: "task".to_owned(),
                inner: e.to_string(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod test_image {
    use crate::executor::ExecutionTest;

    use super::*;

    #[tokio::test]
    async fn test_image() {
        let mut t = ExecutionTest::default();
        let mut context = t.get_context();

        convert(
            &mut context,
            &vec![
                Parameter::tel("source", "\"misc/test.jpg\""),
                Parameter::tel("destination", "\"misc/test2.png\""),
                Parameter::tel(
                    "actions",
                    "[{ type: \"resize\", width: 100, height: 100, filter: \"gaussian\" }]",
                ),
                Parameter::tel("output", "{ type: \"png\" }"),
            ],
            "test",
            None,
        )
        .await
        .unwrap();
    }
}
