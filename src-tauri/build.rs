fn main() {
    // tauri-build only watches tauri.conf.json/capabilities, so a changed icon
    // would otherwise keep the old one embedded in the exe resources.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
