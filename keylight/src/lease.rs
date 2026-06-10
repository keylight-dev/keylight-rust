use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub kid: String,
    #[serde(rename = "licenseKeyHash")]
    pub license_key_hash: String,
    #[serde(rename = "instanceId")]
    pub instance_id: String,
    #[serde(rename = "issuedAt")]
    pub issued_at: i64,
    #[serde(rename = "expiresAt")]
    pub expires_at: i64,
    pub status: String,
    pub signature: String,
    pub entitlements: Vec<String>,
}

impl Lease {
    /// The exact UTF-8 payload that was signed (entitlements re-sorted ascending).
    pub fn payload(&self) -> String {
        let mut ents = self.entitlements.clone();
        ents.sort();
        format!(
            "v3|{}|{}|{}|{}|{}|{}|{}",
            self.kid,
            self.license_key_hash,
            self.instance_id,
            self.issued_at,
            self.expires_at,
            self.status,
            ents.join(",")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_sorts_entitlements_and_matches_v3_format() {
        let l = Lease {
            kid: "k1".into(),
            license_key_hash: "abc".into(),
            instance_id: "i1".into(),
            issued_at: 100,
            expires_at: 200,
            status: "active".into(),
            signature: "sig".into(),
            entitlements: vec!["pro".into(), "admin".into()],
        };
        assert_eq!(l.payload(), "v3|k1|abc|i1|100|200|active|admin,pro");
    }
    #[test]
    fn payload_empty_entitlements_is_trailing_empty() {
        let l = Lease {
            kid: "k1".into(),
            license_key_hash: "abc".into(),
            instance_id: "i1".into(),
            issued_at: 100,
            expires_at: 200,
            status: "active".into(),
            signature: "sig".into(),
            entitlements: vec![],
        };
        assert_eq!(l.payload(), "v3|k1|abc|i1|100|200|active|");
    }
}
