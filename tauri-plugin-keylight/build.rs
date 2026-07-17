const COMMANDS: &[&str] = &[
    "activate",
    "validate",
    "has_entitlement",
    "check_on_launch",
    "refresh_if_needed",
    "report_keyless_state",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
