//! setup mode: this executable carries a package and was started to install
//! it. a window by default; `--silent` (or NSIS's `/S`) for no window and no
//! prompt, with `--components a,b` and `--location <dir>` for rollouts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::container::Container;
use kago::format::setup::{self, Embedded};
use kago::install::{InstallPlan, Operation, Scope};
use kago::manifest::SemanticPath;
use kago::platform;
use kago::transaction;

const SETUP_USAGE: &str = "\
usage: <setup>.exe [--silent | /S] [--components <a,b>] [--location <dir>]
                   [--scope user|machine]

  (nothing)           open the setup window
  --silent, /S        install without a window, with the defaults
  --components <a,b>  with --silent: these optional components, besides the
                      required ones; an empty list installs only those
  --location <dir>    with --silent: install into this folder
  --scope <s>         with --silent, for a package that lets you pick:
                      machine (for everyone, the default) or user

exit code 0 when installed, 1 otherwise; errors go to stderr.
";

/// what the setup's own command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct SetupArgs {
    silent: bool,
    components: Option<Vec<String>>,
    location: Option<PathBuf>,
    scope: Option<Scope>,
    /// started by the setup window with administrator rights, to install
    /// for everyone what was picked there.
    elevated: bool,
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
            "--scope" => match it.next().map(|s| s.parse()) {
                Some(Ok(scope)) => out.scope = Some(scope),
                Some(Err(e)) => return Some(Err(e)),
                None => return Some(Err("`--scope` needs user or machine".to_owned())),
            },
            "--elevated" => out.elevated = true,
            _ => return None,
        }
    }
    let picked = out.components.is_some() || out.location.is_some() || out.scope.is_some();
    if !out.silent && !out.elevated && picked {
        return Some(Err(
            "`--components`, `--location` and `--scope` go with `--silent`; the window asks for them"
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
    let plan = match Container::open(&exe, embedded.offset) {
        Ok(c) => c.header.plan,
        Err(e) => {
            eprintln!("setup failed: {e}");
            return Some(ExitCode::FAILURE);
        }
    };
    let scope = match starting_scope(&plan, parsed.scope) {
        Ok(scope) => scope,
        Err(e) => {
            eprintln!("setup: {e}");
            return Some(ExitCode::FAILURE);
        }
    };
    // for everyone goes to Program Files: ask for administrator rights
    // once, by starting this setup again with the UAC prompt. a window
    // that lets the person pick asks only once they picked everyone.
    let window = !parsed.silent && cfg!(feature = "gui");
    let asks_later = window && plan.scope_open && !parsed.elevated;
    if scope == Scope::Machine && !asks_later && !platform::is_elevated() {
        let mut again = args.clone();
        if plan.scope_open && parsed.scope.is_none() {
            again.extend(["--scope".to_owned(), scope.to_string()]);
        }
        return Some(match platform::run_elevated(&exe, &again, true) {
            Ok(code) => u8::try_from(code).map_or(ExitCode::FAILURE, ExitCode::from),
            Err(e) => {
                eprintln!("setup: {e}");
                ExitCode::FAILURE
            }
        });
    }
    // one setup per package at a time. the one started with administrator
    // rights waits a moment for the window that started it to close.
    let wait = if parsed.elevated { 10_000 } else { 0 };
    let Some(_instance) = platform::claim(plan.package.as_str(), wait) else {
        if window {
            #[cfg(feature = "gui")]
            {
                let (title, text) = kago::gui::setup::already_running(&plan.name);
                platform::tell(&title, &text);
            }
        }
        eprintln!("setup: {} is being installed or removed already", plan.name);
        return Some(ExitCode::FAILURE);
    };
    if window {
        #[cfg(feature = "gui")]
        return Some(self::window(exe, embedded, &plan, scope, &parsed));
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
        scope,
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

/// where the install starts: the scope asked for, else the one it is
/// installed in, else for everyone when the person may pick.
fn starting_scope(plan: &InstallPlan, asked: Option<Scope>) -> Result<Scope, String> {
    if !plan.scope_open {
        return match asked {
            Some(asked) if asked != plan.scope => Err(format!(
                "{} installs for {} only",
                plan.name,
                everyone_or_user(plan.scope)
            )),
            _ => Ok(plan.scope),
        };
    }
    if let Some(asked) = asked {
        return Ok(asked);
    }
    let installed = platform::host().ok().and_then(|p| {
        [Scope::Machine, Scope::User].into_iter().find(|scope| {
            transaction::store(p.as_ref(), *scope)
                .ok()
                .and_then(|store| store.read_receipt(&plan.package).ok().flatten())
                .is_some()
        })
    });
    Ok(installed.unwrap_or(Scope::Machine))
}

fn everyone_or_user(scope: Scope) -> &'static str {
    match scope {
        Scope::Machine => "everyone",
        Scope::User => "the current user",
    }
}

/// what installing in `scope` would find: the installed version and its
/// folder, or the default folder.
#[cfg(feature = "gui")]
fn place(
    host: Option<&dyn platform::Platform>,
    plan: &InstallPlan,
    scope: Scope,
) -> (kago::gui::setup::Place, Option<PathBuf>) {
    use kago::gui::setup::{Existing, Place};
    use kago::manifest::Root;

    let receipt = host
        .and_then(|p| transaction::store(p, scope).ok())
        .and_then(|store| store.read_receipt(&plan.package).ok().flatten());
    let elsewhere = host.is_some_and(|p| other_install(p, scope, &plan.package).is_some());
    let existing = match &receipt {
        Some(r) => match r.version.cmp_precedence(&plan.version) {
            std::cmp::Ordering::Equal => Existing::Same,
            std::cmp::Ordering::Less => Existing::Older(r.version.to_string()),
            std::cmp::Ordering::Greater => Existing::Newer {
                version: r.version.to_string(),
                allowed: plan.downgrade == kago::manifest::Downgrade::Allow,
            },
        },
        None if elsewhere => Existing::Elsewhere {
            for_everyone: scope == Scope::User,
        },
        None => Existing::None,
    };
    let installed = receipt.and_then(|r| r.roots.get(&Root::App).cloned());
    let default = host
        .and_then(|p| p.roots(scope, &plan.name).ok())
        .and_then(|r| r.0.get(&Root::App).cloned());
    let place = Place {
        location: installed.clone().or(default),
        existing,
    };
    (place, installed)
}

#[cfg(feature = "gui")]
fn window(
    exe: PathBuf,
    embedded: Embedded,
    plan: &InstallPlan,
    scope: Scope,
    parsed: &SetupArgs,
) -> ExitCode {
    use kago::gui::setup::{Folder, Info, Installed, License, Picked, run};
    use kago::install::check_folder;
    use kago::manifest::FolderChoice;

    let (info, look, folder) = match Container::open(&exe, embedded.offset) {
        Ok(mut c) => {
            let brand = window_brand(&mut c);
            let host = platform::host().ok();
            let host = host.as_deref();
            let other_scope = match scope {
                Scope::Machine => Scope::User,
                Scope::User => Scope::Machine,
            };
            let (here, folder_now) = place(host, plan, scope);
            // an update or repair starts with what is installed ticked.
            let kept = host
                .and_then(|p| transaction::store(p, scope).ok())
                .and_then(|store| store.read_receipt(&plan.package).ok().flatten())
                .map(|r| plan.kept_choices(&r.components, &r.offered));
            let (there, folder_other) = place(host, plan, other_scope);
            // the elevated setup installs for everyone only.
            let open = plan.scope_open && !parsed.elevated;
            let folder = (c.header.setup.folder == FolderChoice::Choose).then(|| {
                let installed = move |everyone: bool| {
                    if everyone == (scope == Scope::Machine) {
                        folder_now.clone()
                    } else {
                        folder_other.clone()
                    }
                };
                Folder {
                    browse: platform::pick_folder,
                    check: Box::new(move |dir: &Path, everyone| {
                        check_folder(dir, installed(everyone).as_deref())
                    }),
                }
            });
            let picked = parsed.elevated.then(|| Picked {
                components: parsed.components.clone().unwrap_or_else(|| {
                    plan.choices
                        .iter()
                        .filter(|c| c.default)
                        .map(|c| c.id.clone())
                        .collect()
                }),
                location: parsed.location.clone(),
                for_everyone: true,
            });
            let info = Info {
                name: plan.name.clone(),
                version: plan.version.to_string(),
                publisher: plan.publisher.clone(),
                place: here,
                for_everyone: scope == Scope::Machine,
                other: open.then_some(there),
                picked,
                brand,
                license: c.header.setup.license.as_ref().map(|l| License {
                    text: l.text.clone(),
                    accept: l.accept,
                }),
                components: window_components(plan, kept.as_deref()),
            };
            (info, c.header.setup.look, folder)
        }
        Err(e) => {
            eprintln!("setup failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    // picking everyone in a window without administrator rights starts
    // this setup again with them, carrying what was picked.
    let elevate = (info.other.is_some() && !platform::is_elevated()).then(|| handoff(exe.clone()));
    let work = move |reporter: &kago::gui::setup::Reporter, picked: Picked| {
        let mut report =
            |fraction, stage: Stage<'_>| reporter.report(fraction, window_step(&stage));
        let choice = Choice {
            components: Some(&picked.components),
            location: picked.location.as_deref(),
            scope: if picked.for_everyone {
                Scope::Machine
            } else {
                Scope::User
            },
        };
        install(&exe, embedded, &choice, &mut report).map(|done| Installed {
            launch: done.launch,
        })
    };
    let running = running_check(plan.package.clone());
    match run(look, &info, folder, (elevate, Some(running)), work) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("cannot open the setup window: {e}");
            ExitCode::FAILURE
        }
    }
}

/// starts this setup again with administrator rights and what the person
/// picked, to install for everyone.
#[cfg(feature = "gui")]
fn handoff(exe: PathBuf) -> kago::gui::setup::Elevate {
    Box::new(move |picked: &kago::gui::setup::Picked| {
        let mut args = vec![
            "--elevated".to_owned(),
            "--scope".to_owned(),
            Scope::Machine.to_string(),
            "--components".to_owned(),
            picked.components.join(","),
        ];
        if let Some(dir) = &picked.location {
            args.extend(["--location".to_owned(), dir.display().to_string()]);
        }
        platform::run_elevated(&exe, &args, false).map(|_| ())
    })
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

/// the plan's choices as tick boxes, ticked as `kept` says for an update,
/// else as the package says. one for something this machine lacks starts
/// unticked, unless it cannot be left out.
#[cfg(feature = "gui")]
fn window_components(
    plan: &InstallPlan,
    kept: Option<&[String]>,
) -> Vec<kago::gui::setup::Component> {
    plan.choices
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
                default: choice.required
                    || kept.map_or(choice.default && missing.is_none(), |k| {
                        k.contains(&choice.id)
                    }),
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

/// the programs using what `package` installed, for the window to ask
/// before closing them.
#[cfg(feature = "gui")]
fn running_check(package: kago::package::PackageId) -> kago::gui::setup::Running {
    Box::new(move || {
        platform::host()
            .ok()
            .map(|p| installed_files(p.as_ref(), &package))
            .and_then(|files| platform::programs_using(&files).ok())
            .unwrap_or_default()
    })
}

/// the plan with what the person picked, after checking the picks, the
/// folder and that the drives have room.
fn chosen(
    plan: &InstallPlan,
    choice: &Choice<'_>,
    store: &transaction::Store,
    platform: &dyn kago::platform::Platform,
    staging: &Path,
) -> Result<InstallPlan, String> {
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
    // an update or repair keeps what is installed.
    let previous = store.read_receipt(&plan.package).ok().flatten();
    let plan = match (choice.components, &previous) {
        (Some(chosen), _) => plan.select(chosen.iter().map(String::as_str)),
        (None, Some(r)) => {
            let kept = plan.kept_choices(&r.components, &r.offered);
            plan.select(kept.iter().map(String::as_str))
        }
        (None, None) => plan.with_defaults(),
    };
    let app = choice
        .location
        .map(Path::to_path_buf)
        .or_else(|| previous.and_then(|r| r.roots.get(&kago::manifest::Root::App).cloned()))
        .or_else(|| {
            let roots = platform.roots(plan.scope, &plan.name).ok()?;
            roots.0.get(&kago::manifest::Root::App).cloned()
        });
    if let Some(app) = app {
        check_space(plan.payload_size(), staging, &app)?;
    }
    Ok(plan)
}

/// refuses when a drive lacks room. the payload is unpacked into
/// `staging`, then copied to `app`, so both hold it for a moment.
fn check_space(needed: u64, staging: &Path, app: &Path) -> Result<(), String> {
    /// the runtime, the receipt and the journal.
    const SLACK: u64 = 16 << 20;
    let drive = |p: &Path| {
        p.components()
            .next()
            .map(|c| c.as_os_str().to_ascii_uppercase())
    };
    let mut wants = vec![(staging, needed + SLACK)];
    if drive(staging) == drive(app) {
        wants[0].1 += needed;
    } else {
        wants.push((app, needed + SLACK));
    }
    for (path, want) in wants {
        if let Some(free) = platform::free_space(path)
            && free < want
        {
            return Err(format!(
                "not enough space on the drive of {}: {} needed, {} free",
                path.display(),
                megabytes(want),
                megabytes(free)
            ));
        }
    }
    Ok(())
}

fn megabytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "shown rounded")]
    let mb = bytes as f64 / 1_000_000.0;
    format!("{mb:.1} MB")
}

/// every file the package has installed, for everyone or just this user.
fn installed_files(
    platform: &dyn kago::platform::Platform,
    package: &kago::package::PackageId,
) -> Vec<PathBuf> {
    [Scope::User, Scope::Machine]
        .into_iter()
        .filter_map(|scope| transaction::store(platform, scope).ok())
        .filter_map(|store| store.read_receipt(package).ok().flatten())
        .flat_map(|receipt| receipt.files())
        .collect()
}

/// the same package installed in the other scope, with that scope's store.
fn other_install(
    platform: &dyn kago::platform::Platform,
    scope: Scope,
    package: &kago::package::PackageId,
) -> Option<(Scope, transaction::Store)> {
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
struct Choice<'a> {
    /// optional components to install, the defaults when `None`.
    components: Option<&'a [String]>,
    /// `{app}` when not the default or the installed version's folder.
    location: Option<&'a Path>,
    /// for everyone or just this user.
    scope: Scope,
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
    let plan = container
        .header
        .plan
        .in_scope(choice.scope)
        .ok_or_else(|| {
            let plan = &container.header.plan;
            format!(
                "{} installs for {} only",
                plan.name,
                everyone_or_user(plan.scope)
            )
        })?;
    let platform = platform::host()?;
    let store = transaction::store(platform.as_ref(), plan.scope).map_err(|e| e.to_string())?;
    let staging = store.root().join("staging").join(plan.package.as_str());
    let _ = fs::remove_dir_all(&staging);
    let result = (|| {
        // choose first: what is not installed is not unpacked either.
        let plan = chosen(&plan, choice, &store, platform.as_ref(), &staging)?;
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
        // the installed app, or anything else using its files, is closed:
        // the window asked first; a silent install closes it like Inno's
        // CloseApplications.
        platform::close_programs(&installed_files(platform.as_ref(), &plan.package))
            .map_err(|e| format!("cannot close the programs using {}: {e}", plan.name))?;
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
        let picked = parse(&["/S", "--scope", "user"]).unwrap().unwrap();
        assert_eq!(picked.scope, Some(Scope::User));
        assert!(parse(&["/S", "--scope", "all"]).unwrap().is_err());
        assert!(
            parse(&["--scope", "user"]).unwrap().is_err(),
            "the window asks"
        );
        let handed = parse(&["--elevated", "--scope", "machine", "--components", ""])
            .unwrap()
            .unwrap();
        assert!(handed.elevated && !handed.silent);
        assert_eq!(handed.components, Some(Vec::new()));
        // anything else is plain kago, like the uninstall entry's call.
        assert_eq!(parse(&["uninstall", "dev.x.y"]), None);
    }
}
