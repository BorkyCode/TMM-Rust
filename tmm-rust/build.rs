fn main() {
    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/AppIcon.ico");
        res.set("FileDescription", "Tera Mod Manager"); 
        res.set("ProductName", "TMM-Rust");
        res.set("CompanyName", "BorkyCode");

        // ✅ Set copyright field
        res.set("LegalCopyright", "© 2026 BorkyCode. All rights reserved.");
        res.set("FileVersion", "1.0.0.0");
        res.set("ProductVersion", "1.0.0.0");

        res.compile().unwrap();
    }
}