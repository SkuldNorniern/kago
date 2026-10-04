use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::install::{InstallPlan, Roots, Selection, resolve};
use crate::manifest::{Context, Manifest, Root};
use crate::platform::Registration;

/// platform that keeps PATH and registrations in memory and puts every
/// root under one temp dir. can be told to fail one kind of step.
struct Fake {
    base: PathBuf,
    path: RefCell<Vec<PathBuf>>,
    registered: RefCell<BTreeSet<String>>,
    fail_register: bool,
}

impl Fake {
    fn new(base: PathBuf) -> Self {
        Self {
            base,
            path: RefCell::new(vec![PathBuf::from("/already/there")]),
            registered: RefCell::new(BTreeSet::new()),
            fail_register: false,
        }
    }
}

impl Platform for Fake {
    fn roots(&self, _: Scope, name: &str) -> Result<Roots, String> {
        Ok(Roots(BTreeMap::from([
            (Root::App, self.base.join("programs").join(name)),
            (Root::Applications, self.base.join("start")),
            (Root::Config, self.base.join("config").join(name)),
        ])))
    }

    fn state_dir(&self, _: Scope) -> Result<PathBuf, String> {
        Ok(self.base.join("state"))
    }

    fn shortcut_file(&self, name: &str) -> String {
        format!("{name}.lnk")
    }

    fn add_path_entry(&self, _: Scope, entry: &Path) -> Result<bool, String> {
        let mut path = self.path.borrow_mut();
        if path.iter().any(|p| p == entry) {
            return Ok(false);
        }
        path.push(entry.to_path_buf());
        Ok(true)
    }

    fn remove_path_entry(&self, _: Scope, entry: &Path) -> Result<(), String> {
        self.path.borrow_mut().retain(|p| p != entry);
        Ok(())
    }

    fn create_shortcut(&self, link: &Path, target: &Path, _: &[String]) -> Result<(), String> {
        fs::write(link, target.display().to_string()).map_err(|e| e.to_string())
    }

    fn register(&self, _: Scope, r: &Registration) -> Result<String, String> {
        if self.fail_register {
            return Err("registry says no".into());
        }
        let key = format!("uninstall/{}", r.package);
        self.registered.borrow_mut().insert(key.clone());
        Ok(key)
    }

    fn unregister(&self, _: Scope, key: &str) -> Result<(), String> {
        self.registered.borrow_mut().remove(key);
        Ok(())
    }
}

const MANIFEST: &str = r#"
schema = 1

[package]
id = "dev.test.app"
name = "App"
version = "1.0.0"

[application]
entrypoint = "main"

[[files]]
logical_id = "main"
source = "payload/app.exe"
target = "{app}/app.exe"

[[files]]
source = "payload/themes"
target = "{app}/themes"

[[files]]
source = "payload/config.toml"
target = "{config}/config.toml"
ownership = "preserve-if-modified"

[[paths]]
resource = "main"

[[shortcuts]]
name = "App"
target = "main"
"#;

struct World {
    dir: PathBuf,
    fake: Fake,
    store: Store,
}

