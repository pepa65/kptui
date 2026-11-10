fn main() {
	#[cfg(feature = "gui")]
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
		pkg_config::probe_library("pangocairo").expect("Install PangoCairo development files for the target");
		pkg_config::probe_library("x11").expect("Install libX11 development files for the target");
	}
}
