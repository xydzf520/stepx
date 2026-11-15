use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // The framework reads resource 0x101 for the Windows window/taskbar icon.
    // Embedding it also gives the portable executable its own Explorer icon.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let [major, minor, patch] = [
            "CARGO_PKG_VERSION_MAJOR",
            "CARGO_PKG_VERSION_MINOR",
            "CARGO_PKG_VERSION_PATCH",
        ]
        .map(|name| {
            std::env::var(name)
                .unwrap_or_else(|_| panic!("{name} is missing"))
                .parse::<u16>()
                .unwrap_or_else(|_| panic!("{name} must fit in a Windows version field"))
        });
        let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
        let file_version_def = format!("STEPX_FILEVERSION={major},{minor},{patch},0");
        let version_text_def = format!("STEPX_VERSION_TEXT={version}");
        embed_resource::compile_for(
            "assets/stepx_companion.rc",
            ["stepx-companion"],
            [file_version_def.as_str(), version_text_def.as_str()],
        )
        .manifest_required()
        .expect("compile StepX Companion Windows icon");
    }
    // Track application sources/assets so the timestamp changes with the binary,
    // while an unchanged build keeps its original timestamp. No host `date` tool
    // or host timezone is needed, including when cross-compiling.
    for path in ["build.rs", "Cargo.toml", "Cargo.lock", "src", "assets"] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let seconds = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => value
            .parse::<u64>()
            .expect("SOURCE_DATE_EPOCH must be a non-negative Unix timestamp in seconds"),
        Err(std::env::VarError::NotPresent) => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("build clock must be after the Unix epoch")
            .as_secs(),
        Err(error) => panic!("cannot read SOURCE_DATE_EPOCH: {error}"),
    };
    assert!(
        seconds <= 253_402_271_999,
        "build timestamp exceeds year 9999 in UTC+8"
    );
    println!("cargo:rustc-env=STEPX_BUILD_UNIX_SECONDS={seconds}");
}
