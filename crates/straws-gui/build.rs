//! On Windows, puts the icon into the program file itself (shown in
//! Explorer, the taskbar and shortcuts).

fn main() {
    println!("cargo:rerun-if-changed=icons/straws.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/straws.ico");
        res.set("ProductName", "STRAWS");
        res.set("FileDescription", "STRAWS antenna modeller");
        if let Err(e) = res.compile() {
            println!("cargo:warning=couldn't add the icon to the program: {e}");
        }
    }
}
