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
use kago::sign::Signer;
use kago::update::Feed;

use super::args::Args;
use super::check::targets;
use super::kagofile::{self, Loaded};

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
    let Some((loaded, mut plan)) = kagofile::build(args, target, true)? else {
        return Ok(ExitCode::FAILURE);
    };
    let signer = match &loaded.manifest.sign {
        Some(sign) if !args.flag("--no-sign") => {
            Some((Signer::new(sign, &loaded.base)?, sign.programs))
        }
        _ => None,
    };
    if let Some(warning) = icon_warning(&loaded, &plan) {
        eprintln!("warning: {warning}");
    }
    for warning in runtime_warnings(&plan) {
        eprintln!("warning: {warning}");
    }
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
    let scratch = out_dir.join(".kago-sign");
    if let Some((signer, true)) = &signer {
        sign_programs(signer, &mut plan, &scratch)?;
    }
    let stub_bytes = setup::prepare_stub(stub, icon.as_ref(), &out.with_extension("stub"))
        .map_err(|e| format!("{}: {e}", out.display()))?;
    let mut presentation = presentation(&loaded, target)?;
    let mut assets = brand_images(&loaded, target, icon.as_ref())?;
    pick_accent(&mut presentation, &mut plan, &assets);
    for p in &plan.prerequisites {
        let bytes = fs::read(&p.source).map_err(|e| format!("{}: {e}", p.source.display()))?;
        assets.push((p.asset(), bytes));
    }
    let homepage = loaded.manifest.package.homepage.as_deref();
    if let Some((signer, _)) = &signer {
        assets.extend(signed_runtimes(
            signer,
            &stub_bytes,
            &plan.name,
            homepage,
            &scratch,
        )?);
    }
    let size = setup::build(&stub_bytes, &plan, &presentation, &assets, &out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    if stub != me {
        let _ = fs::remove_file(stub);
    }
    let signature = match &signer {
        Some((signer, _)) => {
            signer.sign(&out, &format!("{} Setup", plan.name), homepage)?;
            Some(signer.trusted(&out))
        }
        None => None,
    };
    let _ = fs::remove_dir_all(&scratch);
    let (bytes, digest) = hash(&out).map_err(|e| e.to_string())?;
    if loaded.manifest.update.is_some() {
        write_feed(&out, &plan, target, (bytes, &digest.to_string()))?;
    }
    println!("{}", out.display());
    println!("  {size} bytes, payload {} bytes", plan.payload_size());
    println!("  sha256 {digest}");
    println!(
        "  {}",
        match signature {
            None => "unsigned",
            Some(true) => "signed",
            Some(false) => "signed, but Windows does not trust the certificate (a test one?)",
        }
    );
    Ok(ExitCode::SUCCESS)
}

/// writes `latest.json` next to the setup at `out`, merged into the one
/// already there when that is for the same version, so one file lists the
/// setup of every target.
fn write_feed(
    out: &Path,
    plan: &kago::install::InstallPlan,
    target: Target,
    (size, sha256): (u64, &str),
) -> Result<(), String> {
    let path = out.with_file_name(kago::update::FEED);
    let file = out.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let mut feed = Feed::new(
        plan.package.clone(),
        plan.version.clone(),
        target,
        kago::update::Setup {
            file: file.to_owned(),
            sha256: sha256.to_owned(),
            size,
        },
    );
    let path = path.as_path();
    let old = fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Feed>(&t).ok());
    if let Some(old) = old
        && old.package == feed.package
        && old.version == feed.version
    {
        for (target, setup) in old.setups {
            feed.setups.entry(target).or_insert(setup);
        }
    }
    let text = serde_json::to_string_pretty(&feed).map_err(|e| e.to_string())?;
    fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    println!("  {}: upload it with the setup", path.display());
    Ok(())
}

/// without an accent, the brand's own pictures give one, so buttons match
/// them in the setup and in the uninstaller it leaves.
fn pick_accent(
    presentation: &mut Setup,
    plan: &mut kago::install::InstallPlan,
    assets: &[(String, Vec<u8>)],
) {
    if presentation.brand.accent.is_some() {
        return;
    }
    let tone = ["banner", "side", "logo"].into_iter().find_map(|name| {
        let (_, png) = assets.iter().find(|(n, _)| n == name)?;
        kago::format::image::decode_png(png).ok()?.tone()
    });
    presentation.brand.accent = tone;
    if let Some(look) = &mut plan.look {
        look.accent = look.accent.or(tone);
    }
}

/// how the setup window looks and what it asks.
fn presentation(loaded: &Loaded, target: Target) -> Result<Setup, String> {
    let brand = &loaded.manifest.brand;
    Ok(Setup {
        look: loaded.manifest.install.look,
        license: license(loaded, target)?,
        folder: loaded.manifest.install.folder,
        brand: SetupBrand {
            text: brand.text.clone(),
            accent: brand.accent,
            welcome: brand.welcome.clone(),
            finish: brand.finish.clone(),
            link: brand.link.clone(),
        },
        description: loaded.manifest.package.description.clone(),
    })
}

