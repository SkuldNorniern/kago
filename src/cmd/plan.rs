use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::install::{InstallPlan, Scope, Selection, choose, resolve};
use kago::manifest::{Context, Manifest};
use kago::package::Target;
use kago::payload::Payload;

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
    let Some((_, plan)) = build(args, target, false)? else {
        return Ok(ExitCode::FAILURE);
    };
    print!("{plan}");
    println!("\nno changes made.");
    Ok(ExitCode::SUCCESS)
}

/// manifest argument, `--scope` and `--components` to the manifest, its
/// directory and an install plan for `target`. for a setup, the `build`
/// lines run first unless `--no-build`, and without `--components` every
/// component is offered so the person can choose; otherwise the defaults.
/// `None` after diagnostics were printed.
pub fn build(
    args: &Args,
    target: Target,
    offer: bool,
) -> Result<Option<(Loaded, InstallPlan)>, String> {
    args.at_most_one()?;
    let path = manifest_path(args.first());
    let Some(manifest) = load(&path) else {
        return Ok(None);
    };
    if offer && !args.flag("--no-build") {
        let out = args.value("--out").unwrap_or("dist");
        run_build(
            &manifest,
            path.parent().unwrap_or(Path::new(".")),
            target,
            Path::new(out),
        )?;
    }
    let requested: Option<Scope> = args.value("--scope").map(str::parse).transpose()?;
    let selection = match args.value("--components") {
        Some(list) => {
            let picked: Vec<&str> = list.split(',').filter(|s| !s.is_empty()).collect();
            if let Some(unknown) = picked.iter().find(|c| manifest.component(c).is_none()) {
                return Err(format!("unknown component `{unknown}`"));
            }
            Selection::with(&manifest, picked)
        }
        None if offer => {
            Selection::with(&manifest, manifest.components.iter().map(|c| c.id.as_str()))
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
    let payload = Payload::new(&manifest, base, target);
    let (plan, diagnostics) = InstallPlan::build(&manifest, &resolved, target, scope, &payload);
    drop(payload);
    report(&diagnostics);
    Ok(plan.map(|plan| {
        (
            Loaded {
                manifest,
                base: base.to_path_buf(),
            },
            plan,
        )
    }))
}

/// the Kagofile's `build` steps for `target`, in its folder, each until
/// it ends. each gets the package's version, the target and where the
/// setup goes as `KAGO_*` variables.
fn run_build(manifest: &Manifest, base: &Path, target: Target, out: &Path) -> Result<(), String> {
    let base = if base.as_os_str().is_empty() {
        Path::new(".")
    } else {
        base
    };
    let cx = kago::manifest::Context::target(target);
    let out = std::path::absolute(out).unwrap_or_else(|_| out.to_path_buf());
    for step in manifest.build.iter().filter(|s| s.when.matches(&cx)) {
        let (program, mut arguments) = if step.script {
            interpreter(&step.command.program)
        } else {
            (step.command.program.clone(), Vec::new())
        };
        arguments.extend(step.command.arguments.iter().cloned());
        let line = std::iter::once(step.command.program.as_str())
            .chain(step.command.arguments.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("build: {line}");
        let status = std::process::Command::new(&program)
            .args(&arguments)
            .current_dir(base)
            .env("KAGO_PACKAGE", manifest.package.id.as_str())
            .env("KAGO_NAME", &manifest.package.name)
            .env("KAGO_VERSION", manifest.package.version.to_string())
            .env("KAGO_TARGET", target.to_string())
            .env("KAGO_OUT", &out)
            .status()
            .map_err(|e| format!("build: `{line}` cannot start: {e}"))?;
        if !status.success() {
            return Err(format!("build: `{line}` failed ({status})"));
        }
    }
    Ok(())
}

/// the program that runs a script, by its extension, and its arguments
/// before the script's own.
fn interpreter(script: &str) -> (String, Vec<String>) {
    let extension = Path::new(script)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let owned = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let (program, before): (&str, Vec<String>) = match extension.as_str() {
        "ps1" if cfg!(windows) => (
            "powershell",
            owned(&["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]),
        ),
        "ps1" => ("pwsh", owned(&["-NoProfile", "-File"])),
        "cmd" | "bat" => ("cmd", owned(&["/d", "/c"])),
        "sh" => ("sh", Vec::new()),
        "py" if cfg!(windows) => ("py", Vec::new()),
        "py" => ("python3", Vec::new()),
        _ => return (script.to_owned(), Vec::new()),
    };
    let mut arguments = before;
    arguments.push(script.to_owned());
    (program.to_owned(), arguments)
}

/// a checked manifest and the directory its paths are relative to.
pub struct Loaded {
    pub manifest: Manifest,
    pub base: PathBuf,
}
