//! commands that change the machine: install, uninstall, recover, plus
//! status which only reads.

use std::process::ExitCode;

use kago::install::{Scope, check_folder};
use kago::manifest::FolderChoice;
use kago::package::{PackageId, Target};
use kago::platform::{self, Platform};
use kago::transaction::{self, Recovered, Store};

use super::args::Args;
use super::plan;

fn scope(args: &Args) -> Result<Scope, String> {
    args.value("--scope").map_or(Ok(Scope::User), str::parse)
}

fn open(scope: Scope) -> Result<(Box<dyn Platform>, Store), String> {
    let platform = platform::host()?;
    let store = transaction::store(platform.as_ref(), scope).map_err(|e| e.to_string())?;
    Ok((platform, store))
}

/// installs the manifest for this machine.
pub fn install(args: &Args) -> Result<ExitCode, String> {
    let target = Target::host().ok_or("this machine is not a kago target")?;
    let Some((loaded, plan)) = plan::build(args, target, false)? else {
        return Ok(ExitCode::FAILURE);
    };
    if plan.scope == Scope::Machine && !platform::is_elevated() {
        return Err(format!(
            "{} installs for everyone into Program Files; run this from a terminal opened as administrator",
            plan.name
        ));
    }
    let (platform, store) = open(plan.scope)?;
    let plan = match args.value("--location") {
        None => plan,
        Some(_) if loaded.manifest.install.folder == FolderChoice::Fixed => {
            return Err(format!("{} installs into a fixed folder", plan.name));
        }
        Some(dir) => {
            let dir = std::path::absolute(dir).map_err(|e| format!("{dir}: {e}"))?;
            let installed = store
                .read_receipt(&plan.package)
                .map_err(|e| e.to_string())?
                .and_then(|r| r.roots.get(&kago::manifest::Root::App).cloned());
            check_folder(&dir, installed.as_deref())
                .map_err(|e| format!("{}: {e}", dir.display()))?;
            plan.at(&dir)
        }
    };
    let runtime = std::env::current_exe().map_err(|e| format!("cannot find kago itself: {e}"))?;
    println!(
        "installing {} {} for {}",
        plan.name, plan.version, plan.scope
    );
    let receipt = transaction::install(&plan, platform.as_ref(), &store, &runtime)
        .map_err(|e| e.to_string())?;
    if let Some(app) = receipt.roots.get(&kago::manifest::Root::App) {
        println!("installed to {}", app.display());
    }
    if receipt
        .resources
        .iter()
        .any(|r| matches!(r, kago::receipt::Resource::PathEntry { .. }))
    {
        println!("PATH changed; open a new terminal to use it");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn uninstall(args: &Args) -> Result<ExitCode, String> {
    let [id] = args.positional.as_slice() else {
        return Err("usage: kago uninstall <package-id> [--scope user|machine]".to_owned());
    };
    let package = PackageId::new(id).map_err(|e| e.to_string())?;
    let (platform, store) = open(scope(args)?)?;
    let done =
        transaction::uninstall(&package, platform.as_ref(), &store).map_err(|e| e.to_string())?;
    for kept in &done.kept {
        println!("kept {kept}");
    }
    if done.failed.is_empty() {
        println!("uninstalled {package}");
        Ok(ExitCode::SUCCESS)
    } else {
        for failed in &done.failed {
            eprintln!("could not remove {failed}");
        }
        eprintln!("close the app if it is running, then run `kago uninstall {package}` again");
        Ok(ExitCode::FAILURE)
    }
}

pub fn recover(args: &Args) -> Result<ExitCode, String> {
    args.at_most_one()?;
    let (platform, store) = open(scope(args)?)?;
    let done = transaction::recover(platform.as_ref(), &store).map_err(|e| e.to_string())?;
    if done.is_empty() {
        println!("nothing to recover");
    }
    for r in done {
        match r {
            Recovered::Finished(p) => println!("{p}: finished committed install"),
            Recovered::RolledBack(p) => println!("{p}: rolled back unfinished install"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn status(args: &Args) -> Result<ExitCode, String> {
    args.at_most_one()?;
    let (_, store) = open(scope(args)?)?;
    let receipts = store.receipts().map_err(|e| e.to_string())?;
    if receipts.is_empty() {
        println!("nothing installed");
    }
    for r in receipts {
        println!("{} {} ({}) {}", r.package, r.version, r.scope, r.name);
    }
    Ok(ExitCode::SUCCESS)
}

pub fn verify(args: &Args) -> Result<ExitCode, String> {
    let [id] = args.positional.as_slice() else {
        return Err("usage: kago verify <package-id> [--scope user|machine]".to_owned());
    };
    let package = PackageId::new(id).map_err(|e| e.to_string())?;
    let (_, store) = open(scope(args)?)?;
    let receipt = store
        .read_receipt(&package)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{package} is not installed in this scope"))?;
    let v = transaction::verify(&receipt);
    for m in &v.modified {
        println!("modified {m}");
    }
    for m in &v.missing {
        println!("missing  {m}");
    }
    println!("healthy  {} resources", v.healthy);
    if v.is_healthy() {
        Ok(ExitCode::SUCCESS)
    } else {
        println!("run the {} setup again to repair", receipt.name);
        Ok(ExitCode::FAILURE)
    }
}
