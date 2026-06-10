use serde::Deserialize;
use std::collections::HashMap;

use crate::http::Transport;

#[derive(Deserialize)]
struct KeysetResponse { primary_kid: String, keys: Vec<KeyEntry> }
#[derive(Deserialize)]
struct KeyEntry { kid: String, public_key: String }

/// Parse a `.well-known/keylight-keys` JSON body into a kid->pub map + primary kid.
pub fn parse_keyset(json: &str) -> Option<(String, HashMap<String, String>)> {
    let r: KeysetResponse = serde_json::from_str(json).ok()?;
    let map = r.keys.into_iter().map(|k| (k.kid, k.public_key)).collect();
    Some((r.primary_kid, map))
}

/// Fetch and parse the tenant keyset from `{base}/{tenant}/.well-known/keylight-keys`.
pub fn fetch_keyset(
    transport: &dyn Transport,
    base_url: &str,
    tenant_id: &str,
) -> Option<(String, HashMap<String, String>)> {
    let url = format!("{base_url}/{tenant_id}/.well-known/keylight-keys");
    if let crate::http::TransportOutcome::Response(r) = transport.get(&url, &[]) {
        if r.status == 200 {
            return parse_keyset(&r.body);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_demo_keyset_shape() {
        let json = r#"{"primary_kid":"k1","keys":[{"kid":"k1","alg":"ed25519","public_key":"AAAA"}]}"#;
        let (primary, map) = parse_keyset(json).unwrap();
        assert_eq!(primary, "k1");
        assert_eq!(map.get("k1").unwrap(), "AAAA");
    }
}
