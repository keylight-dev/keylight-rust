//! `upgrade_url()` — the hosted page a customer is sent to in order to upgrade.
//!
//! The URL it used to build, `portal.keylight.dev/p/{tenant}/upgrade/{product}`,
//! is a retired route: the portal's only public page is now `/p/{tenant}/claim/
//! {product}`, and upgrading happens behind a magic-link sign-in. A customer
//! following the old link lands on a 404 rather than on a checkout.
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{LicenseStore, account};
use keylight::{Keylight, KeylightConfig};
use std::sync::Arc;

mod common {
    use keylight::http::{HttpResponse, Transport, TransportOutcome};
    pub struct Noop;
    impl Transport for Noop {
        fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
            TransportOutcome::Response(HttpResponse {
                status: 200,
                body: "{}".into(),
                retry_after: None,
            })
        }
        fn get(&self, _: &str, _: &[(String, String)]) -> TransportOutcome {
            TransportOutcome::Response(HttpResponse {
                status: 200,
                body: "{}".into(),
                retry_after: None,
            })
        }
    }
}

fn client_with_key(dir: &str, key: Option<&str>) -> Keylight {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    if let Some(k) = key {
        store.set_string(account::LICENSE_KEY, k).unwrap();
    }
    let cfg = KeylightConfig::builder("acme", "app1", "sdk_live_test").build();
    Keylight::with_parts(cfg, store, Arc::new(common::Noop))
}

/// The portal identifies a license by its NORMALIZED key — that path segment is
/// matched against `licenses.normalizedKey` server-side — so the SDK has to
/// normalize before building the link, not pass the display key through.
#[test]
fn upgrade_url_points_at_the_authenticated_license_route() {
    let kl = client_with_key("kl-upgrade-1", Some("ACME-XXXX-YYYY-ZZZZ"));
    assert_eq!(
        kl.upgrade_url().unwrap(),
        "https://portal.keylight.dev/t/acme/license/ACMEXXXXYYYYZZZZ/upgrade"
    );
}

/// Normalization is the server's rule: strip whitespace and dashes, uppercase.
/// A key typed with spaces or in lowercase must resolve to the same license.
#[test]
fn upgrade_url_normalizes_whitespace_and_case() {
    let kl = client_with_key("kl-upgrade-2", Some(" acme-xxxx yyyy-zzzz "));
    assert_eq!(
        kl.upgrade_url().unwrap(),
        "https://portal.keylight.dev/t/acme/license/ACMEXXXXYYYYZZZZ/upgrade"
    );
}

/// The product id has no place in the new route: the license itself carries the
/// product server-side, and the old URL shape is gone.
#[test]
fn upgrade_url_carries_no_product_segment_and_no_key_query() {
    let kl = client_with_key("kl-upgrade-3", Some("ACME-XXXX-YYYY-ZZZZ"));
    let url = kl.upgrade_url().unwrap();
    assert!(!url.contains("app1"), "product must not appear: {url}");
    assert!(
        !url.contains('?'),
        "the key rides in the path, not a query: {url}"
    );
    assert!(
        !url.contains("/p/"),
        "the public route is claim-only now: {url}"
    );
}

#[test]
fn upgrade_url_is_none_without_a_stored_key() {
    let kl = client_with_key("kl-upgrade-4", None);
    assert!(kl.upgrade_url().is_none());
}
