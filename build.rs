fn main() {
    println!("cargo:rerun-if-changed=explorer/icons/brand.ico");
    println!("cargo:rerun-if-changed=explorer/icons/paste.ico");
    println!("cargo:rerun-if-changed=explorer/app.manifest");
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let version = env!("CARGO_PKG_VERSION");
        let numbers = format!(
            "{},{},{},0",
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR"),
            env!("CARGO_PKG_VERSION_PATCH")
        );
        let script = format!(
            r#"101 ICON "{}"
116 ICON "{}"
1 24 "{}"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004L
FILETYPE 0x1L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "JustGains"
            VALUE "FileDescription", "JustTools command-line tools"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "just"
            VALUE "LegalCopyright", "MIT License"
            VALUE "OriginalFilename", "just.exe"
            VALUE "ProductName", "JustTools"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
            root.join("explorer/icons/brand.ico")
                .display()
                .to_string()
                .replace('\\', "/"),
            root.join("explorer/icons/paste.ico")
                .display()
                .to_string()
                .replace('\\', "/"),
            root.join("explorer/app.manifest")
                .display()
                .to_string()
                .replace('\\', "/"),
        );
        let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("icons.rc");
        std::fs::write(&path, script).unwrap();
        embed_resource::compile(path, embed_resource::NONE)
            .manifest_required()
            .expect("compile JustTools icons");
    }
}
