fn main() {
    println!("cargo:rerun-if-changed=explorer/icons/brand.ico");
    println!("cargo:rerun-if-changed=explorer/icons/paste.ico");
    println!("cargo:rerun-if-changed=explorer/app.manifest");
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let script = format!(
            "101 ICON \"{}\"\n116 ICON \"{}\"\n1 24 \"{}\"\n",
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
