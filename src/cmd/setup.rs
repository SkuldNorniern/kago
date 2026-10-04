//! setup mode: this executable carries a package and was started to install
//! it. a window by default, `--silent` for no window and no prompt.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::container::Container;
use kago::format::setup::{self, Embedded};
use kago::install::Operation;
use kago::manifest::SemanticPath;
use kago::platform;
use kago::transaction;

/// installs the embedded package when there is one and the arguments are
/// empty or `--silent`. `None` means run as plain kago.
pub fn run_embedded() -> Option<ExitCode> {
    let exe = std::env::current_exe().ok()?;
    let embedded = setup::embedded(&exe).ok()??;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let silent = match args.as_slice() {
        [] => false,
        [flag] if flag == "--silent" => true,
        _ => return None,
    };
    if !silent {
        #[cfg(feature = "gui")]
        return Some(window(exe, embedded));
    }
    Some(
        match install(&exe, embedded, &Choice::default(), &mut |_, _| {}) {
            Ok(done) => {
                println!("installed {}", done.name);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("setup failed: {e}");
                ExitCode::FAILURE
            }
        },
    )
}

#[cfg(feature = "gui")]
fn window(exe: PathBuf, embedded: Embedded) -> ExitCode {
    use kago::gui::setup::{Component, Folder, Info, Installed, License, Picked, Step, run};
    use kago::install::check_folder;
    use kago::manifest::{FolderChoice, Root};

    let (info, look, folder) = match Container::open(&exe, embedded.offset) {
        Ok(c) => {
            let plan = &c.header.plan;
            let host = platform::host().ok();
            // where the installed version is, if there is one.
            let installed = host
                .as_ref()
                .and_then(|p| transaction::store(p.as_ref(), plan.scope).ok())
                .and_then(|store| store.read_receipt(&plan.package).ok().flatten())
                .and_then(|r| r.roots.get(&Root::App).cloned());
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
                upgrade: installed.is_some(),
                location: installed.or(default),
                license: c.header.setup.license.as_ref().map(|l| License {
                    text: l.text.clone(),
                    accept: l.accept,
                }),
                components: plan
                    .choices
                    .iter()
                    .map(|choice| {
                        let missing = choice
                            .detect
                            .filter(|d| !platform::detect(*d))
                            .map(|d| d.name().to_owned());
                        Component {
                            id: choice.id.clone(),
                            name: choice.name.clone(),
                            description: choice.description.clone(),
                            // not ticked for something this machine lacks.
                            default: choice.default && missing.is_none(),
                            depends: choice.depends.clone(),
                            size: choice.size,
                            missing,
                        }
                    })
                    .collect(),
            };
            (info, c.header.setup.look, folder)
        }
        Err(e) => {
            eprintln!("setup failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let work = move |reporter: &kago::gui::setup::Reporter, picked: Picked| {
        let mut report = |fraction, stage: Stage<'_>| {
            let step = match stage {
                Stage::Unpack(path) => Step::Unpack(path.to_string()),
                Stage::Apply(op) => match op {
                    Operation::CreateDirectory { path } => Step::Folder(path.to_string()),
                    Operation::InstallFile { target, .. } => Step::File(target.to_string()),
                    Operation::AddPathEntry { directory, .. } => Step::Path(directory.to_string()),
                    Operation::RegisterSteamVrDriver { directory } => {
                        Step::SteamVr(directory.to_string())
                    }
                    Operation::CreateShortcut { name, .. } => Step::Shortcut(name.clone()),
                    Operation::RegisterApplication { display_name, .. } => {
                        Step::Register(display_name.clone())
                    }
                    Operation::InstallService { id, .. } => Step::Service(id.clone()),
                },
            };
            reporter.report(fraction, step);
        };
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
        let operations = container.header.plan.operations.len();
        #[allow(clippy::cast_precision_loss)]
        let share = |done: usize, total: usize| done as f32 / total.max(1) as f32;
        let plan = container
            .extract_observed(&staging.join("payload"), &mut |i, n, target| {
                progress(share(i, n + operations), Stage::Unpack(target));
            })
            .map_err(|e| format!("cannot unpack: {e}"))?;
        let files = container.header.payload.len();
        let plan = match choice.components {
            Some(chosen) => plan.select(chosen.iter().map(String::as_str)),
            None => plan.with_defaults(),
        };
        let plan = match choice.location {
            Some(dir) => plan.at(dir),
            None => plan,
        };
        let runtime = staging.join(format!("kago{}", std::env::consts::EXE_SUFFIX));
        setup::extract_stub(exe, embedded, &runtime).map_err(|e| e.to_string())?;
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
