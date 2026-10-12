fn main() {
    println!("cargo:rerun-if-changed=../../assets/app-icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../assets/app-icon.ico")
        .canonicalize()
        .expect("application icon is missing");
    let resource = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("app-icon.rc");
    // GPUI loads resource 1 for the title bar/taskbar; Explorer uses it too.
    let icon = icon
        .to_str()
        .unwrap()
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/");
    std::fs::write(&resource, format!("1 ICON \"{icon}\"\n")).unwrap();
    embed_resource::compile_for(&resource, ["peek-gpui"], embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed the Windows application icon");
}
