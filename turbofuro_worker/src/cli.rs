use crate::errors::WorkerError;

const HELP: &str = "\
Turbofuro Worker 🛫

USAGE:
  turbofuro_worker [OPTIONS]

FLAGS:
  -h, --help            Prints this help message

OPTIONS:
  --config PATH         Runs worker in a offline mode with a configuration read from given PATH
  --token TOKEN         Runs worker with a given machine TOKEN
  --port PORT           Runs HTTP server on a given PORT
  --cloud URL           Uses a given URL as cloud API instead of the default one (https://api.turbofuro.com)
  --operator URL        Uses a given URL as operator (debugger, stats) instead of the default one (https://operator.turbofuro.com)
  --name NAME           Sets a name of the worker

Alternatively, you can use the following environment variables:
TURBOFURO_TOKEN, PORT, TURBOFURO_CLOUD_URL, TURBOFURO_OPERATOR_URL, NAME
";

#[derive(Debug, Clone)]
pub struct AppArgs {
    pub token: Option<String>,
    pub port: Option<u16>,
    pub config: Option<std::path::PathBuf>,
    pub cloud_url: Option<String>,
    pub operator_url: Option<String>,
    pub name: Option<String>,
}

pub fn parse_cli_args() -> Result<AppArgs, WorkerError> {
    let mut pargs = pico_args::Arguments::from_env();

    // Help has a higher priority and should be handled separately.
    if pargs.contains(["-h", "--help"]) {
        print!("{}", HELP);
        std::process::exit(0);
    }

    let args = AppArgs {
        token: pargs.opt_value_from_str("--token")?,
        config: pargs.opt_value_from_os_str("--config", parse_path)?,
        port: pargs.opt_value_from_str("--port")?,
        cloud_url: pargs.opt_value_from_str("--cloud")?,
        operator_url: pargs.opt_value_from_str("--operator")?,
        name: pargs.opt_value_from_str("--name")?,
    };

    // It's up to the caller what to do with the remaining arguments.
    let remaining = pargs.finish();
    if !remaining.is_empty() {
        eprintln!("Warning: unused arguments left: {:?}.", remaining);
    }

    Ok(args)
}

fn parse_path(s: &std::ffi::OsStr) -> Result<std::path::PathBuf, &'static str> {
    Ok(s.into())
}
