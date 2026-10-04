mod cmd;

use std::process::ExitCode;

fn main() -> ExitCode {
    cmd::setup::run_embedded().unwrap_or_else(cmd::run)
}
