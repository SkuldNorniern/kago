//! uninstaller mode: kago's runtime saved as `{app}/uninstall.exe`, what
//! Settings › Apps runs. it finds its package by the folder it sits in,
//! moves itself to a temp copy (a running program cannot delete itself),
//! then asks, removes and reports, like an NSIS uninstaller.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use kago::install::Scope;
use kago::manifest::Root;
use kago::package::PackageId;
use kago::platform;
use kago::transaction;

/// the file name the installer gives it.
fn file_name() -> String {
    format!("uninstall{}", std::env::consts::EXE_SUFFIX)
}

/// runs as the uninstaller when started as one. `None` means not one.
pub fn run() -> Option<ExitCode> {
    let exe = std::env::current_exe().ok()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    // the temp copy, started by the uninstaller in the app folder.
    if let [flag, id, rest @ ..] = args.as_slice()
        && flag == "--uninstall"
    {
        let silent = rest.iter().any(|a| is_silent(a));
        let purge = rest.iter().any(|a| a == "--purge");
        let scope = rest
            .windows(2)
            .find(|w| w[0] == "--scope")
            .and_then(|w| w[1].parse().ok())
            .unwrap_or(Scope::User);
        return Some(remove(id, scope, (silent, purge), &exe));
    }
    let named = exe
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(&file_name()));
    // `/S` alone, or with `--purge` to remove the user's data too.
    let silent = args.iter().any(|a| is_silent(a));
    let purge = args.iter().any(|a| a == "--purge");
    if !named || args.iter().any(|a| !is_silent(a) && a != "--purge") {
        return None;
    }
    Some(start(&exe, (silent, purge)))
}

fn is_silent(arg: &str) -> bool {
    matches!(arg, "/S" | "/s" | "--silent")
}

/// finds the package installed in this folder and starts the temp copy.
fn start(exe: &Path, (silent, purge): (bool, bool)) -> ExitCode {
    // per-user installs first, then the ones for everyone.
    let found = (|| -> Result<(PackageId, Scope), String> {
        let platform = platform::host()?;
        let dir = exe.parent().ok_or("the uninstaller has no folder")?;
        for scope in [Scope::User, Scope::Machine] {
            let Ok(store) = transaction::store(platform.as_ref(), scope) else {
                continue;
            };
            let here = store
                .receipts()
                .unwrap_or_default()
                .into_iter()
                // the folder may be written in other case than it was installed.
                .find(|r| {
                    r.roots
                        .get(&Root::App)
                        .is_some_and(|app| app.as_os_str().eq_ignore_ascii_case(dir))
                });
            if let Some(receipt) = here {
                return Ok((receipt.package, scope));
            }
        }
        Err(format!("nothing kago installed lives in {}", dir.display()))
    })();
    let (package, scope) = match found {
        Ok(found) => found,
        Err(e) => {
            eprintln!("uninstall: {e}");
            return ExitCode::FAILURE;
        }
    };
    let copy = temp_copy(package.as_str());
    // the copy must not start in the app folder either; see `remove`.
    let _ = std::env::set_current_dir(std::env::temp_dir());
    if let Err(e) = std::fs::copy(exe, &copy) {
        eprintln!("uninstall: cannot prepare {}: {e}", copy.display());
        return ExitCode::FAILURE;
    }
    let mut arguments = vec![
        "--uninstall".to_owned(),
        package.to_string(),
        "--scope".to_owned(),
        scope.to_string(),
    ];
    if silent {
        arguments.push("--silent".to_owned());
    }
    if purge {
        arguments.push("--purge".to_owned());
    }
    // hand off and exit at once: this file is part of what gets removed,
    // and Windows cannot delete a running program. tools that need the
    // real exit code run QuietUninstallString, which waits.
    let started = if scope == Scope::Machine && !platform::is_elevated() {
        platform::run_elevated(&copy, &arguments, false)
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        Command::new(&copy)
            .args(&arguments)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    match started {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("uninstall: {e}");
            ExitCode::FAILURE
        }
    }
}

/// the temp copy: asks (unless silent), removes, then deletes itself.
fn remove(id: &str, scope: Scope, (silent, purge): (bool, bool), exe: &Path) -> ExitCode {
    // started from the app folder, this process has it as its working
    // folder, and Windows does not delete a folder a process works in.
    let _ = std::env::set_current_dir(std::env::temp_dir());
    // never alongside a setup for the same package.
    let Some(_instance) = platform::claim(id, 10_000) else {
        #[cfg(feature = "gui")]
        if !silent {
            let (title, text) = kago::gui::setup::already_running(id);
            platform::tell(&title, &text);
        }
        eprintln!("uninstall: {id} is being installed or removed already");
        delete_later(exe, id);
        return ExitCode::FAILURE;
    };
    let code = match PackageId::new(id) {
        Ok(package) => remove_package(&package, scope, (silent, purge)),
        Err(e) => {
            eprintln!("uninstall: {e}");
            ExitCode::FAILURE
        }
    };
    delete_later(exe, id);
    code
}

/// the copy `start` made in the temp folder; never any other kago, like
/// one run by hand with `--uninstall`.
fn temp_copy(id: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{id}-uninstall{}", std::env::consts::EXE_SUFFIX))
}

