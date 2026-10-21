# kago

Kago turns a small text file, the `Kagofile`, into a Windows setup program. The setup installs into Program Files by default, or just for the current user. It can be repaired or upgraded, and it uninstalls cleanly through its own `uninstall.exe` or Settings › Apps. Every install keeps a receipt of what it put on the machine. Uninstall removes exactly that, and a failed install undoes itself.

The setup has two looks:

- **modern**: dark, drawn like Lykil and Aira Studio
- **native**: a classic NSIS-style wizard

Both come in English and Korean.

## Install kago

You need Rust 1.89 or newer.

```sh
cargo install --git https://github.com/SkuldNorniern/kago
```

or, from a checkout:

```sh
cargo install --path .
```

Check it with `kago version`.

## Make a setup in three steps

From your project folder, in any language:

```sh
kago init                    # writes a starter Kagofile
kago build                   # runs its build lines, writes dist/<Name>-<version>-setup-windows-x86_64.exe
```

`kago init` reads a `Cargo.toml`, `pyproject.toml` or `package.json` when there is one, and fills in:

- the name, description and homepage
- the `version` line, so the version follows your project file
- `build` lines: `cargo build --release`, PyInstaller for Python, `npm run build` for Node
- the program to install (one `app` per Rust binary; PyInstaller's folder for Python)
- the first `.ico` it finds
- your LICENSE

Without a project file it writes a plain template. Fix the app path to wherever your build puts the program.

Open the Kagofile and set two things:

- your own package id on the first line, like `dev.yourname.app`
- `publisher`

Then run `kago check` to see whether everything is right. Errors point at the line and column.

Run the setup you built to try it. `kago status` lists what is installed, and `kago uninstall <package-id>` removes it again.

## A Kagofile

```
package dev.eira.ozone
  name Ozone
  version from cargo
  publisher Eira
  description Small command-driven text editor

license LICENSE

app ozone
  windows target/release/ozone.exe
  icon windows packaging/windows/ozone.ico

shortcut
path optional "Add ozone to PATH"

component desktop "Desktop shortcut" off
  shortcut desktop
```

The format:

- one statement per line
- the lines under a statement are indented
- `#` starts a comment
- put text in quotes when it has spaces and is not the last thing on the line

[examples/](examples) has the Kagofiles for Ozone, Lykil Studio and Aira Studio.

### package

The first block. Only the id and `version` are required.

| line | means |
|---|---|
| `package <id>` | reverse-domain id, like `dev.eira.ozone`; it never changes between versions |
| `name <text>` | shown everywhere; default: the last part of the id |
| `version 1.2.0` | or read it: `version from toml pyproject.toml project.version`, `version from json package.json version`, `version from file VERSION`, `version from git` (newest tag), `version from cargo [path]` |
| `publisher`, `description`, `homepage` | shown in the setup and in Settings › Apps |
| `setup modern` / `setup native` | the look; modern by default |
| `folder choose` / `folder fixed` | whether people may pick the install folder; choose by default |
| `downgrade deny` / `downgrade allow` | whether an older version may replace a newer one; deny by default |
| `scope machine` / `scope user` / `scope choose` | for everyone in Program Files (the default; the setup asks for administrator rights), just for the current user in `%LOCALAPPDATA%\Programs` with no prompt, or let the person installing pick |

With `scope choose` the setup shows "Install for: Everyone / Only for me", starting on everyone, or on wherever it is installed already. The window opens without administrator rights. Only when the person picks everyone and clicks Install does Windows ask, and the setup carries on as administrator. PATH entries follow the pick. A `service` needs `scope user` under it.

### files

| line | means |
|---|---|
| `app <id>` + `windows <file>` | the program; the first `app` is the one the Start menu and "Run" open |
| `  windows x86_64 <file>` | a file for one architecture (`x86_64`, `aarch64`) |
| `  windows <file> -> {app}/bin/x.exe` | install somewhere else than `{app}` |
| `  to {app}/bin` | the folder for this app's files |
| `  icon windows <file>.ico` | the app's icon, also used for the setup exe and its logo |
| `copy <file or folder>` | more files, into `{app}` |
| `copy <x> -> {config}/x` | somewhere else |
| `copy <x> keep` | config the user may change: kept on upgrade and uninstall when changed |
| `copy <x> user-data` | never removed |

Install paths start with a root:

| root | on Windows |
|---|---|
| `{app}` | `C:\Program Files\<Name>` (`scope user`: `%LOCALAPPDATA%\Programs\<Name>`), or the folder picked in setup |
| `{bin}` | `{app}\bin` |
| `{config}`, `{data}` | `%ProgramData%\<Name>` (`scope user`: `%APPDATA%\<Name>`, `%LOCALAPPDATA%\<Name>`) |
| `{cache}`, `{logs}`, `{state}` | under `{data}` |
| `{desktop}`, `{applications}` | Desktop, Start menu (for everyone, or the user's own) |
| `{user-home}` | the user's home folder (`scope user` only) |

File paths may use `${version}`, `${name}`, `${id}`, `${os}`, `${arch}`, `${target}`, `${triple}` (like `x86_64-pc-windows-msvc`) and `${exe}` (`.exe` on Windows).

### build steps

`kago build` runs these first, so the setup always holds what you just built:

```
build cargo build --release                      # one step on its line

build                                            # or several under it
  python -m PyInstaller --noconfirm --windowed --name tool main.py
  script tools/build-driver.ps1 -Release        # a script of your own
  when windows
    script tools/sign-driver.cmd
  when linux
    script tools/build-driver.sh
```

- Steps run in order from the Kagofile's folder, and the build stops at the first one that fails. `kago build --no-build` skips them, to pack what is there.
- `script` picks the program by the file's extension: `.ps1` PowerShell, `.cmd` and `.bat` cmd, `.sh` sh, `.py` Python (`py` on Windows). Other files run as they are. `kago check` says when a script is missing.
- `when` limits steps to some targets, like any `when` block.
- Every step gets `KAGO_PACKAGE`, `KAGO_NAME`, `KAGO_VERSION`, `KAGO_TARGET` and `KAGO_OUT` (the folder the setup goes to) in its environment.
- A program starts directly, not through a shell; for `npm`, which is `npm.cmd` on Windows, write `cmd /c npm run build`.

### sources

Files can come from somewhere other than the Kagofile's folder:

```
source motion
  dir ../aira-motion                      # another folder

source release
  fetch https://example.com/app-${version}-${triple}.zip
  sha256 windows-x86_64 fe869b1f...       # one per target, or one for all

app ozone
  windows release:ozone.exe               # <source>:<path inside it>
```

A download is only used when its sha256 matches. Leave the `sha256` line out at first: the build prints the exact line to add. Archives (`.zip`, `.tar.gz`) are unpacked; `unpack none` keeps them as they are. Downloads are cached in `.kago/` next to the Kagofile.

### what the install sets up

| line | means |
|---|---|
| `shortcut` | Start menu entry for the first app; `shortcut "Name" desktop -> <app>` for others |
| `path` | puts the app's folder on the user's PATH; `path optional "label"` makes it a tick box |
| `service <app> [arguments]` | installed for everyone: a Windows service (your program has to speak the service protocol); `scope user` under it: starts the app when the person signs in instead. Under it: `name "..."`, `start manual`, `restart never` (it restarts after a crash by default) |
| `firewall <app> ["Rule name"] [public]` | lets the app accept connections through Windows Firewall on private networks (and public ones with `public`), so Windows does not ask the first time it listens; installs for everyone only, the uninstall removes the rule |
| `require windows 10.0.17763` | refuses to install on older Windows; nothing changes |
| `require <check>` | refuses to install unless a `check` finds what it looks for, like `require steamvr` |

### settings outside the package

Values in other programs' settings files, the registry and the environment. Each is put back at uninstall, and on a failed install:

```
json %LOCALAPPDATA%/openvr/openvrpaths.vrpath
  init version 1                           # only when missing; stays after uninstall
  add external_drivers {app}/steamvr/aira  # an item in a list; taken out again
  set theme dark                           # a value; the old one comes back

registry Software\Eira\Aira                 # HKCU per user, HKLM for everyone
  set InstallDir {app}
  set Telemetry 0                          # a whole number is a DWORD
registry HKCU\Software\Eira\Aira set @ "x"  # `@` is the default value; one line works too

env set AIRA_HOME {app}
env add AIRA_PLUGINS {app}/plugins         # an item in a `;` list
```

- A JSON file starts with a root like `{config}/` or a variable like `%APPDATA%/`. Keys are paths like `a.b.c`.
- `{app}` and the other roots in a value become real paths at install.
- A value someone changed since the install is left alone at uninstall.
- `HKLM\` only works for installs for everyone.

### running programs

```
run setup-tool --register {app}            # an app id, an installed file like {app}/x.exe, or a program on PATH
  undo setup-tool --unregister {app}       # runs on rollback and at uninstall
run {app}/tools/cleanup.exe
  at uninstall                             # first thing at uninstall
run {app}/agent.exe --may-uninstall
  at uninstall
  required                                 # it fails: nothing is closed or removed
```

A program run at uninstall is best effort: when it fails, the uninstall goes on. With `required` it may say no: it runs before the app is closed, may take up to 30 minutes, for example to ask for a code from an administrator, and anything but exit code 0 stops the uninstall and shows why.

A `run` that fails at install undoes the install. Prefer a building block above when one fits: kago can check and roll those back itself, but it cannot know what a program did. An install for everyone only runs programs from the package, never ones from the machine.

### setup pages of your own

Ask the person installing for what your app needs, on pages after the welcome page:

```
page server "Server settings"
  about Where Aira Studio sends tracker data.
  text host "Server address" default=localhost required
  number port "Port" default=9000 min=1 max=65535
  choice mode "Mode" local cloud            # the first option is the default, or default=cloud
  text key "Access key" secret              # shown as dots, not remembered for updates
  check telemetry "Send usage data" off     # a tick box, off or on

json {config}/settings.json
  set server.host {input.host}
  set server.port {input.port}              # a number field gives a number, a tick box true or false
run app --register {input.key}
```

- `{input.<id>}` works in `json`, `registry` and `env` values, and in the arguments of `run`, `shortcut` and `service`. `kago check` says when an id has no field.
- Next checks every field on the page: required ones filled in, numbers whole and in range.
- `skip <check>` under a page leaves it out when the check finds its thing, like a key file an earlier install left: its fields keep their last answer or default.
- An update or repair starts each field at the answer given last time. Secret fields start empty.
- Silent installs take answers with `--set host=example.com`, for the setup and for `kago install`; fields left out keep their last answer or default.

A secret field is only hidden on screen. Its value ends up wherever you use it, like a settings file or a program's arguments, and in the receipt that lets uninstall put things back.

### checks

A `check` tells whether something is on the machine. `require`, a component's `detect` and a `prerequisite` use them:

```
check python "Python 3.12"
  registry HKLM\SOFTWARE\Python\PythonCore\3.12      # a key, or `registry <key> <value>`
check tool "Tool" file %ProgramFiles%/Tool/tool.exe
check steamvr "SteamVR" json %LOCALAPPDATA%/openvr/openvrpaths.vrpath runtime   # a key that holds something

require python
```

### actions

An `action` is your own statement, made of the lines above. `$name` is a parameter; `$name.name` is the last part of its path:

```
action plugin dir to={app}/plugins/$dir.name
  expect $dir/plugin.json "a plugin folder holds plugin.json"   # the build fails without it
  copy $dir -> $to
  json {config}/settings.json
    add plugins $to

plugin build:plugins/export
plugin build:plugins/import to={data}/import
```

`use actions.kago` brings in the `check` and `action` lines of another file, to share them between projects.

kago's own library is written the same way ([library.kago](src/manifest/kagofile/library.kago)), and a Kagofile can replace any of it by defining the same name:

| name | does |
|---|---|
| `steamvr <driver folder> [to=<path>]` | copies a SteamVR driver and adds it to SteamVR's driver list; the folder needs `driver.vrdrivermanifest` |
| `check steamvr` | whether SteamVR has run on this machine |
| `vcredist <installer>` | the Visual C++ runtime as a prerequisite |
| `webview2 <installer>` | the WebView2 runtime as a prerequisite |
| `file-type <.ext> <id> "Name" <program> [icon=<file>]` | files with that extension open in the program, with its icon; taken back at uninstall |
| `url-protocol <scheme> "Name" <program>` | links like `scheme://...` start the program with the link |

### components

Parts people can tick or untick in the setup:

```
component steamvr "SteamVR support" required
  about Shows Aira trackers in SteamVR
  detect steamvr
  steamvr driver/aira

component docs "Documentation" off
  copy docs
```

- `on` (default) / `off` sets the starting tick. `required` shows it ticked and greyed out.
- `needs <other>` ticks the other component along with it.
- `about` is shown beside the name, with the size.
- `detect <check>` starts the box unticked, with "SteamVR not found", when the check finds nothing. `detect` also takes a test directly, like `detect file %ProgramFiles%/Tool/tool.exe`.

`when windows` (or `linux`, `x86_64`, ...) blocks limit lines to some targets.

### prerequisites

Other vendors' installers your app needs, like the Visual C++ runtime. The installer is pinned and fetched at build time like any `source`, then carried inside the setup. At install time it runs only when its check says it is missing, with a UAC prompt when it needs administrator rights:

```
source vcredist
  fetch https://aka.ms/vs/17/release/vc_redist.x64.exe
  sha256 ...                        # the build prints the line to add
  unpack none

vcredist vcredist:vc_redist.x64.exe
```

`vcredist` and `webview2` are actions from kago's library: they fill in the name, the silent arguments, the check and that they need administrator rights. Anything else says it all:

```
prerequisite tool "Tool Runtime"
  file deps:tool-setup.msi            # .msi files run through Windows Installer
  arguments /qn
  detect registry HKLM\SOFTWARE\Tool Version   # or: detect file %ProgramFiles%\Tool\tool.exe
  admin
```

Exit codes 0, 3010 and 1641 (restart later) and 1638 (a newer one is there) count as installed. Uninstalling your app leaves prerequisites in place, since other programs may use them.

### updates

```
update github SkuldNorniern/aira-studio       # the newest GitHub release
# update feed https://example.com/aira/latest.json
```

With an `update` line, `kago build` writes `latest.json` next to the setup: the version, the setup's file name, sha256 and size. Publish both together:

- **GitHub:** attach both files to the release. GitHub sends `releases/latest/download/latest.json` to the newest release, and the setup is taken from that same release.
- **Your own server:** put both files in one folder and point `update feed` at the `latest.json` there. Builds for several targets add to the same `latest.json`.

Installed apps then update with the kago runtime that the setup puts in place:

```
%LOCALAPPDATA%\Kago\bin\kago.exe update dev.eira.aira-studio --check    # just says what is newer
%LOCALAPPDATA%\Kago\bin\kago.exe update dev.eira.aira-studio            # the new setup's window
%LOCALAPPDATA%\Kago\bin\kago.exe update dev.eira.aira-studio --silent   # no window
```

For installs for everyone the runtime is `%ProgramData%\Kago\bin\kago.exe`, and the setup asks for administrator rights. Your app can run these itself, for example from a "Check for updates" menu. A download is only run when its sha256 matches `latest.json`. The update closes the app if it is still running, so quit it first if you want to restart it afterwards.

### license

```
license LICENSE           # shown before installing
license LICENSE accept    # must be accepted to install
```

### brand

What the setup looks like, after NSIS Modern UI. All lines are optional:

```
brand
  text Eira Installer             # small text bottom left
  accent 6667ab                   # main color, no #; taken from the pictures below when left out
  logo brand/logo.png             # replaces the app icon in the modern look
  banner brand/banner.png         # modern look: across the top behind the name, fading into the page; supply 1280 x 272
  header brand/header.png         # top right, corners rounded in the modern look; supply 300 x 114
  side brand/side.png             # left edge, fading into the page; supply 328 x 560
  welcome Thanks for choosing Ozone.
  finish Ozone is ready.
  link "Read the guide" https://example.com/guide
```

Images are PNG. They are stored in the setup and scaled for the screen's DPI when the window opens. With a `banner`, the modern look leaves the `header` out. Without `accent`, `kago build` takes the main color of the banner, side picture or logo (the app icon by default), so the buttons match them in the setup and the uninstaller.

### sign

Signs the setup with Authenticode, so UAC shows your name instead of "Unknown publisher" and SmartScreen can build up trust in it. It needs signtool from the Windows SDK; set `KAGO_SIGNTOOL` if it is somewhere unusual.

```
sign
  certificate 1A2B3C...                   # thumbprint of a certificate in your Windows store, or on a USB token
  # certificate env KAGO_CERT             # the same, from an environment variable (CI)
  # pfx certs/code.pfx env KAGO_PFX_PASS  # a .pfx file; its password only ever comes from the environment
  # azure metadata.json dlib C:/tools/Azure.CodeSigning.Dlib.dll   # Azure Trusted Signing
  timestamp http://timestamp.digicert.com # the default
  programs off                            # leave your own .exe and .dll files unsigned
```

What gets signed:

- the setup
- the uninstaller and the runtime it installs
- your own `.exe` and `.dll` files that are not signed yet

Your files are signed as copies inside the setup; the ones in your build folder stay as they are. `kago build --no-sign` skips signing, for quick test builds.

## Commands

```
kago init [dir] [--id <package-id>] [--force]   starter Kagofile
kago check [--target <t>]...                    validate and resolve
kago plan [--components a,b]                    what an install would do, without doing it
kago build [--out <dir>] [--components a,b] [--no-sign]   the setup exe (default: every component offered)
kago install [--components a,b] [--location <dir>]   install straight from the Kagofile
kago uninstall <package-id>
kago verify <package-id>                        compare installed files with the receipt
kago status                                     what is installed
kago recover                                    finish or undo an install a crash cut short
kago update [<package-id>] [--check] [--silent] install a newer version from the `update` feed
kago version
```

## Setup command line

For rollouts with Intune, SCCM, a script or similar:

```
<setup>.exe                                   the window
<setup>.exe --silent                          no window, defaults (also /S, like NSIS)
<setup>.exe --silent --components steamvr,docs --location D:\Apps\Aira --set host=example.com
<setup>.exe --silent --scope user             with scope choose: just this user
<setup>.exe --help
```

The exit code is 0 when installed and 1 otherwise, with the reason on stderr.

The setup says what it will do before it starts:

- a new install
- a repair, for the same version
- an update from the installed version, in the same folder
- moving the app between per-user and Program Files, removing the other copy
- refusing to replace a newer version, unless the package has `downgrade allow`

When the app is running, the setup lists it and closes it before replacing its files, like Inno Setup; a silent setup closes it without asking. Only one setup or uninstaller per package runs at a time, and the folder page shows the space needed and free.

An update or repair keeps the components that are installed. A component that is new in this version starts as the package says.

Every install has `uninstall.exe` in its folder, like an NSIS uninstaller:

- it asks first, then removes everything the install put there, and lists what it kept, like your changed settings
- a running app is closed first; what stays in use is removed when Windows restarts (for installs for everyone)
- it looks like the setup did: modern with the brand's accent and the app icon, or native
- Settings › Apps runs it
- `uninstall.exe /S` removes the app silently
- `QuietUninstallString` (the command-line runtime) gives tools a real exit code

## Not supported yet

- Linux and macOS setups, and MSI, DMG, AppImage, DEB, RPM outputs; `target` lines for them only check the Kagofile
- building a setup for another OS or architecture than the machine running `kago build`
- checking for updates on a schedule; the app or the person runs `kago update`
