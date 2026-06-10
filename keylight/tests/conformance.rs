use keylight::{verify_lease, Lease};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Corpus {
    #[serde(rename = "skewSeconds")]
    skew_seconds: i64,
    vectors: Vec<Vector>,
}
#[derive(Deserialize)]
struct Vector {
    name: String,
    lease: Lease,
    #[serde(rename = "trustedKeys")]
    trusted_keys: HashMap<String, String>,
    now: i64,
    expect: Expect,
}
#[derive(Deserialize, Debug, PartialEq, Eq)]
struct Expect {
    #[serde(rename = "kidKnown")]
    kid_known: bool,
    #[serde(rename = "signatureValid")]
    signature_valid: bool,
    expired: bool,
}

#[test]
fn passes_all_sp0_conformance_vectors() {
    let raw = include_str!("fixtures/vectors.json");
    let corpus: Corpus = serde_json::from_str(raw).expect("parse vectors.json");
    assert!(!corpus.vectors.is_empty());
    for v in &corpus.vectors {
        let r = verify_lease(&v.lease, &v.trusted_keys, v.now, corpus.skew_seconds);
        let got = Expect {
            kid_known: r.kid_known,
            signature_valid: r.signature_valid,
            expired: r.expired,
        };
        assert_eq!(got, v.expect, "vector failed: {}", v.name);
    }
}
