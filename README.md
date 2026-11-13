[![version](https://img.shields.io/crates/v/kptui.svg)](https://crates.io/crates/kptui)
[![build](https://github.com/pepa65/kptui/actions/workflows/rust.yml/badge.svg)](https://github.com/pepa65/kptui/actions/workflows/rust.yml)
[![dependencies](https://deps.rs/repo/github/pepa65/kptui/status.svg)](https://deps.rs/repo/github/pepa65/kptui)
[![docs](https://img.shields.io/badge/docs-kptui-blue.svg)](https://docs.rs/crate/kptui/latest)
[![license](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://github.com/pepa65/kptui/blob/main/LICENSE.md)
[![downloads](https://img.shields.io/crates/d/kptui.svg)](https://crates.io/crates/kptui)

# kptui 0.36.2
**Highly secure TUI password manager for [KeePass](https://keepass.info/) KDBX vaults (includes Linux GUI)**
* Compatible with the PassKeeper/KeePass KDBX `.kdb` and `.kdbx` file format.
* It saves its database file to `KDBX4.1` format, which can be used by:
  KeePassXC, KeePassDX/KeePass2Android, KeeWeb and KeePass2.
* Fits in with the wider KeePass ecosystem.
* Very secure in its default configuration, impossible to brute-force (so unlocking takes a few seconds!).
  - Writes to `KDBX4.1` hardened with Argon2.
  - KDF-Memory: `262144 KiB`
  - KDF-Iterations: `90` (!) [configurable]
  - KDF-Parallel: `4`
* Stores: `Name` of the entry, `Username`, `Password`, `TOTP`, `URL` and `Notes` (only what it needs).
* The Terminal User Interface is responsive and fully keyboard-driven, and a stand-alone static binary.
* The Graphical User Interface runs on X11 and Wayland Linux and is even more secure in wiping all secrets.
  This binary (`kpgui`) is dynamically linked to C and graphical system libraries.

## Features
* **KDBX4.1 vaults**: Reads and writes standard `.kdbx` files, so you can open the same vault in
  KeePassXC, KeePassDX/KeePass2Android, KeeWeb, KeePass2 or any other compatible client.
* **Optional keyfile support**: Can unlocks vaults protected by a master password plus a keyfile,
  but password-only vault is also supported.
* **Configurable KDF iterations**: If more or less paranoid (the default 90 is a high setting).
* **Filtered search**: Start typing on the index screen to filter entries on Name & User,
  or on Name/User/Notes (toggled by control-C).
* **TOTP codes**: Generates live 2FA codes from a stored seed and saves `otpauth://` URI.
  - Editable as: `SECRET [DIGITS [PERIOD]]` (`PERIOD` defaults to `30`, and `DIGITS` to `6`)
* **Auto-exit**: The whole app can close itself automatically after a set period of inactivity
  in seconds (`0`: stays open indefinitely).
* **Themeable**: Ships with a range of themes and supports custom themes.
* **Access Configuration from the app**: On the Login screen or the Index screen, give Control-C:
  - Configuration file: `~/.config/kptui/config.toml`
  - Database (default: `~/.local/share/kptui/default.kdbx`)
  - (Optional) keyfile
  - Auto-exit setting (in seconds, `0`: never auto-exit)
  - Theme selection (location: `~/.config/kptui/themes/`)
  - Change the master password
  - Import from file
  - Export to file
* **Import from other vaults or export**: Import entries from another `.kdbx` file, or from a `.csv`/`.json` file
  (as produced by other password managers). Export to CSV/JSON or KDBX.

## Usage
```
kptui 0.36.2 - TUI password manager for KeePass vaults
Usage:  kptui [slim | version | help]
```

## Installing
### Download static single-binary
```
wget https://github.com/pepa65/kptui/releases/download/0.36.2/kptui
sudo mv kptui /usr/local/bin
sudo chown root:root /usr/local/bin/kptui
sudo chmod +x /usr/local/bin/kptui
```

### Download

### Install with cargo
#### Static musl build from cloned repo
```
# After git-cloning the repo
rustup target add x86_64-unknown-linux-musl
cargo build --release
```

#### Dynamic build with cargo
`cargo install --git https://github.com/pepa65/kptui`

### Install with cargo-binstall
Even without a full Rust toolchain, rust binaries can be installed with the static binary `cargo-binstall`:

```
# Install cargo-binstall for Linux x86_64
# (Other versions are available at https://crates.io/crates/cargo-binstall)
wget github.com/cargo-bins/cargo-binstall/releases/latest/download/cargo-binstall-x86_64-unknown-linux-musl.tgz
tar xf cargo-binstall-x86_64-unknown-linux-musl.tgz
sudo chown root:root cargo-binstall
sudo mv cargo-binstall /usr/local/bin/
```

Only a linux-x86_64 (musl) binary available: `cargo-binstall kptui`

It will be installed in `~/.cargo/bin/` which will need to be added to `PATH`!

## Getting started
* On first launch, if no vault is found at the default (configured) path, `kptui` will:
  - Prompt to create a new database.
  - Prompt to set a master password.
  - But if a password is piped or directed in, this password will be used to create a new database with that password.
  - If Control-C is given, then the configuration can be adjusted:
    * Control-C from the **Login screen**: adjust the configuration file
    * Control-C from the **Index screen** (after decrypting):
      Also allow importing/exporting and re-encrypting the database with a different password or keyfile.
* Once unlocked, entries can be browsed, searched and opened from the Index screen.
* To open an existing vault, set `database` in the config file at `~/.config/kptui/config.toml`.
  - It can also be set on the Configuration screen that's accessible with Control-C from the Login screen.
  - If a keyfile is required, that can also be set in the Config screen with Control-C in the Login screen.
```toml
# Example
database = "~/passwords.kdbx"
keyfile = ""  # Empty means: no keyfile is set
kdf_iterations = 90  # The default
```
* The interface requires a terminal dimensions of at least 44 colums by 24 rows (but 24x12 is still functional).
* Run in smaller terminals with: `kptui slim` (at least 27 columns by 10 rows required, but 19x11 is still functional).
* To show the version: `kptui version`
* To show a short help: `kptui help`
* A password can also be provided non-interactively:
  - Piped in, like: `echo "$password" |kptui`
  - Directed in, like: `kptui <<<"$password"`
  - Use a file with the password: `kptui <password_file`

## Configuration
The configuration file is at a fixed location: `~/.config/kptui/config.toml`.
See `default_config.toml` in this repo:
```toml
database = "~/.local/share/kptui/default.kdbx"
keyfile = "~/.local/share/kptui/default.keyx"  # Optional (empty for password-only vaults)
kdf_iterations = 90  # Can be lower if less paranoid about being brute-forced
auto_exit = 300  # Seconds of inactivity before exiting the app
theme = "melange_dark.toml"  # Theme from ~/.local/share/kptui/themes
```

All fields are optional, as the above are the default values.

## Security notes
* Saved vaults are standard KDBX4.1 files, encrypted with the master password (and, when configured, the keyfile).
  - The keyfile is always in addition to the mandatory password, as an extra requirement.
  - If the configured keyfile is missing, unreadable, empty, or incorrect, an explicit error is given.
  - The keyfile's path is stored in the config, never its contents.
* Passwords can be piped in or entered in the UI, max.length 1024 bytes of UTF-8.
* Changing the master password re-encrypts the whole vault in-place, and requires entering the **current** password first.
  (If a keyfile was set, it remains active.)
* Changing the keyfile re-encrypts the whole vault in-place.
* The app exits after `auto_exit` seconds of inactivity, clearing all decrypted entries and the master password from memory.
  All edits and changes will be lost (0: never auto-exit).
* When the app is open and the vault not locked, secrets are kept in memory!
* When the app exits, there are no plaintext secrets left anywhere in memory.
* Using encrypted swap or zram (or no swap) increases security.
* Booting with kernel cmdline option `init_on_free=1` causes every freed page to get zeroed.
* On Linux, some extra hardening takes place in the operation of kptui.
* A GUI version of kptui is available on Linux, with the idea that no potentially secret text strings are somehow left (on ratatui or in screen buffers), because the GUI can be controlled completely by the app, so zeroizing can be more complete, at most leaving pixels in the shared buffer of the compositor (which Wayland completely wipes). Both X11 and Wayland zeroizes their text assembly buffers and their Cairo image on drop.
  - There is an issue with entering passwords in X11 (not just for `kpgui` or `kptui`, for any application!) that any process running on the same X-server (or as the same user, because that process could get access to the same X-server) can in principle access any keystroke, so any other GUI app that is running at the same time. Wayland does not have this issue. So beware what you have running!
  - Using `kptui` on a text console does enhance security.

### Optional Linux GUI
The GNU/Linux `gui` feature opens the existing vault interface on native Wayland or X11, using the same configuration and vault operations.
* It uses PangoCairo for text shaping and supports XKB layout switching.
* Besides the keyboard shortcuts, field labels in the Config, Index or Entry are left-click selectable, and double-click activated.
* The font size can be changed by resizing the `kpgui` window (the last size is saved to `~/.config/kptui/gui-size`).
* To select text, mouse-drag select works, as well as double-click to select a whole line (both of these copy to the Primary clipboard, which can usually be pasted with middle-click). This operation is safer on Wayland!
* With no arguments, `kpgui` opens the GUI (an x86-64 gnu Linux build of kptui will accept `gui` as argument to open it). It uses native Wayland when `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set, and X11 otherwise. Set environment variables `KPGUI_BACKEND=x11` or `KPGUI_BACKEND=wayland` to choose explicitly. Use `kpgui tui` for the full terminal UI, `kpgui slim` for the compact UI, or `kpgui help` and `kpgui version`.

#### Building the GUI binary (dynamically linked)
To build the GUI (feature: "gui") on Debian/Ubuntu, install `pkg-config`, `libx11-dev`, `libxkbcommon-dev`, `libpango1.0-dev` (Pango 1.44 or newer), `libcairo2-dev`, and from an x86-64 glibc Linux desktop, run: `cargo gui` to build it.

#### Building release packages
Run `./buildpkgs` from the checkout on Debian or Ubuntu to build both `.deb` and `.rpm` packages in `dist/`. It builds `kpgui` once and requires the GUI build dependencies above, plus Python 3, `dpkg-dev`, and `rpm`. The RPM uses the shared libraries available on the build host, so verify it on each target distribution before publishing.
