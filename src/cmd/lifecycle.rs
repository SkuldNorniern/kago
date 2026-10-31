//! commands that change the machine: install and uninstall, plus
//! status which only reads.

use std::process::ExitCode;

use kago::install::{Scope, check_folder};
use kago::manifest::FolderChoice;
use kago::package::{PackageId, Target};
use kago::platform::{self, Platform};
use kago::transaction::{self, Store};

use super::args::Args;
use super::kagofile;

fn scope(args: &Args) -> Result<Scope, String> {
    args.value("--scope").map_or(Ok(Scope::User), str::parse)
}

/// the scope `package` is installed in: `--scope` when given, else the one
/// that has it, per-user first.
fn installed_scope(args: &Args, package: &PackageId) -> Result<Scope, String> {
    if args.value("--scope").is_some() {
        return scope(args);
    }
    let platform = platform::host()?;
    for scope in [Scope::User, Scope::Machine] {
        let Ok(store) = transaction::store(platform.as_ref(), scope) else {
            continue;
        };
        if store.read_receipt(package).ok().flatten().is_some() {
            return Ok(scope);
        }
    }
    Err(format!("{package} is not installed"))
}

fn open(scope: Scope) -> Result<(Box<dyn Platform>, Store), String> {
    let platform = platform::host()?;
    let store = transaction::store(platform.as_ref(), scope).map_err(|e| e.to_string())?;
    Ok((platform, store))
}

/// installs the manifest for this machine.
pub fn install(args: &Args) -> Result<ExitCode, String> {
    let target = Target::host().ok_or("this machine is not a kago target")?;
    let Some((loaded, plan)) = kagofile::build(args, target, false)? else {
        return Ok(ExitCode::FAILURE);
    };
    if plan.scope == Scope::Machine && !platform::is_elevated() {
        return Err(format!(
            "{} installs for everyone into Program Files; run this from a terminal opened as administrator",
            plan.name
        ));
    }
    let (platform, store) = open(plan.scope)?;
    let mut given = std::collections::BTreeMap::new();
    for set in args.all("--set") {
        let (id, value) = set
            .split_once('=')
            .ok_or_else(|| format!("`--set {set}`: expected <field>=<value>"))?;
        given.insert(id.to_owned(), value.to_owned());
    }
    let installed = store
        .read_receipt(&plan.package)
        .map_err(|e| e.to_string())?;
    // without --components an update keeps what is installed now.
    let plan = match (&installed, args.value("--components")) {
        (_, Some(_)) => plan,
        (Some(r), None) => {
            let kept = plan.kept_choices(&r.components, &r.offered);
            plan.select(kept.iter().map(String::as_str))
        }
        (None, None) => plan.with_defaults(),
    };
    let earlier = installed.map(|r| r.answers);
    let plan = plan.answered(&given, earlier.as_ref())?;
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
    if let Some(old) = store.read_receipt(&plan.package).ok().flatten() {
        platform::close_programs(&old.files())
            .map_err(|e| format!("cannot close the programs using {}: {e}", plan.name))?;
    }
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
        return Err(
            "usage: kago uninstall <package-id> [--scope user|machine] [--purge]".to_owned(),
        );
    };
    let package = PackageId::new(id).map_err(|e| e.to_string())?;
    let scope = installed_scope(args, &package)?;
    if scope == Scope::Machine && !platform::is_elevated() {
        return Err(format!(
            "{package} is installed for everyone; run this from a terminal opened as administrator"
        ));
    }
    let (platform, store) = open(scope)?;
    let receipt = store.read_receipt(&package).map_err(|e| e.to_string())?;
    // the package may say no, before anything is closed.
    transaction::ask(&package, &store).map_err(|e| e.to_string())?;
    if let Some(receipt) = &receipt {
        platform::close_programs(&receipt.files())
            .map_err(|e| format!("cannot close the programs using {package}: {e}"))?;
    }
    let mut done =
        transaction::remove(&package, platform.as_ref(), &store).map_err(|e| e.to_string())?;
    if args.flag("--purge")
        && let Some(receipt) = &receipt
    {
        done.failed
            .extend(transaction::purge(receipt, platform.as_ref(), &done.kept));
        done.kept.clear();
    }
    if !done.failed.is_empty() {
        let scheduled = transaction::remove_at_restart(&package, platform.as_ref(), &store)
            .map_err(|e| e.to_string())?;
        done.failed
            .retain(|f| !scheduled.iter().any(|s| f.starts_with(s.as_str())));
        for path in &scheduled {
            println!("removed when Windows restarts: {path}");
        }
    }
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

pub fn status(args: &Args) -> Result<ExitCode, String> {
    args.at_most_one()?;
    let scopes = match args.value("--scope") {
        Some(_) => vec![scope(args)?],
        None => vec![Scope::User, Scope::Machine],
    };
    let mut found = Vec::new();
    for scope in scopes {
        let Ok((_, store)) = open(scope) else {
            continue;
        };
        found.extend(store.receipts().unwrap_or_default());
    }
    // one package when named.
    if let Some(id) = args.positional.first() {
        found.retain(|r| r.package.as_str() == id);
        if found.is_empty() {
            return Err(format!("{id} is not installed"));
        }
    }
    if args.flag("--json") {
        let list: Vec<serde_json::Value> = found
            .iter()
            .map(|r| {
                serde_json::json!({
                    "package": r.package.as_str(),
                    "name": r.name,
                    "version": r.version.to_string(),
                    "scope": r.scope.to_string(),
                    "components": r.components,
                    "location": r.roots.get(&kago::manifest::Root::App),
                    "update": r.update,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }
    for r in &found {
        println!("{} {} ({}) {}", r.package, r.version, r.scope, r.name);
    }
    if found.is_empty() {
        println!("nothing installed");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn verify(args: &Args) -> Result<ExitCode, String> {
    let [id] = args.positional.as_slice() else {
        return Err("usage: kago verify <package-id> [--scope user|machine]".to_owned());
    };
    let package = PackageId::new(id).map_err(|e| e.to_string())?;
    let (_, store) = open(installed_scope(args, &package)?)?;
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
