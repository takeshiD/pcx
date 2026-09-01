//! Command-line entrypoint and process-level behavior.

use std::{ffi::OsString, process::ExitCode};

use clap::{CommandFactory, Parser};

/// Inspect and reduce point-cloud recordings where the data lives.
#[derive(Debug, Parser)]
#[command(
    name = "pcx",
    version,
    about = "Inspect and reduce point-cloud recordings on edge Linux systems",
    long_about = "Inspect and reduce point-cloud recordings on edge Linux systems.\n\npcx is under active development. The current foundation release exposes only help and version information; point-cloud commands are planned for v0.1."
)]
pub struct Cli {}

/// Parse command-line arguments without terminating the process.
pub fn try_run_from<I, T>(args: I) -> Result<(), clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args).map(|_| ())
}

/// Run the process entrypoint using the environment arguments.
pub fn main() -> ExitCode {
    match try_run_from(std::env::args_os()) {
        Ok(()) => {
            let mut command = Cli::command();
            if command.print_help().is_err() {
                return ExitCode::FAILURE;
            }
            println!();
            ExitCode::SUCCESS
        }
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::try_run_from;

    #[test]
    fn accepts_the_program_name_without_product_commands() {
        assert!(try_run_from(["pcx"]).is_ok());
    }

    #[test]
    fn rejects_unknown_arguments() {
        assert!(try_run_from(["pcx", "--unknown"]).is_err());
    }
}
