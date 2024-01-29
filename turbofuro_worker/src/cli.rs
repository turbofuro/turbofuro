const HELP: &str = "\
Turbofuro Worker 🛫

USAGE:
  turbofuro_worker [OPTIONS]

FLAGS:
  -h, --help            Prints help information

OPTIONS:
  --config PATH         Runs worker with a configuration read from given PATH
  --token TOKEN         Runs worker with a given device TOKEN
  --port PORT           Runs HTTP endpoints on a given PORT
  --disable-agent       Disables cloud agent
  --cloud URL           Uses a given cloud URL instead of the default one (https://api.turbofuro.com)
";

#[derive(Debug)]
pub struct AppArgs {
    pub token: Option<String>,
    pub port: Option<u16>,
    pub config: Option<std::path::PathBuf>,
    pub disable_agent: bool,
    pub cloud_url: Option<String>,
}

pub fn parse_cli_args() -> Result<AppArgs, pico_args::Error> {
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
        disable_agent: pargs.contains("--disable-agent"),
        cloud_url: pargs.opt_value_from_str("--cloud")?,
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