/// signs the package's own `.exe` and `.dll` files that are not signed
/// yet. copies are signed, under `scratch`: the build's files stay as
/// they are.
fn sign_programs(
    signer: &Signer,
    plan: &mut kago::install::InstallPlan,
    scratch: &Path,
) -> Result<(), String> {
    let name = plan.name.clone();
    for (i, op) in plan.operations.iter_mut().enumerate() {
        let kago::install::Operation::InstallFile {
            source,
            size,
            sha256,
            ..
        } = op
        else {
            continue;
        };
        let program = source
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("dll"));
        if !program {
            continue;
        }
        let bytes = fs::read(&*source).map_err(|e| format!("{}: {e}", source.display()))?;
        if kago::format::pe::is_signed(&bytes) {
            continue;
        }
        let file = source.file_name().unwrap_or_default();
        let copy = scratch.join(i.to_string()).join(file);
        fs::create_dir_all(copy.parent().unwrap_or(scratch))
            .and_then(|()| fs::write(&copy, &bytes))
            .map_err(|e| format!("{}: {e}", copy.display()))?;
        signer.sign(&copy, &name, None)?;
        (*size, *sha256) = hash(&copy).map_err(|e| e.to_string())?;
        println!("  signed {}", source.display());
        *source = copy;
    }
    Ok(())
}

/// the runtime signed twice over, as a console program for
/// `QuietUninstallString` and as the windowed `uninstall.exe`: a signed
/// file cannot be patched later, so the install takes these as they are.
fn signed_runtimes(
    signer: &Signer,
    stub: &[u8],
    name: &str,
    homepage: Option<&str>,
    scratch: &Path,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    fs::create_dir_all(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let mut out = Vec::new();
    for (asset, windowed) in [
        (setup::RUNTIME_ASSET, false),
        (setup::UNINSTALLER_ASSET, true),
    ] {
        let mut bytes = stub.to_vec();
        if windowed {
            setup::make_windowed(&mut bytes);
        } else {
            setup::make_console(&mut bytes);
        }
        let file = scratch.join(format!("{asset}.exe"));
        fs::write(&file, &bytes).map_err(|e| format!("{}: {e}", file.display()))?;
        signer.sign(&file, &format!("{name} Uninstall"), homepage)?;
        let signed_bytes = fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        out.push((asset.to_owned(), signed_bytes));
    }
    Ok(out)
}

/// the main program shows a blank icon in the Start menu, the taskbar and
/// Settings when it has no icon of its own and the Kagofile gives none.
fn icon_warning(loaded: &Loaded, plan: &kago::install::InstallPlan) -> Option<String> {
    use kago::install::Operation;

    let has_ico = plan
        .operations
        .iter()
        .any(|op| matches!(op, Operation::RegisterApplication { icon: Some(_), .. }));
    if has_ico {
        return None;
    }
    let entry = plan.operations.iter().find_map(|op| match op {
        Operation::RegisterApplication {
            entrypoint: Some(e),
            ..
        } => Some(e),
        _ => None,
    })?;
    let source = plan.operations.iter().find_map(|op| match op {
        Operation::InstallFile { source, target, .. } if target == entry => Some(source),
        _ => None,
    })?;
    let bytes = fs::read(source).ok()?;
    if kago::format::pe::has_icon(&bytes) {
        return None;
    }
    let app = loaded
        .manifest
        .application
        .entrypoint
        .as_deref()
        .unwrap_or("app");
    Some(format!(
        "{} has no icon, so its shortcuts and Settings entry will show a blank one; add `icon windows <file>.ico` under `app {app}`",
        entry.segments().last().unwrap_or_default()
    ))
}

/// programs in the payload that need the Visual C++ runtime, which a fresh
/// Windows does not have, unless the payload ships that DLL itself.
fn runtime_warnings(plan: &kago::install::InstallPlan) -> Vec<String> {
    use kago::format::pe;
    use kago::install::Operation;

    let files: Vec<(&Path, String)> = plan
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::InstallFile { source, target, .. } => {
                let name = target
                    .segments()
                    .last()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                Some((source.as_path(), name))
            }
            _ => None,
        })
        .collect();
    let shipped: Vec<&str> = files.iter().map(|(_, name)| name.as_str()).collect();
    // the library's `vcredist` action installs the runtime first.
    if plan.prerequisites.iter().any(|p| p.id == "vcredist") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (source, name) in &files {
        let program = Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("dll"));
        if !program {
            continue;
        }
        let Ok(bytes) = fs::read(source) else {
            continue;
        };
        let imports = pe::imports(&bytes);
        let missing: Vec<&str> = pe::needs_vc_runtime(&imports)
            .into_iter()
            .filter(|dll| !shipped.contains(dll))
            .collect();
        if !missing.is_empty() {
            out.push(format!(
                "{name} needs {}, the Visual C++ runtime, which a fresh Windows does not have; link the runtime in (Rust: `-C target-feature=+crt-static` in .cargo/config.toml, C++: `/MT`), ship the DLL next to it, or add `vcredist <vc_redist.x64.exe>` to install it first",
                missing.join(", ")
            ));
        }
    }
    out
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
        ("banner", &brand.banner),
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
        && let Some(png) = icon.and_then(kago::format::image::logo_from_icon)
    {
        out.push(("logo".to_owned(), png));
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
