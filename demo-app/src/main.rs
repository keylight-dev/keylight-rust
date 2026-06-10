use clap::{Parser, Subcommand};
use keylight::{Keylight, KeylightConfig};
use std::io::Write;

#[derive(Parser)] struct Cli { #[command(subcommand)] cmd: Cmd }
#[derive(Subcommand)] enum Cmd { Activate { key: String }, Add { text: String }, List, Export { path: String } }

fn kl() -> Keylight {
    let mut cfg = KeylightConfig::builder("keylight-notes-demo", "notes").key_prefix("NOTES").build();
    if let Some((_, keys)) = keylight::keyset::fetch_keyset(&keylight::http::ureq_transport::UreqTransport::default(), &cfg.base_url, &cfg.tenant_id) {
        cfg.trusted_keys.extend(keys);
    }
    Keylight::new(cfg).expect("init")
}
fn notes_path() -> std::path::PathBuf { std::env::temp_dir().join("keylight-notes.txt") }
fn load() -> Vec<String> { std::fs::read_to_string(notes_path()).ok().map(|s| s.lines().map(String::from).collect()).unwrap_or_default() }
fn save(n: &[String]) { let _ = std::fs::write(notes_path(), n.join("\n")); }

fn main() {
    let cli = Cli::parse();
    let k = kl();
    match cli.cmd {
        Cmd::Activate { key } => { let r = k.activate(&key).expect("activate"); println!("{}", if r.activated { "Pro unlocked!" } else { "Activation failed" }); }
        Cmd::Add { text } => {
            let mut n = load();
            let pro = k.has_entitlement("pro");
            if !pro && n.len() >= 3 { eprintln!("Free tier is limited to 3 notes. Activate a Pro key to add more."); std::process::exit(1); }
            n.push(text); save(&n); println!("Added ({} notes).", n.len());
        }
        Cmd::List => { for (i, note) in load().iter().enumerate() { println!("{}. {note}", i + 1); } }
        Cmd::Export { path } => {
            if !k.has_entitlement("pro") { eprintln!("Export is a Pro feature. Activate a Pro key."); std::process::exit(1); }
            let mut f = std::fs::File::create(&path).expect("create"); f.write_all(load().join("\n").as_bytes()).expect("write");
            println!("Exported to {path}");
        }
    }
}
