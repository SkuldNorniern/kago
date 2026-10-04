use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::container::{LicenseText, Setup};
use kago::format::ico::{self, IconGroup};
use kago::format::setup;
use kago::install::{Sources, folder_name, hash};
use kago::manifest::Context;
use kago::package::{Artifact, Os, Target};
use kago::payload::Payload;

use super::args::Args;
use super::check::targets;
use super::plan::{self, Loaded};

/// builds the setup executable for one target.
pub fn run(args: &Args) -> Result<ExitCode, String> {
    let target = match targets(args)?.as_slice() {
        [one] => *one,
        _ => return Err("`build` takes one `--target`".to_owned()),
    };
    if !Artifact::Setup.supports(target.os()) || target.os() != Os::Windows {
        return Err(format!("setup builds for {target} are not supported yet"));
    }
    if Target::host() != Some(target) {
        return Err(format!(
            "a {target} setup needs a kago built for {target} as its runtime; build on that machine for now"
        ));
    }
    let Some((loaded, plan)) = plan::build(args, target)? else {
        return Ok(ExitCode::FAILURE);
    };
    let out_dir = PathBuf::from(args.value("--out").unwrap_or("dist"));
    fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let name = folder_name(&plan.name).replace(' ', "-");
    let out = out_dir.join(format!("{name}-{}-setup-{target}.exe", plan.version));

    let me = std::env::current_exe().map_err(|e| format!("cannot find kago itself: {e}"))?;
    let stub_tmp;
    let stub: &Path = match setup::embedded(&me).map_err(|e| e.to_string())? {
        // running from a setup exe: build from its runtime part only.
        Some(embedded) => {
            stub_tmp = out_dir.join(".kago-stub.exe");
            setup::extract_stub(&me, embedded, &stub_tmp).map_err(|e| e.to_string())?;
            &stub_tmp
        }
        None => &me,
    };
    let icon = icon(&loaded, target)?;
    let presentation = Setup {
        look: loaded.manifest.install.look,
        license: license(&loaded, target)?,
    };
    let size = setup::build(stub, &plan, &presentation, icon.as_ref(), &out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    if stub != me {
        let _ = fs::remove_file(stub);
    }
    let (_, digest) = hash(&out).map_err(|e| e.to_string())?;
    println!("{}", out.display());
    println!("  {size} bytes, payload {} bytes", plan.payload_size());
    println!("  sha256 {digest}");
    println!("  unsigned");
    Ok(ExitCode::SUCCESS)
}

/// the license text to show, read from the manifest's `license` file.
fn license(loaded: &Loaded, target: Target) -> Result<Option<LicenseText>, String> {
    let Some(license) = &loaded.manifest.license else {
        return Ok(None);
    };
    let path = Payload::new(&loaded.manifest, &loaded.base, target).locate(&license.file)?;
    let text = fs::read_to_string(&path).map_err(|e| format!("license {}: {e}", path.display()))?;
    Ok(Some(LicenseText {
        text,
        accept: license.require_acceptance,
    }))
}

/// the first `.ico` among `[[application.icons]]` that matches `target`.
fn icon(loaded: &Loaded, target: Target) -> Result<Option<IconGroup>, String> {
    let cx = Context::target(target);
    let Some(icon) = loaded
        .manifest
        .application
        .icons
        .iter()
        .find(|i| i.when.matches(&cx) && i.source.to_lowercase().ends_with(".ico"))
    else {
        return Ok(None);
    };
    let path = Payload::new(&loaded.manifest, &loaded.base, target).locate(&icon.source)?;
    let bytes = fs::read(&path).map_err(|e| format!("icon {}: {e}", path.display()))?;
    ico::parse(&bytes)
        .map(Some)
        .map_err(|e| format!("icon {}: {e}", path.display()))
}
