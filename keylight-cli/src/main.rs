use clap::{Parser, Subcommand};
use keylight::{Keylight, KeylightConfig, LicenseState};

#[derive(Parser)]
#[command(
    name = "keylight",
    version,
    about = "License your CLI/app with Keylight"
)]
struct Cli {
    #[arg(long, env = "KEYLIGHT_TENANT")]
    tenant: String,
    #[arg(long, env = "KEYLIGHT_PRODUCT")]
    product: String,
    #[arg(long, env = "KEYLIGHT_SDK_KEY")]
    sdk_key: Option<String>,
    #[arg(
        long,
        env = "KEYLIGHT_BASE_URL",
        default_value = "https://api.keylight.dev"
    )]
    base_url: String,
    #[arg(long, help = "Trusted key as kid=base64pub; repeatable")]
    trusted_key: Vec<String>,
    #[arg(
        long,
        help = "Fetch the trusted keyset from the server instead of --trusted-key"
    )]
    fetch_keys: bool,
    #[arg(long)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Activate { key: String },
    Validate,
    Deactivate,
    Status,
    Info,
}

fn build(cli: &Cli) -> Keylight {
    let mut b = KeylightConfig::builder(&cli.tenant, &cli.product).base_url(&cli.base_url);
    if let Some(k) = &cli.sdk_key {
        b = b.sdk_key(k);
    }
    for tk in &cli.trusted_key {
        if let Some((kid, pk)) = tk.split_once('=') {
            b = b.trusted_key(kid, pk);
        }
    }
    let mut cfg = b.build();
    if cli.fetch_keys {
        if let Some((_, keys)) = keylight::keyset::fetch_keyset(
            &keylight::http::ureq_transport::UreqTransport::default(),
            &cfg.base_url,
            &cfg.tenant_id,
        ) {
            cfg.trusted_keys.extend(keys);
        }
    }
    Keylight::new(cfg).expect("init store")
}

fn main() {
    let cli = Cli::parse();
    let kl = build(&cli);
    match &cli.cmd {
        Cmd::Activate { key } => {
            let r = kl.activate(key).expect("activate");
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"activated": r.activated, "error": r.error})
                );
            } else if r.activated {
                println!("Activated. Instance: {}", r.instance_id.unwrap_or_default());
            } else {
                eprintln!("Activation failed: {}", r.error.unwrap_or_default());
                std::process::exit(1);
            }
        }
        Cmd::Validate => {
            let r = kl.validate().expect("validate");
            println!("valid={}", r.valid);
            if !r.valid {
                std::process::exit(1);
            }
        }
        Cmd::Deactivate => {
            kl.deactivate().expect("deactivate");
            println!("Deactivated.");
        }
        Cmd::Status => {
            let s = kl.state();
            let label = match s {
                LicenseState::Licensed => "licensed",
                LicenseState::Trial { .. } => "trial",
                LicenseState::Limited => "limited",
                LicenseState::FreeTier => "free_tier",
                LicenseState::Expired => "expired",
                LicenseState::Invalid => "invalid",
            };
            if cli.json {
                println!("{}", serde_json::json!({"state": label}));
            } else {
                println!("State: {label}");
            }
        }
        Cmd::Info => {
            println!(
                "tenant={} product={} stored_license={}",
                cli.tenant,
                cli.product,
                kl.has_stored_license()
            );
        }
    }
}
