//! `kago inspect <setup.exe>`: what a built setup holds, read from the
//! package inside it, without installing anything.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use kago::container::{Container, Header};
use kago::format::{pe, setup};
use kago::install::Operation;

use super::args::Args;

/// the setup file and what reading it found.
struct Setup<'a> {
    path: &'a Path,
    size: u64,
    signed: bool,
    header: Header,
}

/// prints what the setup holds; `--plan` adds every step it would take,
/// `--json` gives it all as one JSON object.
pub fn run(args: &Args) -> Result<ExitCode, String> {
    args.at_most_one()?;
    let path = Path::new(
        args.first()
            .ok_or("usage: kago inspect <setup.exe> [--plan] [--json]")?,
    );
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let embedded = setup::embedded(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .ok_or_else(|| format!("{} is not a kago setup", path.display()))?;
    let container =
        Container::open(path, embedded.offset).map_err(|e| format!("{}: {e}", path.display()))?;
    let setup = Setup {
        path,
        size: bytes.len() as u64,
        signed: pe::is_signed(&bytes),
        header: container.header,
    };
    if args.flag("--json") {
        let report = json(&setup, args.flag("--plan"));
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        text(&setup, args.flag("--plan"));
    }
    Ok(ExitCode::SUCCESS)
}

fn json(setup: &Setup<'_>, operations: bool) -> serde_json::Value {
    let header = &setup.header;
    let plan = &header.plan;
    let packed: u64 = header.payload.iter().map(|e| e.packed).sum();
    serde_json::json!({
        "file": setup.path,
        "size": setup.size,
        "signed": setup.signed,
        "package": plan.package.as_str(),
        "name": plan.name,
        "version": plan.version.to_string(),
        "publisher": plan.publisher,
        "description": header.setup.description,
        "target": plan.target.to_string(),
        "scope": plan.scope.to_string(),
        "scope_open": plan.scope_open,
        "look": header.setup.look.to_string(),
        "license": header.setup.license.as_ref().map(|l| serde_json::json!({ "accept": l.accept })),
        "payload": { "files": header.payload.len(), "bytes": plan.payload_size(), "packed": packed },
        "components": plan.choices.iter().map(|c| serde_json::json!({
            "id": c.id, "name": c.name, "default": c.default,
            "required": c.required, "size": c.size,
        })).collect::<Vec<_>>(),
        "pages": plan.inputs,
        "prerequisites": plan.prerequisites.iter().map(|p| serde_json::json!({
            "id": p.id, "name": p.name, "admin": p.admin,
        })).collect::<Vec<_>>(),
        "update": plan.update,
        "assets": header.assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        "operations": operations.then_some(&plan.operations),
    })
}

fn mb(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a size to read, not to count")]
    let mb = bytes as f64 / 1_048_576.0;
    format!("{mb:.1} MB")
}

fn text(setup: &Setup<'_>, operations: bool) {
    let header = &setup.header;
    let plan = &header.plan;
    let by = plan
        .publisher
        .as_ref()
        .map(|p| format!(" by {p}"))
        .unwrap_or_default();
    println!("{} {} ({}){by}", plan.name, plan.version, plan.package);
    if let Some(about) = &header.setup.description {
        println!("  {about}");
    }
    println!();
    let signed = if setup.signed { "signed" } else { "unsigned" };
    println!(
        "setup:      {} ({}, {signed})",
        setup.path.display(),
        mb(setup.size)
    );
    println!("target:     {}", plan.target);
    if plan.scope_open {
        println!("scope:      {}, or the other when picked", plan.scope);
    } else {
        println!("scope:      {}", plan.scope);
    }
    println!("look:       {}", header.setup.look);
    let packed: u64 = header.payload.iter().map(|e| e.packed).sum();
    println!(
        "payload:    {} files, {} ({} packed)",
        header.payload.len(),
        mb(plan.payload_size()),
        mb(packed)
    );
    if let Some(license) = &header.setup.license {
        let accept = if license.accept {
            ", must be accepted"
        } else {
            ""
        };
        println!("license:    shown{accept}");
    }
    for c in &plan.choices {
        let state = match (c.required, c.default) {
            (true, _) => "required",
            (false, true) => "on",
            (false, false) => "off",
        };
        let size = if c.size > 0 {
            format!(", {}", mb(c.size))
        } else {
            String::new()
        };
        println!("component:  {} \"{}\" ({state}{size})", c.id, c.name);
    }
    for page in &plan.inputs {
        let fields: Vec<&str> = page.fields.iter().map(|f| f.id.as_str()).collect();
        println!(
            "page:       {} \"{}\": {}",
            page.id,
            page.title,
            fields.join(", ")
        );
    }
    for p in &plan.prerequisites {
        let admin = if p.admin { ", needs administrator" } else { "" };
        println!("first:      {} \"{}\"{admin}", p.id, p.name);
    }
    if let Some(feed) = &plan.update {
        println!("updates:    {feed}");
    }
    if !header.assets.is_empty() {
        let names: Vec<&str> = header.assets.iter().map(|a| a.name.as_str()).collect();
        println!("assets:     {}", names.join(", "));
    }
    let files = plan
        .operations
        .iter()
        .filter(|op| {
            matches!(
                op,
                Operation::InstallFile { .. } | Operation::CreateDirectory { .. }
            )
        })
        .count();
    println!(
        "steps:      {files} files and folders, {} more",
        plan.operations.len() - files
    );
    if operations {
        println!();
        for op in &plan.operations {
            println!("  {op}");
        }
    }
}
