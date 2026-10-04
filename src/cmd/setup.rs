//! setup mode: this executable carries a package and was started to install
//! it.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

use kago::container::Container;
use kago::format::setup::{self, Embedded};
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
    let code = match install(&exe, embedded) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("setup failed: {e}");
            ExitCode::FAILURE
        }
    };
    if !silent {
        print!("\npress Enter to close");
        let _ = io::stdout().flush();
        let _ = io::stdin().lock().read_line(&mut String::new());
    }
    Some(code)
}

fn install(exe: &Path, embedded: Embedded) -> Result<(), String> {
    let mut container = Container::open(exe, embedded.offset).map_err(|e| e.to_string())?;
    let header = &container.header.plan;
    println!("{} {} setup", header.name, header.version);
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
        println!("installing for {}...", plan.scope);
        let receipt = transaction::install(&plan, platform.as_ref(), &store, &runtime)
            .map_err(|e| e.to_string())?;
        if let Some(app) = receipt.roots.get(&kago::manifest::Root::App) {
            println!("installed to {}", app.display());
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}
