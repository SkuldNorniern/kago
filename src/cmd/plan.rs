use std::path::Path;
use std::process::ExitCode;

use kago::install::{InstallPlan, Scope, Selection, choose, resolve};
use kago::manifest::Context;
use kago::package::Target;

use super::args::Args;
use super::check::targets;
use super::{load, manifest_path, report};

/// dry run: resolves and builds the install plan, prints it, changes
/// nothing.
pub fn run(args: &Args) -> Result<ExitCode, String> {
    let target = match targets(args)?.as_slice() {
        [one] => *one,
        _ => return Err("`plan` takes one `--target`".to_owned()),
    };
    let Some(plan) = build(args, target)? else {
        return Ok(ExitCode::FAILURE);
    };
    print!("{plan}");
    println!("\nno changes made.");
    Ok(ExitCode::SUCCESS)
}

/// manifest argument, `--scope` and `--components` to an install plan for
/// `target`. `None` after diagnostics were printed.
pub fn build(args: &Args, target: Target) -> Result<Option<InstallPlan>, String> {
    args.at_most_one()?;
    let path = manifest_path(args.first());
    let Some(manifest) = load(&path) else {
        return Ok(None);
    };
    let requested: Option<Scope> = args.value("--scope").map(str::parse).transpose()?;
    let selection = match args.value("--components") {
        Some(list) => {
            let picked: Vec<&str> = list.split(',').filter(|s| !s.is_empty()).collect();
            if let Some(unknown) = picked.iter().find(|c| manifest.component(c).is_none()) {
                return Err(format!("unknown component `{unknown}`"));
            }
            Selection::with(&manifest, picked)
        }
        None => Selection::defaults(&manifest),
    };

    let (resolved, diagnostics) = resolve(&manifest, &Context::target(target), selection);
    report(&diagnostics);
    let Some(resolved) = resolved else {
        return Ok(None);
    };
    let scope = match choose(&manifest.install, &resolved, requested) {
        Ok(scope) => scope,
        Err(d) => {
            eprintln!("{d}\n");
            return Ok(None);
        }
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let (plan, diagnostics) = InstallPlan::build(&manifest, &resolved, target, scope, base);
    report(&diagnostics);
    Ok(plan)
}
