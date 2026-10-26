//! `kago update`: finds newer versions of installed packages from their
//! published releases and runs the new setup. the app can run this too.

use std::process::{Command, ExitCode};

use kago::install::Scope;
use kago::platform;
use kago::receipt::Receipt;
use kago::transaction;
use kago::update::{self, Offer};

use super::args::Args;

pub fn run(args: &Args) -> Result<ExitCode, String> {
    let wanted = match args.positional.as_slice() {
        [] => None,
        [id] => Some(id.as_str()),
        _ => return Err("usage: kago update [<package-id>] [--check] [--silent]".to_owned()),
    };
    let check_only = args.flag("--check");
    let silent = args.flag("--silent");
    let host = platform::host()?;
    let mut found = false;
    let mut failed = false;
    for scope in [Scope::User, Scope::Machine] {
        let Ok(store) = transaction::store(host.as_ref(), scope) else {
            continue;
        };
        for receipt in store.receipts().unwrap_or_default() {
            if wanted.is_some_and(|id| id != receipt.package.as_str()) {
                continue;
            }
            found = true;
            let Some(feed) = receipt.update.clone() else {
                if wanted.is_some() {
                    return Err(format!(
                        "{} has no `update` line in its Kagofile, so kago does not know where to look",
                        receipt.package
                    ));
                }
                continue;
            };
            let dir = store.root().join("updates").join(receipt.package.as_str());
            let result = newer(&receipt, &feed, &dir).and_then(|offer| match offer {
                None => {
                    println!("{} {} is up to date", receipt.package, receipt.version);
                    Ok(())
                }
                Some(offer) if check_only => {
                    println!(
                        "{} {} -> {} ({} bytes)",
                        receipt.package, receipt.version, offer.version, offer.size
                    );
                    Ok(())
                }
                Some(offer) => install(&receipt, &offer, &dir, silent),
            });
            let _ = std::fs::remove_dir_all(&dir);
            if let Err(e) = result {
                eprintln!("{}: {e}", receipt.package);
                failed = true;
            }
        }
    }
    if !found {
        return Err(wanted.map_or_else(
            || "nothing installed".to_owned(),
            |id| format!("{id} is not installed"),
        ));
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// the newer version `feed` offers over what `receipt` has, if any.
fn newer(receipt: &Receipt, feed: &str, dir: &std::path::Path) -> Result<Option<Offer>, String> {
    let (feed, at) = update::read(feed, dir)?;
    feed.offer(&receipt.package, &receipt.version, receipt.target, &at)
}

/// downloads the setup and runs it: with its window, which says it updates
/// from the installed version, or `--silent`. a setup for everyone asks
/// for administrator rights itself.
fn install(
    receipt: &Receipt,
    offer: &Offer,
    dir: &std::path::Path,
    silent: bool,
) -> Result<(), String> {
    println!(
        "{} {} -> {}: downloading",
        receipt.package, receipt.version, offer.version
    );
    let setup = update::download(offer, dir)?;
    let mut command = Command::new(&setup);
    if silent {
        command.arg("/S");
    }
    let status = command
        .status()
        .map_err(|e| format!("cannot start {}: {e}", setup.display()))?;
    if status.success() {
        println!("{} updated to {}", receipt.package, offer.version);
        Ok(())
    } else {
        Err(format!(
            "the {} setup ended with {}",
            offer.version,
            status.code().unwrap_or(1)
        ))
    }
}
