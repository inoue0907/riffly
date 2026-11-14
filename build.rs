//! Give the Windows executable its icon and its name.
//!
//! GPUI's Windows backend gives every window the icon stored in the
//! executable as resource 1, so embedding it here is what puts Riffly's icon
//! on the title bar, the taskbar and Alt-Tab, as well as on the .exe in
//! Explorer. The icons are drawn by `tools/make_icon.py`.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/icon/riffly.ico")
        // Icons for the files Riffly opens, by resource id; `shell.rs`
        // points each file type at one of these.
        .set_icon_with_id("assets/icon/riffly-doc.ico", "2")
        .set_icon_with_id("assets/icon/riffly-md.ico", "3")
        .set_icon_with_id("assets/icon/riffly-html.ico", "4")
        .set_icon_with_id("assets/icon/riffly-pdf.ico", "5")
        .set("ProductName", "Riffly")
        .set("FileDescription", "Riffly");
    if let Err(error) = resource.compile() {
        // A missing resource compiler costs the icon, not the build.
        println!("cargo:warning=the Windows icon was not embedded: {error}");
    }
}
