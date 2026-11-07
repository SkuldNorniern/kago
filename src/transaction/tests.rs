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
    has_steamvr: bool,
    login: RefCell<BTreeMap<String, String>>,
    /// what was left for the next restart, in order.
    at_restart: RefCell<Vec<PathBuf>>,
    firewall: RefCell<BTreeMap<String, crate::platform::Firewall>>,
    services: RefCell<BTreeMap<String, crate::platform::WindowsService>>,
    /// services stopped and set, in order, with what the program file
    /// held when one was set.
    service_calls: RefCell<Vec<String>>,
    /// registry values and environment variables; JSON goes to real files
    /// under `base`.
    values: RefCell<BTreeMap<String, crate::manifest::Value>>,
    /// driver store: published name to the `.inf` text it holds.
    drivers: RefCell<BTreeMap<String, String>>,
    /// devices: instance id to hardware id.
    devices: RefCell<BTreeMap<String, String>>,
}

impl Fake {
    fn new(base: PathBuf) -> Self {
        Self {
            base,
            path: RefCell::new(vec![PathBuf::from("/already/there")]),
            registered: RefCell::new(BTreeSet::new()),
            fail_register: false,
            has_steamvr: false,
            login: RefCell::new(BTreeMap::new()),
            at_restart: RefCell::new(Vec::new()),
            firewall: RefCell::new(BTreeMap::new()),
            services: RefCell::new(BTreeMap::new()),
            service_calls: RefCell::new(Vec::new()),
            values: RefCell::new(BTreeMap::new()),
            drivers: RefCell::new(BTreeMap::new()),
            devices: RefCell::new(BTreeMap::new()),
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

    fn satisfied(&self, _: &crate::manifest::Check) -> bool {
        self.has_steamvr
    }

    fn value(&self, slot: &crate::install::Slot) -> Result<Option<crate::manifest::Value>, String> {
        if let crate::install::Slot::Json { file, key } = slot {
            return crate::platform::json_value(file, key);
        }
        Ok(self.values.borrow().get(&slot.to_string()).cloned())
    }

    fn set_value(
        &self,
        slot: &crate::install::Slot,
        value: Option<&crate::manifest::Value>,
    ) -> Result<(), String> {
        if let crate::install::Slot::Json { file, key } = slot {
            return crate::platform::set_json_value(file, key, value);
        }
        let mut values = self.values.borrow_mut();
        match value {
            Some(v) => values.insert(slot.to_string(), v.clone()),
            None => values.remove(&slot.to_string()),
        };
        Ok(())
    }

    fn service(&self, name: &str) -> Result<Option<crate::platform::WindowsService>, String> {
        Ok(self.services.borrow().get(name).cloned())
    }

    fn set_service(
        &self,
        name: &str,
        service: Option<&crate::platform::WindowsService>,
    ) -> Result<(), String> {
        let held = service
            .and_then(|s| fs::read_to_string(s.command_line.trim_matches('"')).ok())
            .unwrap_or_default();
        self.service_calls
            .borrow_mut()
            .push(format!("set {name} {held}"));
        let mut services = self.services.borrow_mut();
        match service {
            Some(service) => services.insert(name.to_owned(), service.clone()),
            None => services.remove(name),
        };
        Ok(())
    }

    fn stop_service(&self, name: &str) -> Result<(), String> {
        self.service_calls.borrow_mut().push(format!("stop {name}"));
        Ok(())
    }

    fn firewall_rule(
        &self,
        name: &str,
        _: &str,
    ) -> Result<Option<crate::platform::Firewall>, String> {
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

    fn add_driver(&self, inf: &Path) -> Result<crate::platform::DriverAdded, String> {
        let text = fs::read_to_string(inf).map_err(|e| e.to_string())?;
        let mut store = self.drivers.borrow_mut();
        if let Some((published, _)) = store.iter().find(|(_, t)| **t == text) {
            return Ok(crate::platform::DriverAdded {
                published: published.clone(),
                new: false,
            });
        }
        let published = format!("oem{}.inf", store.len() + 10);
        store.insert(published.clone(), text);
        Ok(crate::platform::DriverAdded {
            published,
            new: true,
        })
    }

    fn remove_driver(&self, published: &str) -> Result<(), String> {
        self.drivers.borrow_mut().remove(published);
        Ok(())
    }

    fn device(&self, id: &str) -> Result<Option<String>, String> {
        Ok(self
            .devices
            .borrow()
            .iter()
            .find(|(_, h)| h.eq_ignore_ascii_case(id))
            .map(|(i, _)| i.clone()))
    }

    fn create_device(&self, id: &str, _: &Path) -> Result<String, String> {
        let mut devices = self.devices.borrow_mut();
        let instance = format!("ROOT\\DEV\\{:04}", devices.len());
        devices.insert(instance.clone(), id.to_owned());
        Ok(instance)
    }

    fn update_devices(&self, _: &str, _: &Path) -> Result<(), String> {
        Ok(())
    }

    fn remove_device(&self, instance: &str) -> Result<(), String> {
        self.devices.borrow_mut().remove(instance);
        Ok(())
    }

    /// a folder named `Shared` stands for one like the user's Documents.
    fn shared_folder(&self, dir: &Path) -> bool {
        dir.file_name().is_some_and(|n| n == "Shared")
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
        self.plan_in(manifest, Scope::User)
    }

    fn plan_in(&self, manifest: &str, scope: Scope) -> InstallPlan {
        let m = Manifest::parse("Kagofile", manifest).0.unwrap();
        let target = "windows-x86_64".parse().unwrap();
        let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
            .0
            .unwrap();
        let (plan, d) = InstallPlan::build(&m, &r, target, scope, self.dir.as_path());
        plan.unwrap_or_else(|| panic!("{d:?}"))
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
fn a_partial_uninstall_keeps_its_uninstaller_and_entry_to_run_again() {
    let w = World::new("partial");
    w.install(MANIFEST).unwrap();
    let uninstaller = w
        .app()
        .join(format!("uninstall{}", std::env::consts::EXE_SUFFIX));
    // something that cannot go: a folder where the program was.
    let stuck = w.app().join("app.exe");
    fs::remove_file(&stuck).unwrap();
    fs::create_dir_all(stuck.join("busy")).unwrap();

    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(done.failed.len(), 1, "{:?}", done.failed);
    assert!(uninstaller.is_file(), "the uninstaller stays");
    assert_eq!(w.fake.registered.borrow().len(), 1, "so does the entry");
    assert!(!w.app().join("themes/dark.toml").exists(), "the rest went");

    fs::remove_dir_all(&stuck).unwrap();
    let again = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(again.failed.is_empty(), "{:?}", again.failed);
    assert!(!w.app().exists());
    assert!(w.fake.registered.borrow().is_empty());
    assert_eq!(w.store.read_receipt(&World::id()).unwrap(), None);
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
fn a_file_kept_for_the_person_is_not_scheduled_at_restart() {
    let w = World::new("restart-replaced");
    let manifest = MANIFEST.replace(
        "copy payload/config.toml -> {config}/config.toml keep",
        "copy payload/config.toml -> {config}/config.toml",
    );
    let config = w.dir.join("machine").join("config").join("App");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("config.toml"), b"mine").unwrap();
    w.install(&manifest).unwrap();
    remove_at_restart(&World::id(), &w.fake, &w.store).unwrap();
    assert!(
        !w.fake
            .at_restart
            .borrow()
            .contains(&config.join("config.toml"))
    );
    assert!(w.store.read_receipt(&World::id()).unwrap().is_some());
}

#[test]
fn an_entry_left_with_only_files_for_the_restart_goes_too() {
    let w = World::new("restart-entry");
    let receipt = w.install(MANIFEST).unwrap();
    let mut rest = receipt.clone();
    rest.resources.retain(|r| {
        matches!(
            r,
            Resource::File { .. } | Resource::Directory { .. } | Resource::Registration { .. }
        )
    });
    w.store.write_receipt(&rest).unwrap();
    remove_at_restart(&World::id(), &w.fake, &w.store).unwrap();
    assert!(w.fake.registered.borrow().is_empty());
    assert_eq!(w.store.read_receipt(&World::id()).unwrap(), None);
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

const DRIVER: &str = "copy payload/drv -> {app}/drv\ndriver {app}/drv/x.inf\n  device Root\\X\n";

#[test]
fn drivers_and_their_devices_come_and_go() {
    let w = World::new("win-driver");
    fs::create_dir_all(w.dir.join("payload/drv")).unwrap();
    fs::write(w.dir.join("payload/drv/x.inf"), b"[Version]").unwrap();
    fs::write(w.dir.join("payload/drv/x.sys"), b"sys").unwrap();
    let plan = w.plan_in(&format!("{MANIFEST}{DRIVER}"), Scope::Machine);
    let receipt = install(&plan, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap();
    assert_eq!(w.fake.drivers.borrow().len(), 1);
    assert_eq!(
        w.fake.devices.borrow().values().collect::<Vec<_>>(),
        [r"Root\X"]
    );
    assert!(
        receipt
            .resources
            .iter()
            .any(|r| matches!(r, Resource::Driver { .. }))
    );

    // a repair finds both in place and keeps owning them.
    let again = install(&plan, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap();
    assert_eq!(w.fake.devices.borrow().len(), 1);
    assert!(
        again
            .resources
            .iter()
            .any(|r| matches!(r, Resource::Device { .. }))
    );
    assert!(
        again
            .resources
            .iter()
            .any(|r| matches!(r, Resource::Driver { .. }))
    );

    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(w.fake.drivers.borrow().is_empty());
    assert!(w.fake.devices.borrow().is_empty());
}

#[test]
fn a_driver_someone_else_added_stays() {
    let w = World::new("driver-theirs");
    fs::create_dir_all(w.dir.join("payload/drv")).unwrap();
    fs::write(w.dir.join("payload/drv/x.inf"), b"[Version]").unwrap();
    w.fake
        .drivers
        .borrow_mut()
        .insert("oem3.inf".to_owned(), "[Version]".to_owned());
    w.fake
        .devices
        .borrow_mut()
        .insert(r"ROOT\X\0000".to_owned(), r"Root\X".to_owned());
    let plan = w.plan_in(&format!("{MANIFEST}{DRIVER}"), Scope::Machine);
    install(&plan, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap();
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(w.fake.drivers.borrow().contains_key("oem3.inf"));
    assert_eq!(w.fake.devices.borrow().len(), 1);
}

#[test]
fn a_failed_install_takes_its_driver_out() {
    let w = World::new("driver-fail");
    let fake = Fake {
        fail_register: true,
        ..Fake::new(w.dir.join("machine"))
    };
    fs::create_dir_all(w.dir.join("payload/drv")).unwrap();
    fs::write(w.dir.join("payload/drv/x.inf"), b"[Version]").unwrap();
    let plan = w.plan_in(&format!("{MANIFEST}{DRIVER}"), Scope::Machine);
    assert!(install(&plan, &fake, &w.store, &w.dir.join("kago.exe")).is_err());
    assert!(fake.drivers.borrow().is_empty(), "rolled back");
    assert!(fake.devices.borrow().is_empty(), "rolled back");
}

#[test]
fn services_for_everyone_are_windows_services() {
    let w = World::new("winservice");
    let manifest = format!("{MANIFEST}service main --background\n  restart never\n");
    let plan = w.plan_in(&manifest, Scope::Machine);
    install(&plan, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap();
    let service = w.fake.services.borrow().get("dev.test.app.main").cloned();
    let service = service.unwrap();
    assert!(service.command_line.contains("app.exe"));
    assert!(service.command_line.ends_with("--background"));
    assert!(service.automatic && !service.restart_on_failure);
    assert_eq!(service.display_name, "App main");
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(w.fake.services.borrow().is_empty());

    // a failed install takes its service out again.
    let mut broken = plan.clone();
    broken.operations.push(Operation::CreateDirectory {
        path: "{app}/x".parse().unwrap(),
    });
    let w2 = World::new("winservice-fail");
    let fake = Fake {
        fail_register: true,
        ..Fake::new(w2.dir.join("machine"))
    };
    assert!(install(&broken, &fake, &w2.store, &w2.dir.join("kago.exe")).is_err());
    assert!(fake.services.borrow().is_empty(), "rolled back");
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

/// an upgrade that died after replacing a service's program: the old
/// program goes back first, then the service starts it.
fn half_upgraded_service(w: &World) -> PathBuf {
    let exe = w.app().join("app.exe");
    let backup = w.dir.join("backup-app.exe");
    fs::create_dir_all(w.app()).unwrap();
    fs::write(&backup, b"old").unwrap();
    fs::write(&exe, b"new").unwrap();
    let mut j = journal::Journal::begin(
        &w.store.journal_path(&World::id()),
        &World::id(),
        &crate::package::Version::new(1, 1, 0),
    )
    .unwrap();
    j.step(Undo::RestoreFile {
        path: exe.clone(),
        backup,
    })
    .unwrap();
    j.step(Undo::RestoreService {
        name: "svc".into(),
        previous: Some(crate::platform::WindowsService {
            display_name: "App".into(),
            command_line: format!("\"{}\"", exe.display()),
            automatic: true,
            disabled: false,
            restart_on_failure: false,
        }),
    })
    .unwrap();
    exe
}

#[test]
fn rollback_starts_a_service_on_its_program_put_back() {
    let w = World::new("crash-service");
    let exe = half_upgraded_service(&w);
    recover(&w.fake, &w.store).unwrap();
    assert_eq!(fs::read(&exe).unwrap(), b"old");
    assert_eq!(*w.fake.service_calls.borrow(), ["stop svc", "set svc old"]);
}

#[test]
fn rollback_puts_a_file_back_whose_folder_is_gone() {
    let w = World::new("crash-folder-gone");
    let exe = half_upgraded_service(&w);
    fs::remove_dir_all(w.app()).unwrap();
    let recovered = recover(&w.fake, &w.store).unwrap();
    assert_eq!(recovered, [Recovered::RolledBack(World::id())]);
    assert_eq!(fs::read(&exe).unwrap(), b"old");
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

/// the library's `steamvr`, with SteamVR's list in the sandbox instead of
/// `%LOCALAPPDATA%`.
const DRIVER_ACTION: &str = "action vr-driver dir to={app}/steamvr/$dir.name
  expect $dir/driver.vrdrivermanifest \"no driver manifest\"
  copy $dir -> $to
  json {config}/openvrpaths.vrpath
    init jsonid vrpathreg
    add external_drivers $to
";

#[test]
fn an_action_adds_a_driver_keeps_it_on_upgrade_and_takes_it_out() {
    let mut w = World::new("driver");
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
    let list = w.dir.join("machine/config/App/openvrpaths.vrpath");
    fs::create_dir_all(list.parent().unwrap()).unwrap();
    fs::write(&list, r#"{"external_drivers":["/other/driver"]}"#).unwrap();
    let drivers = || -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&list).unwrap()).unwrap()
            ["external_drivers"]
            .clone()
    };
    let manifest = format!("{MANIFEST}{DRIVER_ACTION}vr-driver payload/driver/aira\n");
    let driver = w
        .dir
        .join("machine")
        .join("programs")
        .join("App")
        .join("steamvr")
        .join("aira");

    w.fake.fail_register = true;
    w.install(&manifest).unwrap_err();
    assert_eq!(
        drivers(),
        serde_json::json!(["/other/driver"]),
        "rolled back"
    );

    w.fake.fail_register = false;
    w.install(&manifest).unwrap();
    assert!(driver.join("bin/win64/driver_aira.dll").is_file());
    let added = serde_json::json!(["/other/driver", driver.display().to_string()]);
    assert_eq!(drivers(), added);

    w.install(&manifest.replace("1.0.0", "1.1.0")).unwrap();
    assert_eq!(drivers(), added, "upgrade still owns the driver");

    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(drivers(), serde_json::json!(["/other/driver"]));
    assert!(!driver.exists());
}

#[test]
fn a_missing_expected_file_is_refused_at_build() {
    let w = World::new("expect");
    fs::create_dir_all(w.dir.join("payload/notadriver")).unwrap();
    fs::write(w.dir.join("payload/notadriver/x.dll"), b"dll").unwrap();
    let m = Manifest::parse(
        "Kagofile",
        &format!("{MANIFEST}{DRIVER_ACTION}vr-driver payload/notadriver\n"),
    )
    .0
    .unwrap();
    let target = "windows-x86_64".parse().unwrap();
    let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
        .0
        .unwrap();
    let (plan, d) = InstallPlan::build(&m, &r, target, Scope::User, w.dir.as_path());
    assert!(plan.is_none());
    let d: Vec<_> = d.iter().collect();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, crate::diagnostic::INSTALL_SOURCE);
    assert!(
        d[0].message.contains("no driver manifest"),
        "{}",
        d[0].message
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

/// a world whose app is this test binary, so `run` has a real program:
/// `--list` exits 0, an unknown flag fails.
fn run_world(name: &str) -> World {
    let w = World::new(name);
    fs::copy(
        std::env::current_exe().unwrap(),
        w.dir.join("payload/app.exe"),
    )
    .unwrap();
    w
}

#[test]
fn a_run_runs_and_its_failure_rolls_back() {
    let w = run_world("run-install");
    w.install(&format!("{MANIFEST}run main --list\n")).unwrap();
    assert!(w.app().join("app.exe").is_file());
    uninstall(&World::id(), &w.fake, &w.store).unwrap();

    let err = w
        .install(&format!("{MANIFEST}run main --no-such-flag\n"))
        .unwrap_err();
    assert!(matches!(err, Error::RolledBack(_)), "{err}");
    assert!(!w.app().exists());
}

#[test]
fn a_run_at_uninstall_is_kept_for_then_and_skipped_on_upgrade() {
    let w = run_world("run-uninstall");
    let manifest = format!("{MANIFEST}run main --list\n  at uninstall\n");
    let receipt = w.install(&manifest).unwrap();
    assert!(receipt.resources.iter().any(
        |r| matches!(r, Resource::UninstallHook { arguments, .. } if arguments == &["--list"])
    ));
    // dropping it in an update must not run it.
    w.install(&MANIFEST.replace("1.0.0", "1.1.0")).unwrap();
    assert!(w.app().join("app.exe").is_file());
    // a broken one does not keep the package installed.
    w.install(
        &format!("{MANIFEST}run main --no-such-flag\n  at uninstall\n").replace("1.0.0", "1.2.0"),
    )
    .unwrap();
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(done.failed.is_empty(), "{:?}", done.failed);
    assert!(!w.app().exists());
}

#[test]
fn a_required_run_at_uninstall_that_fails_keeps_everything() {
    let w = run_world("run-required");
    w.install(&format!(
        "{MANIFEST}run main --no-such-flag\n  at uninstall\n  required\n"
    ))
    .unwrap();
    let err = uninstall(&World::id(), &w.fake, &w.store).unwrap_err();
    assert!(matches!(err, Error::Kept(_)), "{err}");
    assert!(w.app().join("app.exe").is_file());
    assert!(w.store.read_receipt(&World::id()).unwrap().is_some());

    // one that agrees lets the uninstall through.
    w.install(
        &format!("{MANIFEST}run main --list\n  at uninstall\n  required\n")
            .replace("1.0.0", "1.1.0"),
    )
    .unwrap();
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(!w.app().exists());
}

#[test]
fn an_undo_runs_on_rollback_and_is_kept_for_uninstall() {
    let w = run_world("run-undo");
    // the undo fails, so a rollback that ran it says so.
    let err = w
        .install(&format!(
            "{MANIFEST}run main --list\n  undo main --no-such-flag\nrun main --no-such-flag\n"
        ))
        .unwrap_err();
    assert!(err.to_string().contains("rollback was incomplete"), "{err}");

    let w = run_world("run-undo-kept");
    let receipt = w
        .install(&format!(
            "{MANIFEST}run main --list {{app}}\n  undo main --list {{app}}\n"
        ))
        .unwrap();
    let app = w.dir.join("machine").join("programs").join("App");
    assert!(receipt.resources.iter().any(|r| matches!(
        r,
        Resource::UninstallHook { arguments, .. } if arguments == &["--list".to_owned(), app.display().to_string()]
    )));
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(done.failed.is_empty(), "{:?}", done.failed);
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

#[test]
fn changes_are_put_back_at_uninstall() {
    let w = World::new("changes");
    let settings = w.dir.join("machine/config/App/other.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        r#"{"drivers":["/someone/else"],"theme":"light"}"#,
    )
    .unwrap();
    let manifest = format!(
        "{MANIFEST}json {{config}}/other.json\n  add drivers {{app}}/driver\n  set theme dark\n  init format 2\n\
         registry Software\\Test\n  set Folder {{app}}\n  set Count 3\n\
         env set APP_HOME {{app}}\n"
    );
    w.install(&manifest).unwrap();
    let read = || -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap()
    };
    let app = w.dir.join("machine").join("programs").join("App");
    let driver = app.join("driver").display().to_string();
    assert_eq!(
        read()["drivers"],
        serde_json::json!(["/someone/else", driver])
    );
    assert_eq!(read()["theme"], "dark");
    assert_eq!(read()["format"], 2);
    let values = w.fake.values.borrow().clone();
    assert_eq!(
        values[r"HKCU\Software\Test Folder"].as_str(),
        Some(app.display().to_string().as_str())
    );
    assert_eq!(
        values[r"HKCU\Software\Test Count"],
        crate::manifest::Value::word("3", false)
    );
    assert!(values.contains_key("%APP_HOME%"));

    // an update finds them set already and still owns them.
    w.install(&manifest).unwrap();
    assert_eq!(read()["drivers"].as_array().unwrap().len(), 2);

    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    let after = read();
    assert_eq!(after["drivers"], serde_json::json!(["/someone/else"]));
    assert_eq!(after["theme"], "light", "put back as it was");
    assert_eq!(after["format"], 2, "init stays");
    assert!(w.fake.values.borrow().is_empty());
}

#[test]
fn a_value_changed_since_stays_at_uninstall() {
    let w = World::new("changed-since");
    let manifest = format!("{MANIFEST}env set APP_HOME here\n");
    w.install(&manifest).unwrap();
    w.fake.values.borrow_mut().insert(
        "%APP_HOME%".into(),
        crate::manifest::Value::text("mine now"),
    );
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(
        w.fake.values.borrow()["%APP_HOME%"].as_str(),
        Some("mine now")
    );
}

#[test]
fn a_failed_install_takes_its_changes_back() {
    let w = World::new("changes-fail");
    let settings = w.dir.join("machine/config/App/other.json");
    let manifest = format!(
        "{MANIFEST}json {{config}}/other.json\n  add drivers x\n  set theme dark\nenv set APP_HOME here\n"
    );
    let fake = Fake {
        fail_register: true,
        ..Fake::new(w.dir.join("machine"))
    };
    let mut plan = w.plan(&manifest);
    // the changes run first, then the registration fails.
    let register = plan
        .operations
        .iter()
        .position(|op| matches!(op, Operation::RegisterApplication { .. }))
        .unwrap();
    let op = plan.operations.remove(register);
    plan.operations.push(op);
    assert!(install(&plan, &fake, &w.store, &w.dir.join("kago.exe")).is_err());
    assert!(!settings.exists(), "the file the install made is gone");
    assert!(fake.values.borrow().is_empty());
}

#[test]
fn settings_file_the_install_made_goes_at_uninstall() {
    let w = World::new("settings-made");
    let settings = w.dir.join("machine/config/App/made.json");
    w.install(&format!(
        "{MANIFEST}json {{config}}/made.json\n  set a.b 1\n"
    ))
    .unwrap();
    assert!(settings.is_file());
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(!settings.exists());
}

#[test]
fn settings_file_with_someone_elses_key_stays_at_uninstall() {
    let w = World::new("settings-kept");
    let settings = w.dir.join("machine/config/App/kept.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"other":1}"#).unwrap();
    w.install(&format!(
        "{MANIFEST}json {{config}}/kept.json\n  set a.b 1\n"
    ))
    .unwrap();
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    let left: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(left, serde_json::json!({ "other": 1 }));
}

#[test]
fn purge_takes_what_uninstall_kept_and_the_settings_folder() {
    let w = World::new("purge");
    let receipt = w.install(MANIFEST).unwrap();
    let config = w.dir.join("machine").join("config").join("App");
    fs::write(config.join("config.toml"), b"changed by the user").unwrap();
    fs::write(config.join("written-by-the-app.json"), b"{}").unwrap();
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(config.join("config.toml").is_file(), "kept without purge");
    let own = w.dir.join("machine").join("elsewhere").join("App data");
    fs::create_dir_all(&own).unwrap();
    fs::write(own.join("recording.bin"), b"x").unwrap();
    let receipt = Receipt {
        purge: vec![own.clone()],
        ..receipt
    };
    let failed = purge(&receipt, &w.fake, &done.kept);
    assert!(!own.exists(), "a folder the Kagofile names");
    assert!(failed.is_empty(), "{failed:?}");
    assert!(!config.exists());
    assert!(
        w.dir.join("machine").join("config").exists(),
        "only the package's folder"
    );
}

#[test]
fn purge_leaves_shared_files_alone() {
    let w = World::new("purge-shared");
    let manifest = MANIFEST.replace(
        "copy payload/config.toml -> {config}/config.toml keep",
        "copy payload/config.toml -> {app}/shared.toml shared",
    );
    let receipt = w.install(&manifest).unwrap();
    let shared = w.app().join("shared.toml");
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(shared.is_file(), "uninstall keeps a shared file");
    let failed = purge(&receipt, &w.fake, &done.kept);
    assert!(failed.is_empty(), "{failed:?}");
    assert!(
        shared.is_file(),
        "purge does too: another program may use it"
    );
}

#[test]
fn purge_never_takes_a_folder_others_use() {
    let w = World::new("purge-known");
    let receipt = w.install(MANIFEST).unwrap();
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    let theirs = w.dir.join("machine").join("Shared");
    fs::create_dir_all(&theirs).unwrap();
    fs::write(theirs.join("letter.txt"), b"x").unwrap();
    let receipt = Receipt {
        purge: vec![theirs.clone()],
        ..receipt
    };
    let failed = purge(&receipt, &w.fake, &done.kept);
    assert!(theirs.join("letter.txt").is_file());
    assert_eq!(failed.len(), 1, "{failed:?}");
}

#[test]
fn a_file_the_person_had_comes_back_at_uninstall() {
    let mut w = World::new("replaced");
    let manifest = MANIFEST.replace(
        "copy payload/config.toml -> {config}/config.toml keep",
        "copy payload/config.toml -> {config}/config.toml",
    );
    let config = w.dir.join("machine").join("config").join("App");
    fs::create_dir_all(&config).unwrap();
    let file = config.join("config.toml");
    fs::write(&file, b"mine").unwrap();
    w.fake.fail_register = true;
    w.install(&manifest).unwrap_err();
    assert_eq!(fs::read(&file).unwrap(), b"mine", "rolled back");
    w.fake.fail_register = false;
    w.install(&manifest).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"cfg");
    // an update keeps it aside, still the person's.
    w.install(&manifest.replace("1.0.0", "1.1.0")).unwrap();
    let done = uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert!(done.failed.is_empty(), "{:?}", done.failed);
    assert_eq!(fs::read(&file).unwrap(), b"mine", "put back");
    assert!(
        !w.store
            .root()
            .join("replaced")
            .join(World::id().as_str())
            .exists()
    );
}

/// a junction made with `mklink /J`, which needs no administrator.
#[cfg(windows)]
#[test]
fn an_install_for_everyone_never_writes_through_a_link() {
    let w = World::new("linked");
    let elsewhere = w.dir.join("system");
    fs::create_dir_all(&elsewhere).unwrap();
    let config = w.dir.join("machine").join("config");
    fs::create_dir_all(&config).unwrap();
    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(config.join("App"))
        .arg(&elsewhere)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let plan = w.plan_in(MANIFEST, Scope::Machine);
    let err = install(&plan, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap_err();
    assert!(err.to_string().contains("link"), "{err}");
    assert_eq!(
        fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "nothing went there"
    );
    // just for the user, the folder is theirs to link.
    w.install(MANIFEST).unwrap();
    fs::remove_dir(config.join("App")).unwrap();
}

#[cfg(windows)]
#[test]
fn an_update_that_only_changes_a_name_s_case_keeps_the_file() {
    let w = World::new("case");
    let v1 = MANIFEST.replace(
        "copy payload/themes",
        "copy payload/config.toml -> {app}/Tool.toml",
    );
    w.install(&v1).unwrap();
    let v2 = v1
        .replace("{app}/Tool.toml", "{app}/tool.toml")
        .replace("1.0.0", "1.1.0");
    w.install(&v2).unwrap();
    assert!(w.app().join("tool.toml").is_file(), "not dropped as stale");
}

#[test]
fn an_update_without_a_secret_leaves_the_setting_using_it() {
    let w = World::new("secret-kept");
    let manifest = format!(
        "{MANIFEST}\npage login \"Login\"\n  text token \"Token\" secret\n\nenv set KAGO_TOKEN {{input.token}}\n"
    );
    let answers = BTreeMap::from([("token".to_owned(), "s3cret".to_owned())]);
    let first = w.plan(&manifest).answered(&answers, None, &[]).unwrap();
    install(&first, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap();
    let held = || w.fake.values.borrow().values().next().cloned();
    assert_eq!(held().unwrap().as_str(), Some("s3cret"));

    let newer = manifest.replace("1.0.0", "1.1.0");
    let earlier = BTreeMap::new();
    let second = w
        .plan(&newer)
        .answered(&BTreeMap::new(), Some(&earlier), &[])
        .unwrap();
    install(&second, &w.fake, &w.store, &w.dir.join("kago.exe")).unwrap();
    assert_eq!(held().unwrap().as_str(), Some("s3cret"), "not blanked");

    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(held(), None, "still owned, so it is taken back");
}

/// moving between scopes: the new copy goes in first, finds the setting the
/// old one owns, and sets it again once the old one took it back.
#[test]
fn a_shared_setting_survives_moving_between_scopes() {
    let w = World::new("move-scope");
    let manifest = format!("{MANIFEST}\nenv set KAGO_MOVED yes\n");
    let other = Store::new(w.dir.join("other-state"));
    let runtime = w.dir.join("kago.exe");
    install(&w.plan(&manifest), &w.fake, &other, &runtime).unwrap();
    let held = || w.fake.values.borrow().values().next().cloned();
    let new = w.plan(&manifest);
    install(&new, &w.fake, &w.store, &runtime).unwrap();
    remove(&World::id(), &w.fake, &other).unwrap();
    assert_eq!(held(), None, "the old copy took it back");
    install(&new, &w.fake, &w.store, &runtime).unwrap();
    assert!(held().is_some(), "set again");
    uninstall(&World::id(), &w.fake, &w.store).unwrap();
    assert_eq!(held(), None, "and owned by the new copy");
}
