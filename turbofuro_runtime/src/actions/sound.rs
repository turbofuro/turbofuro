use std::io::BufReader;
use tokio::task::spawn_blocking;
use tracing::instrument;

use crate::evaluations::{eval_opt_number_param, eval_string_param};
use crate::{
    errors::ExecutionError,
    executor::{ExecutionContext, Parameter},
};

fn inner_play_sound(path: String, volume: f32, speed: f32) -> Result<(), ExecutionError> {
    let (_stream, handle) =
        rodio::OutputStream::try_default().map_err(|e| ExecutionError::StateInvalid {
            message: e.to_string(),
            subject: "output_stream".to_string(),
            inner: "rodio".to_string(),
        })?;

    let sink = rodio::Sink::try_new(&handle).map_err(|e| ExecutionError::StateInvalid {
        message: e.to_string(),
        subject: "sink".to_string(),
        inner: "rodio".to_string(),
    })?;

    sink.set_volume(volume);
    sink.set_speed(speed);

    let file = std::fs::File::open(path).map_err(|e| ExecutionError::StateInvalid {
        message: e.to_string(),
        subject: "file".to_string(),
        inner: "rodio".to_string(),
    })?;

    let decoder =
        rodio::Decoder::new(BufReader::new(file)).map_err(|e| ExecutionError::StateInvalid {
            message: e.to_string(),
            subject: "decoder".to_string(),
            inner: "rodio".to_string(),
        })?;

    sink.append(decoder);
    sink.sleep_until_end();

    Ok(())
}

#[instrument(level = "trace", skip_all)]
pub async fn play_sound<'a>(
    context: &mut ExecutionContext<'a>,
    parameters: &Vec<Parameter>,
    _step_id: &str,
    _store_as: Option<&str>,
) -> Result<(), ExecutionError> {
    let path = eval_string_param("path", parameters, context)?;
    let volume = eval_opt_number_param("volume", parameters, context)?;
    let speed = eval_opt_number_param("speed", parameters, context)?;

    let volume = volume.map(|v| v as f32).unwrap_or(1.0);
    let speed = speed.map(|v| v as f32).unwrap_or(1.0);

    spawn_blocking(move || inner_play_sound(path.clone(), volume, speed))
        .await
        .map_err(|e| ExecutionError::StateInvalid {
            message: e.to_string(),
            subject: "play_sound".to_string(),
            inner: "tokio".to_string(),
        })?
}
