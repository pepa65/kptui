[![version](https://img.shields.io/crates/v/kptui.svg)](https://crates.io/crates/kptui)
[![build](https://github.com/pepa65/kptui/actions/workflows/rust.yml/badge.svg)](https://github.com/pepa65/kptui/actions/workflows/rust.yml)
[![dependencies](https://deps.rs/repo/github/pepa65/kptui/status.svg)](https://deps.rs/repo/github/pepa65/kptui)
[![docs](https://img.shields.io/badge/docs-kptui-blue.svg)](https://docs.rs/crate/kptui/latest)
[![license](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://github.com/pepa65/kptui/blob/main/LICENSE)
[![downloads](https://img.shields.io/crates/d/kptui.svg)](https://crates.io/crates/kptui)

# kptui 0.19.0
**TUI password manager for [KeePass](https://keepass.info/) vaults**

* Compatible with the `.kdbx` (4) file format.
* Fits in with the wider KeePass ecosystem.
* Fast to open.
* Keyboard-driven.
* Stores (only what it needs): `Name`, `Username`, `Password`, `TOTP`, `URL` and `Notes`

## Features
- **KDBX4 vaults**: reads and writes standard `.kdbx` files, so you can open the same vault in KeePassXC, KeePassDX, or any other compatible client.
- **Optional keyfile support**: unlocks vaults protected by a master password plus a KeePass keyfile while preserving password-only vault support.
- **Fuzzy search**: start typing on the index screen to filter entries by name or user.
- **TOTP codes**: generates live 2FA codes from a stored seed or `otpauth://` URI.
- **Reuse warnings**: flags entries that share a password or username with another entry, so you can spot weak spots at a glance.
- **Auto-lock**: the vault locks itself after a period of inactivity.
- **Themeable**: ships with a default color scheme and supports custom themes.
- **Change your master password**: from Settings, without needing to touch a file manager or another app.
- **Import from other vaults or exports**: pull entries in from another `.kdbx` file, or from a CSV/JSON export produced by another password manager.

## Installing
### Download static single-binary
```
wget https://github.com/pepa65/kptui/releases/download/0.19.0/kptui
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
* On first launch, if no vault is found at the configured path, `kptui` will:
  - Prompt to create a new database.
  - Prompt to set a master password.
  - But if a password is piped or directed in, this password will be used to
    create a new database with that password.
* Once unlocked, entries can be browsed, searched and opened from the index screen.
* To open an existing vault, set `default_database` in the config file at `~/.config/kptui/config.toml`.
  (A keyfile can also be set with `keyfile`, but the password is still required.)
```toml
# Example
default_database = "~/passwords.kdbx"
keyfile = "~/passwords.keyx"
```
* The interface requires a terminal dimensions of at least 44 colums by 23 rows (but 12 rows is still functional).
* Run in smaller terminals with: `kptui --slim` (at least 39 columns by 5 rows required, but 22 x 11 is still functional).
* To show the version: `kptui --version`
* To show a short help: `kptui --help`
* A password can also be provided non-interactively:
  - Piped in, like: `echo "$password" |kptui`
  - Directed in, like: `kptui <<<"$password"`
  - Use a file with the password: `kptui <password_file`

## Configuration
The configuration file is expected in a fixed location: `~/.config/kptui/config.toml`.
See `sample-config.toml` in this repo, or:
```toml
default_database = "~/.local/share/kptui/default.kdbx"
keyfile = "~/.local/share/kptui/default.keyx"  # Optional (omit for password-only vaults)
auto_lock = 300  # Seconds of inactivity before locking
theme = "melange_dark.toml"
```

All fields are optional, the above are the default values.

## Security notes
* Vaults are standard KDBX4 files, encrypted with the master password (and, when configured, the keyfile).
* The keyfile is always in addition to the mandatory password, as an extra requirement.
* The configured keyfile is read when unlocking, and if missing, unreadable, empty, or incorrect,
  an explicit error is given.
* The keyfile's path is stored in the config, never its contents.
* Changing the master password re-encrypts the whole vault in place, and requires entering the _current_ password first.
  For a keyfile-protected vault, the configured keyfile remains part of the new composite key.
* The app locks after `auto_lock` seconds of inactivity, clearing decrypted entries and the master password from memory.
  All edits and changes will be lost.
* When the app is open and the vault not locked, secrets are kept in memory!
* When the app exits, there are no plaintext secrets left anywhere in memory.
