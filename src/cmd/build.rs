use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::container::{LicenseText, Setup, SetupBrand};
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
    let Some((loaded, plan)) = plan::build(args, target, true)? else {
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
    let brand = &loaded.manifest.brand;
    let presentation = Setup {
        look: loaded.manifest.install.look,
        license: license(&loaded, target)?,
        folder: loaded.manifest.install.folder,
        brand: SetupBrand {
            text: brand.text.clone(),
            accent: brand.accent,
            welcome: brand.welcome.clone(),
            finish: brand.finish.clone(),
            link: brand.link.clone(),
        },
    };
    let assets = brand_images(&loaded, target, icon.as_ref())?;
    let size = setup::build(stub, &plan, &presentation, &assets, icon.as_ref(), &out)
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

/// the brand's PNGs as setup assets, each checked to decode. without a
/// `logo`, the app icon's PNG image stands in when it has one.
fn brand_images(
    loaded: &Loaded,
    target: Target,
    icon: Option<&IconGroup>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let brand = &loaded.manifest.brand;
    let payload = Payload::new(&loaded.manifest, &loaded.base, target);
    let mut out = Vec::new();
    for (name, reference) in [
        ("logo", &brand.logo),
        ("header", &brand.header),
        ("side", &brand.side),
    ] {
        let Some(reference) = reference else {
            continue;
        };
        let path = payload.locate(reference)?;
        let bytes = fs::read(&path).map_err(|e| format!("brand {name} {}: {e}", path.display()))?;
        kago::format::image::decode_png(&bytes)
            .map_err(|e| format!("brand {name} {}: {e}", path.display()))?;
        out.push((name.to_owned(), bytes));
    }
    if brand.logo.is_none()
        && let Some(png) = icon.and_then(kago::format::image::png_in_icon)
    {
        out.push(("logo".to_owned(), png.to_vec()));
    }
    Ok(out)
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
