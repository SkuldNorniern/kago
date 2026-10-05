mod args;
mod build;
mod check;
mod init;
mod lifecycle;
mod plan;
pub mod setup;
pub mod uninstaller;
mod update;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::Args;
use kago::diagnostic::Diagnostics;
use kago::manifest::{self, Manifest};

const USAGE: &str = "\
usage: kago <command>

commands:
  init [dir] [--id <package-id>] [--force]
                write a starter Kagofile, filled in from Cargo.toml
  check [manifest] [--target <target>]...
                validate the Kagofile and resolve it for each target
                (default: the host)
  plan [manifest] [--target <target>] [--scope user|machine]
       [--components <a,b>]
                print what an install would do, without doing it
  build [manifest] [--target <target>] [--scope user|machine]
        [--components <a,b>] [--out <dir>] [--no-sign] [--no-build]
                run the Kagofile's `build` lines, then build the setup
                executable (default out: dist), signed when the Kagofile
                has a `sign` block
  install [manifest] [--scope user|machine] [--components <a,b>] [--location <dir>]
                install the manifest on this machine
  uninstall <package-id> [--scope user|machine]
                remove everything the install owns
  verify <package-id> [--scope user|machine]
                check installed files against the receipt
  status [--scope user|machine]
                list installed packages
  recover [--scope user|machine]
                finish or roll back installs a crash left behind
  update [<package-id>] [--check] [--silent]
                install newer versions from where the Kagofile's `update`
                line points; --check only says what is newer
  version       print kago's version

manifest is a Kagofile or the directory holding one (default: .)
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
        "init" => Args::parse_with(rest, &["--id"], &["--force"]).and_then(|a| init::run(&a)),
        "check" => Args::parse(rest, &["--target"]).and_then(|a| check::run(&a)),
        "plan" => {
            Args::parse(rest, &["--target", "--scope", "--components"]).and_then(|a| plan::run(&a))
        }
        "build" => Args::parse_with(
            rest,
            &["--target", "--scope", "--components", "--out"],
            &["--no-sign", "--no-build"],
        )
        .and_then(|a| build::run(&a)),
        "install" => Args::parse(rest, &["--scope", "--components", "--location"])
            .and_then(|a| lifecycle::install(&a)),
        "uninstall" => Args::parse(rest, &["--scope"]).and_then(|a| lifecycle::uninstall(&a)),
        "verify" => Args::parse(rest, &["--scope"]).and_then(|a| lifecycle::verify(&a)),
        "status" => Args::parse(rest, &["--scope"]).and_then(|a| lifecycle::status(&a)),
        "recover" => Args::parse(rest, &["--scope"]).and_then(|a| lifecycle::recover(&a)),
        "update" => {
            Args::parse_with(rest, &[], &["--check", "--silent"]).and_then(|a| update::run(&a))
        }
        "version" | "--version" | "-V" => {
            println!("kago {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
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

/// `Kagofile` path from an optional file-or-directory argument.
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
