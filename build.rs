//! Give the Windows executable its icon and its name.
//!
//! GPUI's Windows backend gives every window the icon stored in the
//! executable as resource 1, so embedding it here is what puts Riffly's icon
//! on the title bar, the taskbar and Alt-Tab, as well as on the .exe in
//! Explorer. The icon is drawn by `tools/make_icon.py`.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/riffly.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/icon/riffly.ico")
        .set("ProductName", "Riffly")
        .set("FileDescription", "Riffly");
    if let Err(error) = resource.compile() {
        // A missing resource compiler costs the icon, not the build.
        println!("cargo:warning=the Windows icon was not embedded: {error}");
    }
}
