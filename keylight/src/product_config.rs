//! Server-owned product settings: trial length and free tier.
//!
//! These two values belong to the server, not to the build. The fields on
//! [`crate::KeylightConfig`] are demoted to a *seed*, used only before this
//! install has ever reached the server.
//!
//! Both fields are `Option`, and **absence is meaningful**: `None` means "never
//! heard from the server", which is a different thing from `Some(0)` /
//! `Some(false)`. A tenant who turns trials off in the dashboard sends a real
//! `0`; collapsing that into "absent" would fall back to the seed and silently
//! re-enable the trial they just disabled.

use serde::{Deserialize, Serialize};

/// The cached pair, as persisted in the store.
///
/// `skip_serializing_if` keeps a never-heard field out of the JSON entirely
/// rather than writing `null`, so the round-trip preserves the absent/zero
/// distinction that the whole feature turns on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedProductConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial_duration_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_tier_enabled: Option<bool>,
}

impl CachedProductConfig {
    pub fn is_empty(&self) -> bool {
        self.trial_duration_days.is_none() && self.free_tier_enabled.is_none()
    }
}

/// The wire shape, as it appears in the `/config` response body and riding on
/// `validate` and keyless-beacon responses.
///
/// Every field is optional because a worker predating this feature sends none
/// of them, and that must leave a cached value alone rather than overwrite it.
/// The signature fields are part of the frozen wire contract but are not
/// verified by this SDK yet — they are accepted and ignored so that adding
/// verification later is a change to this module alone.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProductConfigFields {
    #[serde(default)]
    pub trial_duration_days: Option<u32>,
    #[serde(default)]
    pub free_tier_enabled: Option<bool>,
    #[serde(default)]
    pub issued_at: Option<i64>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub kid: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
}

impl ProductConfigFields {
    /// True when the response carried neither setting — an older worker, or a
    /// route that simply has nothing to say about product config.
    pub fn is_empty(&self) -> bool {
        self.trial_duration_days.is_none() && self.free_tier_enabled.is_none()
    }
}
