//! setup mode: this executable carries a package and was started to install
//! it. a window by default; `--silent` (or NSIS's `/S`) for no window and no
//! prompt, with `--components a,b` and `--location <dir>` for rollouts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::container::Container;
use kago::format::setup::{self, Embedded};
use kago::install::Operation;
use kago::manifest::SemanticPath;
use kago::platform;
use kago::transaction;

const SETUP_USAGE: &str = "\
usage: <setup>.exe [--silent | /S] [--components <a,b>] [--location <dir>]

  (nothing)           open the setup window
  --silent, /S        install without a window, with the defaults
  --components <a,b>  with --silent: these optional components, besides the
                      required ones; an empty list installs only those
  --location <dir>    with --silent: install into this folder

exit code 0 when installed, 1 otherwise; errors go to stderr.
";

/// what the setup's own command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct SetupArgs {
    silent: bool,
    components: Option<Vec<String>>,
    location: Option<PathBuf>,
    help: bool,
}

/// the setup's arguments, or `None` when they are not setup arguments, so
/// the executable runs as plain kago (the uninstall entry relies on that).
fn setup_args(args: &[String]) -> Option<Result<SetupArgs, String>> {
    let mut out = SetupArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--silent" | "/S" | "/s" => out.silent = true,
            "--help" | "-h" | "/?" => out.help = true,
            "--components" => match it.next() {
                Some(list) => {
                    out.components = Some(
                        list.split(',')
                            .map(str::trim)
                            .filter(|c| !c.is_empty())
                            .map(str::to_owned)
                            .collect(),
                    );
                }
                None => return Some(Err("`--components` needs a list like a,b".to_owned())),
            },
            "--location" => match it.next() {
                Some(dir) => out.location = Some(PathBuf::from(dir)),
                None => return Some(Err("`--location` needs a folder".to_owned())),
            },
            _ => return None,
        }
    }
    if !out.silent && (out.components.is_some() || out.location.is_some()) {
        return Some(Err(
            "`--components` and `--location` go with `--silent`; the window asks for them"
                .to_owned(),
        ));
    }
    Some(Ok(out))
}

/// installs the embedded package when there is one and the arguments are
/// setup arguments. `None` means run as plain kago.
pub fn run_embedded() -> Option<ExitCode> {
    let exe = std::env::current_exe().ok()?;
    let embedded = setup::embedded(&exe).ok()??;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match setup_args(&args)? {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("setup: {e}\n\n{SETUP_USAGE}");
            return Some(ExitCode::FAILURE);
        }
    };
    if parsed.help {
        print!("{SETUP_USAGE}");
        return Some(ExitCode::SUCCESS);
    }
    // a package for everyone goes to Program Files: ask for administrator
    // rights once, by starting this setup again with the UAC prompt.
    let for_everyone = Container::open(&exe, embedded.offset)
        .is_ok_and(|c| c.header.plan.scope == kago::install::Scope::Machine);
    if for_everyone && !platform::is_elevated() {
        return Some(match platform::run_elevated(&exe, &args, true) {
            Ok(code) => u8::try_from(code).map_or(ExitCode::FAILURE, ExitCode::from),
            Err(e) => {
                eprintln!("setup: {e}");
                ExitCode::FAILURE
            }
        });
    }
    if !parsed.silent {
        #[cfg(feature = "gui")]
        return Some(window(exe, embedded));
    }
    let location = match parsed.location.as_deref().map(std::path::absolute) {
        Some(Ok(dir)) => Some(dir),
        Some(Err(e)) => {
            eprintln!("setup failed: {e}");
            return Some(ExitCode::FAILURE);
        }
        None => None,
    };
    let choice = Choice {
        components: parsed.components.as_deref(),
        location: location.as_deref(),
    };
    Some(match install(&exe, embedded, &choice, &mut |_, _| {}) {
        Ok(done) => {
            println!("installed {}", done.name);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("setup failed: {e}");
            ExitCode::FAILURE
        }
    })
}

