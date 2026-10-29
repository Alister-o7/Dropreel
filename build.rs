//! Embeds the app icon and version information into Dropreel.exe.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // A fourth version number is Cargo build metadata ("0.3.5+1"): Windows shows "0.3.5.1".
    let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let shown = cargo_version.replace('+', ".");
    let number = |var: &str| -> u64 {
        std::env::var(var)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    let build: u64 = cargo_version
        .split_once('+')
        .and_then(|(_, b)| b.parse().ok())
        .unwrap_or(0);
    let numeric = number("CARGO_PKG_VERSION_MAJOR") << 48
        | number("CARGO_PKG_VERSION_MINOR") << 32
        | number("CARGO_PKG_VERSION_PATCH") << 16
        | build;
    let mut res = winresource::WindowsResource::new();
    res.set("FileVersion", &shown)
        .set("ProductVersion", &shown)
        .set_version_info(winresource::VersionInfo::FILEVERSION, numeric)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, numeric);
    res.set_icon("assets/icon.ico")
        .set("ProductName", "Dropreel")
        .set("FileDescription", "Dropreel — movie library and player")
        .set("CompanyName", "Dropreel")
        .set("LegalCopyright", "MIT License — see LICENSE");
    if let Err(e) = res.compile() {
        // A missing resource compiler should not break the build; the exe just has no icon.
        println!("cargo:warning=could not embed the Windows icon: {e}");
    }
}
