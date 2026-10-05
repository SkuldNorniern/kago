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
    steamvr: RefCell<Vec<PathBuf>>,
    fail_register: bool,
    has_steamvr: bool,
    login: RefCell<BTreeMap<String, String>>,
    /// what was left for the next restart, in order.
    at_restart: RefCell<Vec<PathBuf>>,
    firewall: RefCell<BTreeMap<String, crate::platform::Firewall>>,
}

impl Fake {
    fn new(base: PathBuf) -> Self {
        Self {
            base,
            path: RefCell::new(vec![PathBuf::from("/already/there")]),
            registered: RefCell::new(BTreeSet::new()),
            steamvr: RefCell::new(vec![PathBuf::from("/other/driver")]),
            fail_register: false,
            has_steamvr: false,
            login: RefCell::new(BTreeMap::new()),
            at_restart: RefCell::new(Vec::new()),
            firewall: RefCell::new(BTreeMap::new()),
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

    fn login_item(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.login.borrow().get(name).cloned())
    }

    fn add_login_item(
        &self,
        name: &str,
        exe: &Path,
        arguments: &[String],
    ) -> Result<String, String> {
        let command = format!("{} {}", exe.display(), arguments.join(" "));
        self.login
            .borrow_mut()
            .insert(name.to_owned(), command.clone());
        Ok(command)
    }

    fn set_login_item(&self, name: &str, command: Option<&str>) -> Result<(), String> {
        let mut items = self.login.borrow_mut();
        match command {
            Some(c) => items.insert(name.to_owned(), c.to_owned()),
            None => items.remove(name),
        };
        Ok(())
    }

    fn os_version(&self) -> Option<String> {
        Some("10.0.19045".into())
    }

    fn has(&self, detect: crate::manifest::Detect) -> bool {
        detect == crate::manifest::Detect::SteamVr && self.has_steamvr
    }

    fn add_steamvr_driver(&self, directory: &Path) -> Result<bool, String> {
        let mut drivers = self.steamvr.borrow_mut();
        if drivers.iter().any(|d| d == directory) {
            return Ok(false);
        }
        drivers.push(directory.to_path_buf());
        Ok(true)
    }

    fn remove_steamvr_driver(&self, directory: &Path) -> Result<(), String> {
        self.steamvr.borrow_mut().retain(|d| d != directory);
        Ok(())
    }

    fn firewall_rule(&self, name: &str) -> Result<Option<crate::platform::Firewall>, String> {
        Ok(self.firewall.borrow().get(name).cloned())
    }

    fn set_firewall_rule(
        &self,
        name: &str,
        _: &str,
        rule: Option<&crate::platform::Firewall>,
    ) -> Result<(), String> {
        let mut rules = self.firewall.borrow_mut();
        match rule {
            Some(rule) => rules.insert(name.to_owned(), rule.clone()),
            None => rules.remove(name),
        };
        Ok(())
    }

    fn delete_at_restart(&self, path: &Path) -> Result<(), String> {
        self.at_restart.borrow_mut().push(path.to_path_buf());
        Ok(())
    }

    fn create_shortcut(
        &self,
        link: &Path,
        target: &Path,
        _: &[String],
        _: Option<&Path>,
    ) -> Result<(), String> {
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

const MANIFEST: &str = r"
package dev.test.app
  name App
  version 1.0.0

app main
  windows payload/app.exe

copy payload/themes
copy payload/config.toml -> {config}/config.toml keep
path
shortcut App
";

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
        let m = Manifest::parse("Kagofile", manifest).0.unwrap();
        let target = "windows-x86_64".parse().unwrap();
        let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
            .0
            .unwrap();
        InstallPlan::build(&m, &r, target, Scope::User, self.dir.as_path())
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
    let uninstaller = w
        .app()
        .join(format!("uninstall{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(
        fs::read(&uninstaller).unwrap(),
        b"runtime",
        "the runtime, as the uninstaller"
    );
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
fn what_stays_in_use_goes_at_restart_files_first() {
    let w = World::new("restart");
    w.install(MANIFEST).unwrap();
    let scheduled = remove_at_restart(&World::id(), &w.fake, &w.store).unwrap();
    let order = w.fake.at_restart.borrow();
    assert_eq!(order.len(), scheduled.len());
    let app = order.iter().position(|p| *p == w.app()).unwrap();
    let file = order
        .iter()
        .position(|p| *p == w.app().join("app.exe"))
        .unwrap();
    assert!(file < app, "the folder after its files");
    // files and folders are no longer the package's; the rest stays.
    let left = w.store.read_receipt(&World::id()).unwrap().unwrap();
    assert!(
        left.resources
            .iter()
            .all(|r| !matches!(r, Resource::File { .. } | Resource::Directory { .. })),
        "{:?}",
        left.resources
    );
}

#[test]
fn firewall_rules_come_and_go_with_installs_for_everyone() {
    let w = World::new("firewall");
    let manifest = MANIFEST.replace("  version 1.0.0\n", "  version 1.0.0\n  scope choose\n")
        + "firewall main public\n";
    let mut plan = w.plan(&manifest);
    plan.scope = Scope::Machine;
    install(&plan, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap();
    let rule = w.fake.firewall.borrow().get("App").cloned().unwrap();
    assert_eq!(rule.program, w.app().join("app.exe"));
    assert!(rule.public);
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(w.fake.firewall.borrow().is_empty());

    // just for this user: no rights to add one, so none is added.
    plan.scope = Scope::User;
    install(&plan, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap();
    assert!(w.fake.firewall.borrow().is_empty());
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
        "copy payload/themes
",
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
        "  version 0.9.0",
        "  version 0.9.0
  downgrade allow",
    );
    assert_eq!(w.install(&allowed).unwrap().version.to_string(), "0.9.0");
}

#[test]
fn verify_finds_missing_and_modified() {
    let w = World::new("verify");
    let receipt = w.install(MANIFEST).unwrap();
    assert!(verify(&receipt).is_healthy());
    fs::write(w.app().join("app.exe"), b"tampered").unwrap();
    fs::remove_file(w.app().join("themes/dark.toml")).unwrap();
    fs::write(w.dir.join("machine/config/App/config.toml"), b"mine").unwrap();
    let v = verify(&receipt);
    assert_eq!(v.modified.len(), 1, "{v:?}");
    assert!(v.modified[0].ends_with("app.exe"));
    assert_eq!(v.missing.len(), 1, "{v:?}");
    assert!(v.missing[0].ends_with("dark.toml"));
    // repair is the same version again.
    let again = w.install(MANIFEST).unwrap();
    assert!(verify(&again).modified.is_empty());
    assert!(verify(&again).missing.is_empty());
}

#[test]
fn modified_config_survives_upgrade() {
    let w = World::new("keep-config");
    w.install(MANIFEST).unwrap();
    let config = w.dir.join("machine/config/App/config.toml");
    fs::write(&config, b"mine").unwrap();
    fs::write(w.dir.join("payload/config.toml"), b"cfg v2").unwrap();
    w.install(&MANIFEST.replace("1.0.0", "1.1.0")).unwrap();
    assert_eq!(fs::read(&config).unwrap(), b"mine");
    // still kept by uninstall after the upgrade.
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(fs::read(&config).unwrap(), b"mine");
}

#[test]
fn unmodified_config_is_upgraded() {
    let w = World::new("upgrade-config");
    w.install(MANIFEST).unwrap();
    fs::write(w.dir.join("payload/config.toml"), b"cfg v2").unwrap();
    w.install(&MANIFEST.replace("1.0.0", "1.1.0")).unwrap();
    assert_eq!(
        fs::read(w.dir.join("machine/config/App/config.toml")).unwrap(),
        b"cfg v2"
    );
}

#[test]
fn upgrade_then_uninstall_leaves_nothing() {
    let w = World::new("upgrade-uninstall");
    w.install(MANIFEST).unwrap();
    fs::write(w.dir.join("payload/app.exe"), b"v2").unwrap();
    w.install(&MANIFEST.replace("1.0.0", "1.1.0")).unwrap();
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(done.failed.is_empty() && done.kept.is_empty(), "{done:?}");
    assert!(!w.app().exists());
    assert!(!w.dir.join("machine/config/App").exists());
    assert!(!w.store.backup_dir(&World::id()).exists());
}

#[test]
fn every_operation_is_observed_in_order() {
    let w = World::new("observed");
    let plan = w.plan(MANIFEST);
    let mut seen = Vec::new();
    super::install_observed(
        &plan,
        &w.fake,
        &w.store,
        &w.dir.join("kago.exe"),
        &mut |i, n, op| seen.push((i, n, op.clone())),
    )
    .unwrap();
    let expected: Vec<_> = plan
        .operations
        .iter()
        .enumerate()
        .map(|(i, op)| (i, plan.operations.len(), op.clone()))
        .collect();
    assert_eq!(seen, expected);
}

#[test]
fn steamvr_driver_is_registered_kept_on_upgrade_and_removed() {
    let mut w = World::new("steamvr");
    fs::create_dir_all(w.dir.join("payload/driver/aira/bin/win64")).unwrap();
    fs::write(
        w.dir.join("payload/driver/aira/driver.vrdrivermanifest"),
        b"{}",
    )
    .unwrap();
    fs::write(
        w.dir.join("payload/driver/aira/bin/win64/driver_aira.dll"),
        b"dll",
    )
    .unwrap();
    let manifest = format!("{MANIFEST}steamvr payload/driver/aira\n");
    let driver = w.app().join("steamvr").join("aira");
    let others = || vec![PathBuf::from("/other/driver")];

    w.fake.fail_register = true;
    w.install(&manifest).unwrap_err();
    assert_eq!(*w.fake.steamvr.borrow(), others(), "rolled back");

    w.fake.fail_register = false;
    w.install(&manifest).unwrap();
    assert!(driver.join("bin/win64/driver_aira.dll").is_file());
    assert_eq!(
        *w.fake.steamvr.borrow(),
        [others(), vec![driver.clone()]].concat()
    );

    w.install(&manifest.replace("1.0.0", "1.1.0")).unwrap();
    let receipt = w.store.read_receipt(&World::id()).unwrap().unwrap();
    assert!(
        receipt
            .resources
            .iter()
            .any(|r| matches!(r, Resource::SteamVrDriver { .. })),
        "upgrade still owns the driver"
    );
    assert_eq!(w.fake.steamvr.borrow().len(), 2);

    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(*w.fake.steamvr.borrow(), others());
    assert!(!driver.exists());
}

#[test]
fn steamvr_driver_without_manifest_is_refused_at_build() {
    let w = World::new("steamvr-bad");
    fs::create_dir_all(w.dir.join("payload/notadriver")).unwrap();
    fs::write(w.dir.join("payload/notadriver/x.dll"), b"dll").unwrap();
    let m = Manifest::parse(
        "Kagofile",
        &format!("{MANIFEST}steamvr payload/notadriver\n"),
    )
    .0
    .unwrap();
    let target = "windows-x86_64".parse().unwrap();
    let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
        .0
        .unwrap();
    let (plan, d) = InstallPlan::build(&m, &r, target, Scope::User, w.dir.as_path());
    assert!(plan.is_none());
    assert_eq!(
        d.iter().map(|d| d.code).collect::<Vec<_>>(),
        [crate::diagnostic::INSTALL_SOURCE]
    );
}

#[test]
fn path_entry_survives_reinstall_and_upgrade() {
    let w = World::new("path-upgrade");
    w.install(MANIFEST).unwrap();
    w.install(MANIFEST).unwrap();
    assert!(
        w.fake.path.borrow().contains(&w.app()),
        "PATH lost on reinstall"
    );
    w.install(&MANIFEST.replace("1.0.0", "1.1.0")).unwrap();
    assert!(
        w.fake.path.borrow().contains(&w.app()),
        "PATH lost on upgrade"
    );
    let receipt = w.store.read_receipt(&World::id()).unwrap().unwrap();
    assert!(
        receipt
            .resources
            .iter()
            .any(|r| matches!(r, Resource::PathEntry { .. }))
    );
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(*w.fake.path.borrow(), [PathBuf::from("/already/there")]);
}

#[test]
fn chosen_folder_is_kept_on_upgrade_and_cleaned_on_uninstall() {
    let w = World::new("location");
    let custom = w.dir.join("elsewhere").join("App");
    install(
        &w.plan(MANIFEST).at(&custom),
        &w.fake,
        &w.store,
        &w.dir.join("kago.exe"),
    )
    .unwrap();
    assert_eq!(fs::read(custom.join("app.exe")).unwrap(), b"v1");
    assert!(!w.app().exists(), "nothing in the default folder");
    assert!(w.fake.path.borrow().contains(&custom));

    // an upgrade without a folder stays where the install is.
    w.install(&MANIFEST.replace("1.0.0", "1.1.0")).unwrap();
    assert!(custom.join("app.exe").is_file());
    assert!(!w.app().exists());

    // a new folder moves it.
    let moved = w.dir.join("moved");
    install(
        &w.plan(&MANIFEST.replace("1.0.0", "1.2.0")).at(&moved),
        &w.fake,
        &w.store,
        &w.dir.join("kago.exe"),
    )
    .unwrap();
    assert!(moved.join("app.exe").is_file());
    assert!(!custom.exists(), "old folder cleaned up");
    assert!(!w.fake.path.borrow().contains(&custom));

    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(!moved.exists());
}

#[test]
fn unmet_needs_stop_before_anything_changes() {
    let mut w = World::new("needs");
    let err = w
        .install(&format!(
            "{MANIFEST}require steamvr\nrequire windows 99.0\n"
        ))
        .unwrap_err();
    assert!(
        matches!(&err, Error::Unmet(needs) if needs == &["SteamVR", "Windows 99.0 or newer"]),
        "{err}"
    );
    assert!(!w.app().exists());
    assert!(w.store.journals().unwrap().is_empty());

    w.fake.has_steamvr = true;
    w.install(&format!(
        "{MANIFEST}require steamvr\nrequire windows 10.0.19041\n"
    ))
    .unwrap();
    assert!(w.app().join("app.exe").is_file());
}

/// a world whose app is this test binary, so hooks have a real program:
/// `--list` exits 0, an unknown flag fails.
fn hook_world(name: &str) -> World {
    let w = World::new(name);
    fs::copy(
        std::env::current_exe().unwrap(),
        w.dir.join("payload/app.exe"),
    )
    .unwrap();
    w
}

#[test]
fn post_install_hook_runs_and_failure_rolls_back() {
    let w = hook_world("hook-post");
    w.install(&format!("{MANIFEST}hook post-install main --list\n"))
        .unwrap();
    assert!(w.app().join("app.exe").is_file());
    uninstall(&World::id(), &w.fake, &w.store).unwrap();

    let err = w
        .install(&format!(
            "{MANIFEST}hook post-install main --no-such-flag\n"
        ))
        .unwrap_err();
    assert!(matches!(err, Error::RolledBack(_)), "{err}");
    assert!(!w.app().exists());
}

#[test]
fn pre_uninstall_hook_is_kept_for_uninstall_and_skipped_on_upgrade() {
    let w = hook_world("hook-pre");
    let manifest = format!("{MANIFEST}hook pre-uninstall main --list\n");
    let receipt = w.install(&manifest).unwrap();
    assert!(receipt.resources.iter().any(
        |r| matches!(r, Resource::UninstallHook { arguments, .. } if arguments == &["--list"])
    ));
    // dropping the hook in an update must not run it.
    w.install(&MANIFEST.replace("1.0.0", "1.1.0")).unwrap();
    assert!(w.app().join("app.exe").is_file());
    // a broken uninstall hook does not keep the package installed.
    w.install(
        &format!("{MANIFEST}hook pre-uninstall main --no-such-flag\n").replace("1.0.0", "1.2.0"),
    )
    .unwrap();
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(done.failed.is_empty(), "{:?}", done.failed);
    assert!(!w.app().exists());
}

#[test]
fn service_starts_at_sign_in_and_survives_a_failed_upgrade() {
    let mut w = World::new("service");
    let manifest = format!("{MANIFEST}service main --background\n");
    let receipt = w.install(&manifest).unwrap();
    let name = "dev.test.app.main";
    let command = w.fake.login.borrow().get(name).cloned().unwrap();
    assert!(command.ends_with("app.exe --background"), "{command}");
    assert!(
        receipt
            .resources
            .iter()
            .any(|r| matches!(r, Resource::LoginItem { name: n } if n == name))
    );

    // a failed upgrade puts the old entry back.
    w.fake.fail_register = true;
    w.install(
        &manifest
            .replace("--background", "--new")
            .replace("1.0.0", "1.1.0"),
    )
    .unwrap_err();
    assert_eq!(w.fake.login.borrow().get(name), Some(&command));

    w.fake.fail_register = false;
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(w.fake.login.borrow().is_empty());
}