#[cfg(feature = "gui")]
fn window(exe: PathBuf, embedded: Embedded) -> ExitCode {
    use kago::gui::setup::{Existing, Folder, Info, Installed, License, Picked, run};
    use kago::install::check_folder;
    use kago::manifest::{FolderChoice, Root};

    let (info, look, folder) = match Container::open(&exe, embedded.offset) {
        Ok(mut c) => {
            let brand = window_brand(&mut c);
            let plan = &c.header.plan;
            let host = platform::host().ok();
            // where the installed version is, if there is one.
            let receipt = host
                .as_ref()
                .and_then(|p| transaction::store(p.as_ref(), plan.scope).ok())
                .and_then(|store| store.read_receipt(&plan.package).ok().flatten());
            let elsewhere = host
                .as_ref()
                .is_some_and(|p| other_install(p.as_ref(), plan.scope, &plan.package).is_some());
            let existing = receipt.as_ref().map_or(Existing::None, |r| {
                match r.version.cmp_precedence(&plan.version) {
                    std::cmp::Ordering::Equal => Existing::Same,
                    std::cmp::Ordering::Less => Existing::Older(r.version.to_string()),
                    std::cmp::Ordering::Greater => Existing::Newer {
                        version: r.version.to_string(),
                        allowed: plan.downgrade == kago::manifest::Downgrade::Allow,
                    },
                }
            });
            let existing = if existing == Existing::None && elsewhere {
                Existing::Elsewhere {
                    for_everyone: plan.scope == kago::install::Scope::User,
                }
            } else {
                existing
            };
            // where the installed version is, if there is one.
            let installed = receipt.and_then(|r| r.roots.get(&Root::App).cloned());
            let default = host
                .as_ref()
                .and_then(|p| p.roots(plan.scope, &plan.name).ok())
                .and_then(|r| r.0.get(&Root::App).cloned());
            let folder = (c.header.setup.folder == FolderChoice::Choose).then(|| {
                let installed = installed.clone();
                Folder {
                    browse: platform::pick_folder,
                    check: Box::new(move |dir: &Path| check_folder(dir, installed.as_deref())),
                }
            });
            let info = Info {
                name: plan.name.clone(),
                version: plan.version.to_string(),
                publisher: plan.publisher.clone(),
                existing,
                location: installed.or(default),
                brand,
                license: c.header.setup.license.as_ref().map(|l| License {
                    text: l.text.clone(),
                    accept: l.accept,
                }),
                components: window_components(&plan.choices),
            };
            (info, c.header.setup.look, folder)
        }
        Err(e) => {
            eprintln!("setup failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let work = move |reporter: &kago::gui::setup::Reporter, picked: Picked| {
        let mut report =
            |fraction, stage: Stage<'_>| reporter.report(fraction, window_step(&stage));
        let choice = Choice {
            components: Some(&picked.components),
            location: picked.location.as_deref(),
        };
        install(&exe, embedded, &choice, &mut report).map(|done| Installed {
            launch: done.launch,
        })
    };
    match run(look, &info, folder, work) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("cannot open the setup window: {e}");
            ExitCode::FAILURE
        }
    }
}

/// the setup's brand with its images decoded. an image that does not
/// decode is left out rather than stopping the setup.
#[cfg(feature = "gui")]
fn window_brand(c: &mut Container) -> kago::gui::setup::Brand {
    let mut image = |name: &str| {
        c.asset(name)
            .ok()
            .flatten()
            .and_then(|png| kago::format::image::decode_png(&png).ok())
    };
    let (logo, header, side) = (image("logo"), image("header"), image("side"));
    let brand = &c.header.setup.brand;
    kago::gui::setup::Brand {
        text: brand.text.clone(),
        accent: brand.accent,
        welcome: brand.welcome.clone(),
        finish: brand.finish.clone(),
        link: brand.link.clone(),
        logo,
        header,
        side,
    }
}

/// the plan's choices as tick boxes. one for something this machine lacks
/// starts unticked, unless it cannot be left out.
#[cfg(feature = "gui")]
fn window_components(choices: &[kago::install::Choice]) -> Vec<kago::gui::setup::Component> {
    choices
        .iter()
        .map(|choice| {
            let missing = choice
                .detect
                .filter(|d| !platform::detect(*d))
                .map(|d| d.name().to_owned());
            kago::gui::setup::Component {
                id: choice.id.clone(),
                name: choice.name.clone(),
                description: choice.description.clone(),
                default: choice.required || (choice.default && missing.is_none()),
                required: choice.required,
                depends: choice.depends.clone(),
                size: choice.size,
                missing,
            }
        })
        .collect()
}

/// a line for the details list.
#[cfg(feature = "gui")]
fn window_step(stage: &Stage<'_>) -> kago::gui::setup::Step {
    use kago::gui::setup::Step;
    match stage {
        Stage::Unpack(path) => Step::Unpack(path.to_string()),
        Stage::Apply(op) => match op {
            Operation::CreateDirectory { path } => Step::Folder(path.to_string()),
            Operation::InstallFile { target, .. } => Step::File(target.to_string()),
            Operation::AddPathEntry { directory, .. } => Step::Path(directory.to_string()),
            Operation::RegisterSteamVrDriver { directory } => Step::SteamVr(directory.to_string()),
            Operation::CreateShortcut { name, .. } => Step::Shortcut(name.clone()),
            Operation::RegisterApplication { display_name, .. } => {
                Step::Register(display_name.clone())
            }
            Operation::RunHook { command, .. } => Step::Hook(command.to_string()),
            Operation::InstallService { id, .. } => Step::Service(id.clone()),
        },
    }
}

struct Done {
    name: String,
    /// offered by the window as "Open".
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    launch: Option<PathBuf>,
}

