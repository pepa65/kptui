[![version](https://img.shields.io/crates/v/kptui.svg)](https://crates.io/crates/kptui)
[![build](https://github.com/pepa65/kptui/actions/workflows/rust.yml/badge.svg)](https://github.com/pepa65/kptui/actions/workflows/rust.yml)
[![dependencies](https://deps.rs/repo/github/pepa65/kptui/status.svg)](https://deps.rs/repo/github/pepa65/kptui)
[![docs](https://img.shields.io/badge/docs-kptui-blue.svg)](https://docs.rs/crate/kptui/latest)
[![license](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://github.com/pepa65/kptui/blob/main/LICENSE)
[![downloads](https://img.shields.io/crates/d/kptui.svg)](https://crates.io/crates/kptui)

# kptui 0.25.0
**Highly secure TUI password manager for [KeePass](https://keepass.info/) KDBX vaults**
* Compatible with the PassKeeper/KeePass KDBX `.kdb` and `.kdbx` file format.
* It saves its database file to `KDBX4.1` which can be used by KeePassXC and KeePass2.
* Fits in with the wider PassKeeper/KeePass ecosystem.
* Very secure in its default configuration, impossible to brute-force (so unlocking takes a few seconds!).
  - Writes to `KDBX4.1` hardened with Argon2.
  - KDF-Memory: `262144 KiB`
  - KDF-iterations: `90` (!)
  - KDF-parallel: `4`
* The Terminal User Interface is responsive and fully keyboard-driven.
* Stores: `Name`, `Username`, `Password`, `TOTP`, `URL` and `Notes` (only what it needs).

## Features
* **KDBX4.1 vaults**: Reads and writes standard `.kdbx` files, so you can open the same vault in
  KeePassXC, KeePassDX, KeePass2 or any other compatible client.
* **Optional keyfile support**: Can unlocks vaults protected by a master password plus a keyfile,
  but password-only vault is also supported.
* **Filtered search**: Start typing on the index screen to filter entries by Name or Username.
* **TOTP codes**: Generates live 2FA codes from a stored seed and saves `otpauth://` URI.
  - Editable as: `SECRET [DIGITS [PERIOD]]` (`PERIOD` defaults to `30`, and `DIGITS` to `6`)
* **Auto-exit**: The whole app can close itself automatically after a set period of inactivity
  in seconds (`0`: stays open indefinitely).
* **Themeable**: Ships with a range of themes and supports custom themes.
* **Access Configuration from the app**: On the Login screen or the Index screen, give Control-C:
  - Database (default: `~/.local/share/kptui/default.kdbx`)
  - (Optional) keyfile
  - Auto-exit setting (in seconds, `0`: never auto-exit)
  - Theme selection (location: `~/.config/kptui/themes`)
  - Change the master password
  - Import from file
  - Export to file
* **Import from other vaults or export**: Import entries from another `.kdbx` file, or from a `.csv`/`.json` file
  (as produced by other password managers). Export to CSV/JSON or KDBX.

## Usage
```
kptui 0.25.0 - TUI password manager for KeePass vaults
Usage:  kptui [slim | version | help]
```

## Installing
### Download static single-binary
```
wget https://github.com/pepa65/kptui/releases/download/0.25.0/kptui
sudo mv kptui /usr/local/bin
sudo chown root:root /usr/local/bin/kptui
sudo chmod +x /usr/local/bin/kptui
```

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
  - But if a password is piped or directed in, this password will be used to
    create a new database with that password.
  - If Control-C is given, then the configuration can be adjusted.
* Once unlocked, entries can be browsed, searched and opened from the Index screen.
* To open an existing vault, set `default_database` in the config file at `~/.config/kptui/config.toml`.
  - It can also be set on the Configuration screen that's accessible with Control-C.
  - If a keyfile is required, that can also be set in the configuration.
```toml
# Example
default_database = "~/passwords.kdbx"
keyfile = "~/passwords.keyx"
```
* The interface requires a terminal dimensions of at least 44 colums by 23 rows (but 12 rows is still functional).
* Run in smaller terminals with: `kptui slim` (at least 39 columns by 5 rows required, but 22 x 11 is still functional).
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
default_database = "~/.local/share/kptui/default.kdbx"
keyfile = "~/.local/share/kptui/default.keyx"  # Optional (omit for password-only vaults)
auto_exit = 300  # Seconds of inactivity before exiting the app
theme = "melange_dark.toml"  # Theme from ~/.local/share/kptui/themes
```

All fields are optional, as the above are the default values.

## Security notes
* Saved vaults are standard KDBX4.1 files, encrypted with the master password (and, when configured, the keyfile).
  - The keyfile is always in addition to the mandatory password, as an extra requirement.
  - If the configured keyfile is missing, unreadable, empty, or incorrect, an explicit error is given.
  - The keyfile's path is stored in the config, never its contents.
* Changing the master password re-encrypts the whole vault in place, and requires entering the **current** password first.
  - If a keyfile was set, it remains part of the new composite key.
* The app exits after `auto_exit` seconds of inactivity, clearing all decrypted entries and the master password from memory.
  All edits and changes will be lost (0: never auto-exit).
* When the app is open and the vault not locked, secrets are kept in memory!
* When the app exits, there are no plaintext secrets left anywhere in memory.
