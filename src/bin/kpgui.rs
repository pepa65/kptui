#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
compile_error!("kpgui requires a GNU/Linux target; use --target x86_64-unknown-linux-gnu or aarch64-unknown-linux-gnu");

// Both binaries use the same vault, TUI and GUI implementation.
include!("../main.rs");
