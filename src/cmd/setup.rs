//! setup mode: this executable carries a package and was started to install
//! it. a window by default, `--silent` for no window and no prompt.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::container::Container;
use kago::format::setup::{self, Embedded};
use kago::install::Operation;
use kago::manifest::Root;
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
    Some(match install(&exe, embedded) {
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
    use kago::gui::setup::{Info, Installed, run};

    let info = match Container::open(&exe, embedded.offset) {
        Ok(c) => {
            let plan = &c.header.plan;
            Info {
                name: plan.name.clone(),
                version: plan.version.to_string(),
                publisher: plan.publisher.clone(),
                location: platform::host()
                    .ok()
                    .and_then(|p| p.roots(plan.scope, &plan.name).ok())
                    .and_then(|r| r.0.get(&Root::App).cloned()),
            }
        }
        Err(e) => {
            eprintln!("setup failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let work = move || {
        install(&exe, embedded).map(|done| Installed {
            launch: done.launch,
        })
    };
    match run(&info, work) {
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
    launch: Option<PathBuf>,
}

fn install(exe: &Path, embedded: Embedded) -> Result<Done, String> {
    let mut container = Container::open(exe, embedded.offset).map_err(|e| e.to_string())?;
    let header = &container.header.plan;
    let platform = platform::host()?;
    let store = transaction::store(platform.as_ref(), header.scope).map_err(|e| e.to_string())?;
    let staging = store.root().join("staging").join(header.package.as_str());
    let _ = fs::remove_dir_all(&staging);
    let result = (|| {
        let plan = container
            .extract(&staging.join("payload"))
            .map_err(|e| format!("cannot unpack: {e}"))?;
        let runtime = staging.join(format!("kago{}", std::env::consts::EXE_SUFFIX));
        setup::extract_stub(exe, embedded, &runtime).map_err(|e| e.to_string())?;
        let receipt = transaction::install(&plan, platform.as_ref(), &store, &runtime)
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
