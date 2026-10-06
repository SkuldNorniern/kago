//! starting a program for `run`: with the arguments the Kagofile gave,
//! never through a shell.

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// longest a program may take before it counts as failed.
const LIMIT: Duration = Duration::from_secs(120);

/// longest a `required` program at uninstall may take: it may wait for a
/// person, like one typing a code from an administrator.
const WAITING_LIMIT: Duration = Duration::from_secs(30 * 60);

/// runs `exe` with `arguments` in its own folder and waits for it.
///
/// # Errors
///
/// when it cannot start, exits with a failure, or runs past the limit.
pub(super) fn run(exe: &Path, arguments: &[String]) -> Result<(), String> {
    run_within(exe, arguments, LIMIT)
}

/// [`run`] for a program that may wait for a person.
///
/// # Errors
///
/// as [`run`].
pub(super) fn run_waiting(exe: &Path, arguments: &[String]) -> Result<(), String> {
    run_within(exe, arguments, WAITING_LIMIT)
}

fn run_within(exe: &Path, arguments: &[String], limit: Duration) -> Result<(), String> {
    let mut command = Command::new(exe);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = exe.parent().filter(|d| !d.as_os_str().is_empty()) {
        command.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW`: no console flashes up over the setup.
        const NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(NO_WINDOW);
    }
    let name = exe.display();
    let mut child = command
        .spawn()
        .map_err(|e| format!("{name} cannot start: {e}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("{name}: {e}"))? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("{name} failed ({status})"))
            };
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            return Err(format!(
                "{name} did not finish within {} seconds",
                limit.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
