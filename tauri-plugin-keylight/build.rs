const COMMANDS: &[&str] = &["activate", "validate", "has_entitlement"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
