# kago

Kago turns a small text file, the `Kagofile`, into a Windows setup program. The setup installs into the user's own folders, can be repaired or upgraded, and uninstalls cleanly from Settings › Apps. Every install keeps a receipt of what it put on the machine. Uninstall removes exactly that, and a failed install undoes itself.

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

From your project folder (for a Rust project, next to its `Cargo.toml`):

```sh
cargo build --release        # build your app first
kago init                    # writes a starter Kagofile from Cargo.toml
kago build                   # writes dist/<Name>-<version>-setup-windows-x86_64.exe
```

`kago init` fills in:

- the name, description and homepage
- `version from cargo`, so the version always follows your crate
- one `app` per binary
- the first `.ico` it finds
- your LICENSE

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
| `version 1.2.0` | or `version from cargo`, `version from cargo <path>`, `version from file <path>`, `version from git` (newest tag) |
| `publisher`, `description`, `homepage` | shown in the setup and in Settings › Apps |
| `setup modern` / `setup native` | the look; modern by default |
| `folder choose` / `folder fixed` | whether people may pick the install folder; choose by default |
| `downgrade deny` / `downgrade allow` | whether an older version may replace a newer one; deny by default |
| `scope user` | per-user install, the default (machine-wide is not supported yet) |

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
| `{app}` | `%LOCALAPPDATA%\Programs\<Name>`, or the folder picked in setup |
| `{bin}` | `{app}\bin` |
| `{config}` | `%APPDATA%\<Name>` |
| `{data}` | `%LOCALAPPDATA%\<Name>` |
| `{cache}`, `{logs}`, `{state}` | under `{data}` |
| `{desktop}`, `{applications}` | Desktop, Start menu |
| `{user-home}` | the user's home folder |

File paths may use `${version}`, `${name}`, `${id}`, `${os}`, `${arch}`, `${target}`, `${triple}` (like `x86_64-pc-windows-msvc`) and `${exe}` (`.exe` on Windows).

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
| `steamvr <driver folder>` | copies a SteamVR driver and registers it with SteamVR; the folder needs `driver.vrdrivermanifest` |
| `service <app> [arguments]` | starts the app when the user signs in; `name "..."` under it |
| `hook post-install <app> [arguments]` | runs one of your own programs after the files are in; if it fails, the install is undone |
| `hook pre-uninstall <app> [arguments]` | runs first on uninstall |
| `require windows 10.0.17763` | refuses to install on older Windows; nothing changes |
| `require steamvr` | refuses to install without SteamVR |

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
- `detect steamvr` starts the box unticked, with "SteamVR not found", when SteamVR is missing.

`when windows` (or `linux`, `x86_64`, ...) blocks limit lines to some targets.

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
  accent 6667ab                   # main color, no #
  logo brand/logo.png             # replaces the app icon in the modern look
  header brand/header.png         # top right; supply 300 x 114
  side brand/side.png             # left of welcome and finish; supply 328 x 560
  welcome Thanks for choosing Ozone.
  finish Ozone is ready.
  link "Read the guide" https://example.com/guide
```

Images are PNG. They are stored in the setup and scaled for the screen's DPI when the window opens.

## Commands

```
kago init [dir] [--id <package-id>] [--force]   starter Kagofile
kago check [--target <t>]...                    validate and resolve
kago plan [--components a,b]                    what an install would do, without doing it
kago build [--out <dir>] [--components a,b]     the setup exe (default: every component offered)
kago install [--components a,b] [--location <dir>]   install straight from the Kagofile
kago uninstall <package-id>
kago verify <package-id>                        compare installed files with the receipt
kago status                                     what is installed
kago recover                                    finish or undo an install a crash cut short
kago version
```

## Setup command line

For rollouts with Intune, SCCM, a script or similar:

```
<setup>.exe                                   the window
<setup>.exe --silent                          no window, defaults (also /S, like NSIS)
<setup>.exe --silent --components steamvr,docs --location D:\Apps\Aira
<setup>.exe --help
```

The exit code is 0 when installed and 1 otherwise, with the reason on stderr.

Running the same setup again repairs the install. A newer setup upgrades it in the same folder.

## Not supported yet

- installing for all users (`scope machine`) and machine-wide services: these need an elevation helper
- code signing; setups are unsigned, so SmartScreen may warn on first run
- Linux and macOS setups, and MSI, DMG, AppImage, DEB, RPM outputs; `target` lines for them only check the Kagofile
- building a setup for another OS or architecture than the machine running `kago build`
- updates from a server (the `update` part of a package)
