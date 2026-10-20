//! Embeds the app icon and version information into DropReel.exe.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "DropReel")
        .set("FileDescription", "DropReel — movie library and player")
        .set("CompanyName", "DropReel")
        .set("LegalCopyright", "MIT License — see LICENSE");
    if let Err(e) = res.compile() {
        // A missing resource compiler should not break the build; the exe just has no icon.
        println!("cargo:warning=could not embed the Windows icon: {e}");
    }
}
