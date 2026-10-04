use std::process::ExitCode;

use kago::install::{Selection, resolve};
use kago::manifest::Context;
use kago::package::Target;

use super::args::Args;
use super::{load, manifest_path, report};

/// validates the manifest, then resolves it for each `--target` (or the
/// host) with default components.
pub fn run(args: &Args) -> Result<ExitCode, String> {
    args.at_most_one()?;
    let path = manifest_path(args.first());
    let Some(manifest) = load(&path) else {
        return Ok(ExitCode::FAILURE);
    };
    let targets = targets(args)?;
    let mut ok = true;
    for target in targets {
        let (resolved, diagnostics) = resolve(
            &manifest,
            &Context::target(target),
            Selection::defaults(&manifest),
        );
        report(&diagnostics);
        if let Some(r) = resolved {
            println!("{target}: ok, {} files", r.files.len());
        } else {
            println!("{target}: failed");
            ok = false;
        }
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `--target` values, or the host when none is given.
pub fn targets(args: &Args) -> Result<Vec<Target>, String> {
    let given: Vec<Target> = args
        .all("--target")
        .map(|t| t.parse().map_err(|e| format!("{e}")))
        .collect::<Result<_, _>>()?;
    if !given.is_empty() {
        return Ok(given);
    }
    Target::host()
        .map(|t| vec![t])
        .ok_or_else(|| "this machine is not a kago target, pass `--target`".to_owned())
}