fn remove_package(package: &PackageId, scope: Scope, (silent, purge): (bool, bool)) -> ExitCode {
    let opened = platform::host().and_then(|p| {
        let store = transaction::store(p.as_ref(), scope).map_err(|e| e.to_string())?;
        Ok((p, store))
    });
    let (platform, store) = match opened {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("uninstall: {e}");
            return ExitCode::FAILURE;
        }
    };
    #[cfg(feature = "gui")]
    if !silent {
        let receipt = match store.read_receipt(package) {
            Ok(Some(receipt)) => receipt,
            Ok(None) => {
                eprintln!("uninstall: {package} is not installed");
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("uninstall: {e}");
                return ExitCode::FAILURE;
            }
        };
        let look = receipt.look.clone();
        let info = kago::gui::setup::Removing {
            name: receipt.name.clone(),
            version: receipt.version.to_string(),
            location: receipt.roots.get(&Root::App).cloned(),
            // receipts from before the look was kept get the classic one.
            look: look
                .as_ref()
                .map_or(kago::manifest::SetupLook::Native, |l| l.setup),
            accent: look.as_ref().and_then(|l| l.accent),
            text: look.and_then(|l| l.text),
            logo: installed_icon(&receipt),
            running: platform::programs_using(&receipt.files()).unwrap_or_default(),
        };
        let package = package.clone();
        let work = move |purge: bool| {
            let platform = platform::host()?;
            let store = transaction::store(platform.as_ref(), scope).map_err(|e| e.to_string())?;
            uninstall_retrying(&package, platform.as_ref(), &store, purge)
                .map(|done| kago::gui::setup::Removed {
                    kept: done.kept,
                    failed: done.failed,
                    at_restart: done.at_restart,
                })
                .map_err(|e| e.to_string())
        };
        drop((platform, store));
        return match kago::gui::setup::run_uninstall(&info, work) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("uninstall: cannot open the window: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let _ = silent;
    match uninstall_retrying(package, platform.as_ref(), &store, purge) {
        Ok(done) if done.failed.is_empty() => ExitCode::SUCCESS,
        Ok(done) => {
            for failed in done.failed {
                eprintln!("could not remove {failed}");
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("uninstall: {e}");
            ExitCode::FAILURE
        }
    }
}

/// the app's installed `.ico` as an image, for the modern window.
#[cfg(feature = "gui")]
fn installed_icon(receipt: &kago::receipt::Receipt) -> Option<kago::format::image::Rgba> {
    use kago::format::{ico, image};
    let path = receipt.resources.iter().find_map(|r| match r {
        kago::receipt::Resource::File { path, .. }
            if path.to_string().to_ascii_lowercase().ends_with(".ico") =>
        {
            receipt.absolute(path)
        }
        _ => None,
    })?;
    let icon = ico::parse(&std::fs::read(path).ok()?).ok()?;
    image::decode_png(&image::logo_from_icon(&icon)?).ok()
}

/// uninstalls, then tries again for a few seconds whatever was in use,
/// such as the app folder's `uninstall.exe` that started this copy and is
/// still closing.
fn uninstall_retrying(
    package: &PackageId,
    platform: &dyn platform::Platform,
    store: &transaction::Store,
    purge: bool,
) -> Result<transaction::Uninstalled, transaction::Error> {
    // the package may say no, before anything is closed.
    transaction::ask(package, store)?;
    let receipt = store.read_receipt(package)?;
    // what still runs from the app folder goes first; what cannot be
    // closed shows up as not removed.
    if let Ok(Some(receipt)) = store.read_receipt(package) {
        let _ = platform::close_programs(&receipt.files());
    }
    let mut done = transaction::remove(package, platform, store)?;
    for _ in 0..20 {
        if done.failed.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        match transaction::remove(package, platform, store) {
            Ok(again) => done.failed = again.failed,
            Err(transaction::Error::NotInstalled(_)) => done.failed.clear(),
            Err(e) => return Err(e),
        }
    }
    if !done.failed.is_empty() {
        // still in use: gone at the next restart.
        let scheduled = transaction::remove_at_restart(package, platform, store)?;
        done.failed
            .retain(|f| !scheduled.iter().any(|s| f.starts_with(s.as_str())));
        done.at_restart = scheduled;
    }
    if purge && let Some(receipt) = &receipt {
        done.failed
            .extend(transaction::purge(receipt, platform, &done.kept));
        done.kept.clear();
    }
    Ok(done)
}

/// the temp copy cannot delete itself while running; a hidden helper
/// waits a moment and deletes it, the way NSIS cleans up its `Au_.exe`.
fn delete_later(exe: &Path, id: &str) {
    let copy = temp_copy(id);
    let same = |a: &Path, b: &Path| {
        a.display().to_string().to_lowercase() == b.display().to_string().to_lowercase()
    };
    if !same(exe, &copy) {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW`.
        const NO_WINDOW: u32 = 0x0800_0000;
        // passed as written: Rust's own quoting (`\"`) is not what cmd reads.
        let script = format!(
            "/d /c ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"",
            exe.display()
        );
        let _ = Command::new("cmd")
            .raw_arg(script)
            .creation_flags(NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::fs::remove_file(exe);
    }
}
