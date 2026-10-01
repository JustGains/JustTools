//! Compile the menu icons and version details into the extension.
//!
//! The resource script is generated so icon IDs come from `justtools_menu::Icon`
//! and the version from this crate, leaving nothing to keep in step by hand.

fn main() {
    println!("cargo:rerun-if-changed=../icons");
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        resources();
        // rustc exports the COM entry points by name; MSVC's reminder that
        // they are conventionally PRIVATE does not apply to a library that
        // nothing links against.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg-cdylib=/IGNORE:4104");
        }
    }
}

#[cfg(windows)]
fn resources() {
    use std::fmt::Write;
    use std::path::PathBuf;

    let icons = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../icons");
    let mut script = String::new();
    for icon in justtools_menu::Icon::ALL {
        let path = icons.join(format!("{}.ico", icon.file_stem()));
        let path = path.to_string_lossy().replace('\\', "/");
        writeln!(script, "{} ICON \"{path}\"", icon.resource_id()).unwrap();
    }
    let version = env!("CARGO_PKG_VERSION");
    let numbers = format!(
        "{},{},{},0",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    );
    write!(
        script,
        r#"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004L
FILETYPE 0x2L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "JustGains"
            VALUE "FileDescription", "JustTools File Explorer context menu"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "justtools_shell"
            VALUE "LegalCopyright", "MIT License"
            VALUE "OriginalFilename", "justtools_shell.dll"
            VALUE "ProductName", "JustTools"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    )
    .unwrap();
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("resources.rc");
    std::fs::write(&output, script).unwrap();
    embed_resource::compile(&output, embed_resource::NONE)
        .manifest_required()
        .expect("the context-menu icons must compile into the extension");
}
