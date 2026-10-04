mod args;
mod check;
mod lifecycle;
mod plan;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::Args;
use kago::diagnostic::Diagnostics;
use kago::manifest::{self, Manifest};

const USAGE: &str = "\
usage: kago <command>

commands:
  check [manifest] [--target <target>]...
                validate kago.toml and resolve it for each target
                (default: the host)
  plan [manifest] [--target <target>] [--scope user|machine]
       [--components <a,b>]
                print what an install would do, without doing it
  install [manifest] [--scope user|machine] [--components <a,b>]
                install the manifest on this machine
  uninstall <package-id> [--scope user|machine]
                remove everything the install owns
  status [--scope user|machine]
                list installed packages
  recover [--scope user|machine]
                finish or roll back installs a crash left behind

manifest is a kago.toml or the directory holding one (default: .)
targets look like windows-x86_64, macos-aarch64, linux-x86_64-musl
";

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = if let Some((c, rest)) = args.split_first() {
        (c.as_str(), rest)
    } else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let result = match command {
        "check" => Args::parse(rest, &["--target"]).and_then(|a| check::run(&a)),
        "plan" => {
            Args::parse(rest, &["--target", "--scope", "--components"]).and_then(|a| plan::run(&a))
        }
        "install" => {
            Args::parse(rest, &["--scope", "--components"]).and_then(|a| lifecycle::install(&a))
        }
        "uninstall" => Args::parse(rest, &["--scope"]).and_then(|a| lifecycle::uninstall(&a)),
        "status" => Args::parse(rest, &["--scope"]).and_then(|a| lifecycle::status(&a)),
        "recover" => Args::parse(rest, &["--scope"]).and_then(|a| lifecycle::recover(&a)),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => Err(format!("unknown command `{command}`\n\n{USAGE}")),
    };
    match result {
        Ok(code) => code,
        Err(message) => {
            eprintln!("kago: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `kago.toml` path from an optional file-or-directory argument.
fn manifest_path(arg: Option<&str>) -> PathBuf {
    let path = Path::new(arg.unwrap_or("."));
    if path.is_dir() {
        path.join(manifest::FILE_NAME)
    } else {
        path.to_path_buf()
    }
}

/// prints diagnostics to stderr, blank line between each.
fn report(diagnostics: &Diagnostics) {
    for d in diagnostics {
        eprintln!("{d}\n");
    }
}

/// loads and reports. `None` when the manifest has errors.
fn load(path: &Path) -> Option<Manifest> {
    let (manifest, diagnostics) = Manifest::load(path);
    report(&diagnostics);
    manifest
}
