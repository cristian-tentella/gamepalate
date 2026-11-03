fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        let mut resource = tauri_winres::WindowsResource::new();
        resource
            .set_icon("gamepalate.ico")
            .set("ProductName", "gamepalate")
            .set("FileDescription", "gamepalate")
            .set("InternalName", "gamepalate")
            .set("OriginalFilename", "gamepalate.exe")
            .set("LegalCopyright", "© 2026 Cristian Tentella");
        resource
            .compile()
            .expect("the Windows resources should compile");
    }
}