impl World {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kago-tx-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("payload/themes")).unwrap();
        fs::write(dir.join("payload/app.exe"), b"v1").unwrap();
        fs::write(dir.join("payload/themes/dark.toml"), b"dark").unwrap();
        fs::write(dir.join("payload/config.toml"), b"cfg").unwrap();
        fs::write(dir.join("kago.exe"), b"runtime").unwrap();
        let fake = Fake::new(dir.join("machine"));
        let store = Store::new(fake.state_dir(Scope::User).unwrap());
        Self { dir, fake, store }
    }

    fn plan(&self, manifest: &str) -> InstallPlan {
        let m = Manifest::parse("kago.toml", manifest).0.unwrap();
        let target = "windows-x86_64".parse().unwrap();
        let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
            .0
            .unwrap();
        InstallPlan::build(&m, &r, target, Scope::User, &self.dir)
            .0
            .unwrap()
    }

    fn app(&self) -> PathBuf {
        self.dir.join("machine/programs/App")
    }

    fn install(&self, manifest: &str) -> Result<Receipt, Error> {
        install(
            &self.plan(manifest),
            &self.fake,
            &self.store,
            &self.dir.join("kago.exe"),
        )
    }

    fn id() -> PackageId {
        PackageId::new("dev.test.app").unwrap()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn install_then_uninstall_leaves_nothing() {
    let w = World::new("roundtrip");
    let receipt = w.install(MANIFEST).unwrap();
    assert_eq!(fs::read(w.app().join("app.exe")).unwrap(), b"v1");
    assert!(w.app().join("themes/dark.toml").is_file());
    assert!(w.dir.join("machine/start/App.lnk").is_file());
    assert!(w.fake.path.borrow().contains(&w.app()));
    assert_eq!(w.fake.registered.borrow().len(), 1);
    assert!(w.store.runtime_path().is_file());
    assert_eq!(w.store.read_receipt(&World::id()).unwrap(), Some(receipt));
    assert!(w.store.journals().unwrap().is_empty());

    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(done.failed.is_empty(), "{:?}", done.failed);
    assert!(!w.app().exists());
    assert!(!w.dir.join("machine/config/App").exists());
    assert!(!w.dir.join("machine/start/App.lnk").exists());
    assert_eq!(*w.fake.path.borrow(), [PathBuf::from("/already/there")]);
    assert!(w.fake.registered.borrow().is_empty());
    assert_eq!(w.store.read_receipt(&World::id()).unwrap(), None);
    assert!(!w.store.lock_path(&World::id()).exists());
}

#[test]
fn failure_rolls_everything_back() {
    let mut w = World::new("rollback");
    w.fake.fail_register = true;
    let err = w.install(MANIFEST).unwrap_err();
    assert!(matches!(err, Error::RolledBack(_)), "{err}");
    assert!(!w.app().exists());
    assert!(!w.dir.join("machine/start/App.lnk").exists());
    assert_eq!(*w.fake.path.borrow(), [PathBuf::from("/already/there")]);
    assert_eq!(w.store.read_receipt(&World::id()).unwrap(), None);
    assert!(w.store.journals().unwrap().is_empty());
}

#[test]
fn failed_upgrade_restores_old_files() {
    let mut w = World::new("upgrade-fail");
    w.install(MANIFEST).unwrap();
    fs::write(w.dir.join("payload/app.exe"), b"v2").unwrap();
    w.fake.fail_register = true;
    w.install(&MANIFEST.replace("1.0.0", "1.1.0")).unwrap_err();
    assert_eq!(fs::read(w.app().join("app.exe")).unwrap(), b"v1");
    let receipt = w.store.read_receipt(&World::id()).unwrap().unwrap();
    assert_eq!(receipt.version.to_string(), "1.0.0");
}

#[test]
fn upgrade_removes_files_the_new_version_dropped() {
    let w = World::new("upgrade");
    w.install(MANIFEST).unwrap();
    fs::write(w.dir.join("payload/app.exe"), b"v2").unwrap();
    let without_themes = MANIFEST.replace(
        "[[files]]\nsource = \"payload/themes\"\ntarget = \"{app}/themes\"\n",
        "",
    );
    let receipt = w
        .install(&without_themes.replace("1.0.0", "1.1.0"))
        .unwrap();
    assert_eq!(receipt.version.to_string(), "1.1.0");
    assert_eq!(fs::read(w.app().join("app.exe")).unwrap(), b"v2");
    assert!(!w.app().join("themes").exists());
    assert_eq!(w.fake.registered.borrow().len(), 1);
}

#[test]
fn modified_config_survives_uninstall() {
    let w = World::new("preserve");
    w.install(MANIFEST).unwrap();
    let config = w.dir.join("machine/config/App/config.toml");
    fs::write(&config, b"mine").unwrap();
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(fs::read(&config).unwrap(), b"mine");
    assert!(done.kept.iter().any(|k| k.ends_with("config.toml")));
    assert!(!w.app().exists());
}

#[test]
fn crash_is_rolled_back_on_next_run() {
    let w = World::new("crash");
    // a half-done install: one file placed, then the process died.
    let file = w.app().join("app.exe");
    fs::create_dir_all(w.app()).unwrap();
    let mut j = journal::Journal::begin(
        &w.store.journal_path(&World::id()),
        &World::id(),
        &crate::package::Version::new(1, 0, 0),
    )
    .unwrap();
    j.step(Undo::RemoveDirectory { path: w.app() }).unwrap();
    j.step(Undo::RemoveFile { path: file.clone() }).unwrap();
    fs::write(&file, b"half").unwrap();
    drop(j);

    let recovered = recover(&w.fake, &w.store).unwrap();
    assert_eq!(recovered, [Recovered::RolledBack(World::id())]);
    assert!(!w.app().exists());
    assert!(w.store.journals().unwrap().is_empty());
}

#[test]
fn second_process_is_locked_out() {
    let w = World::new("lock");
    let _held = lock(&w.store, &World::id()).unwrap();
    assert!(matches!(w.install(MANIFEST), Err(Error::Locked(_))));
}

#[test]
fn downgrade_is_refused_unless_allowed() {
    let w = World::new("downgrade");
    w.install(MANIFEST).unwrap();
    let older = MANIFEST.replace("1.0.0", "0.9.0");
    let err = w.install(&older).unwrap_err();
    assert!(matches!(err, Error::Downgrade { .. }), "{err}");
    assert_eq!(
        w.store
            .read_receipt(&World::id())
            .unwrap()
            .unwrap()
            .version
            .to_string(),
        "1.0.0"
    );
    // same version again is a reinstall, not a downgrade.
    w.install(MANIFEST).unwrap();
    let allowed = older.replace(
        "[application]",
        "[install]
downgrade = \"allow\"

[application]",
    );
    assert_eq!(w.install(&allowed).unwrap().version.to_string(), "0.9.0");
}
