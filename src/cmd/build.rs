use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::format::setup;
use kago::install::{folder_name, hash};
use kago::package::{Artifact, Os, Target};

use super::args::Args;
use super::check::targets;
use super::plan;

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
    let Some(plan) = plan::build(args, target)? else {
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
    let size = setup::build(stub, &plan, &out).map_err(|e| format!("{}: {e}", out.display()))?;
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
