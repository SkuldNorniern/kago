mod cmd;

use std::process::ExitCode;

fn main() -> ExitCode {
    cmd::uninstaller::run()
        .or_else(cmd::setup::run_embedded)
        .unwrap_or_else(cmd::run)
}