/// what the install is at, for a progress display.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
enum Stage<'a> {
    Unpack(&'a SemanticPath),
    Apply(&'a Operation),
}

/// the same package installed in the other scope, with that scope's store.
fn other_install(
    platform: &dyn kago::platform::Platform,
    scope: kago::install::Scope,
    package: &kago::package::PackageId,
) -> Option<(kago::install::Scope, transaction::Store)> {
    use kago::install::Scope;
    let other = match scope {
        Scope::User => Scope::Machine,
        Scope::Machine => Scope::User,
    };
    let store = transaction::store(platform, other).ok()?;
    store
        .read_receipt(package)
        .ok()
        .flatten()
        .map(|_| (other, store))
}

/// what the person installing picked.
#[derive(Default)]
struct Choice<'a> {
    /// optional components to install, the defaults when `None`.
    components: Option<&'a [String]>,
    /// `{app}` when not the default or the installed version's folder.
    location: Option<&'a Path>,
}

/// installs the embedded package as `choice` says. `progress` hears each
/// step with how far along the whole install is, 0 to 1: unpacking each
/// file, then each operation.
fn install(
    exe: &Path,
    embedded: Embedded,
    choice: &Choice<'_>,
    progress: &mut dyn FnMut(f32, Stage<'_>),
) -> Result<Done, String> {
    let mut container = Container::open(exe, embedded.offset).map_err(|e| e.to_string())?;
    let header = &container.header.plan;
    let platform = platform::host()?;
    let store = transaction::store(platform.as_ref(), header.scope).map_err(|e| e.to_string())?;
    let staging = store.root().join("staging").join(header.package.as_str());
    let _ = fs::remove_dir_all(&staging);
    let result = (|| {
        // choose first: what is not installed is not unpacked either.
        let plan = &container.header.plan;
        if let Some(chosen) = choice.components
            && let Some(unknown) = chosen
                .iter()
                .find(|c| !plan.choices.iter().any(|choice| &choice.id == *c))
        {
            let known: Vec<&str> = plan.choices.iter().map(|c| c.id.as_str()).collect();
            return Err(format!(
                "unknown component `{unknown}`; this setup has {}",
                if known.is_empty() {
                    "none".to_owned()
                } else {
                    known.join(", ")
                }
            ));
        }
        if let Some(dir) = choice.location {
            let installed = store
                .read_receipt(&plan.package)
                .ok()
                .flatten()
                .and_then(|r| r.roots.get(&kago::manifest::Root::App).cloned());
            kago::install::check_folder(dir, installed.as_deref())
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let plan = match choice.components {
            Some(chosen) => plan.select(chosen.iter().map(String::as_str)),
            None => plan.with_defaults(),
        };
        let operations = plan.operations.len();
        #[allow(clippy::cast_precision_loss)]
        let share = |done: usize, total: usize| done as f32 / total.max(1) as f32;
        let mut files = 0;
        let plan = container
            .extract_plan(plan, &staging.join("payload"), &mut |i, n, target| {
                files = n;
                progress(share(i, n + operations), Stage::Unpack(target));
            })
            .map_err(|e| format!("cannot unpack: {e}"))?;
        let plan = match choice.location {
            Some(dir) => plan.at(dir),
            None => plan,
        };
        let runtime = staging.join(format!("kago{}", std::env::consts::EXE_SUFFIX));
        setup::extract_stub(exe, embedded, &runtime).map_err(|e| e.to_string())?;
        // the same package installed the other way (just for this user, or
        // for everyone) goes first, so there are never two. what ownership
        // keeps, like user data, stays.
        if let Some((scope, other)) = other_install(platform.as_ref(), plan.scope, &plan.package) {
            transaction::uninstall(&plan.package, platform.as_ref(), &other)
                .map_err(|e| format!("cannot remove the {scope} install first: {e}"))?;
        }
        let receipt = transaction::install_observed(
            &plan,
            platform.as_ref(),
            &store,
            &runtime,
            &mut |i, n, op| progress(share(files + i, files + n), Stage::Apply(op)),
        )
        .map_err(|e| e.to_string())?;
        let launch = plan.operations.iter().find_map(|op| match op {
            Operation::RegisterApplication {
                entrypoint: Some(e),
                ..
            } => receipt.absolute(e),
            _ => None,
        });
        Ok(Done {
            name: plan.name.clone(),
            launch,
        })
    })();
    let _ = fs::remove_dir_all(&staging);
    if let Some(parent) = staging.parent() {
        // only when no other setup is staging.
        let _ = fs::remove_dir(parent);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<Result<SetupArgs, String>> {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        setup_args(&args)
    }

    #[test]
    fn setup_arguments() {
        assert_eq!(parse(&[]), Some(Ok(SetupArgs::default())));
        let silent = parse(&[
            "/S",
            "--components",
            "steamvr, desktop",
            "--location",
            "D:/Apps/X",
        ])
        .unwrap()
        .unwrap();
        assert!(silent.silent);
        assert_eq!(silent.components.unwrap(), ["steamvr", "desktop"]);
        assert_eq!(silent.location, Some(PathBuf::from("D:/Apps/X")));
        assert!(
            parse(&["--location", "D:/x"]).unwrap().is_err(),
            "the window asks itself"
        );
        assert!(parse(&["--components"]).unwrap().is_err());
        // anything else is plain kago, like the uninstall entry's call.
        assert_eq!(parse(&["uninstall", "dev.x.y"]), None);
    }
}
