//! [`KeylightConfig`] and its builder, including client-side key-format validation.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct KeylightConfig {
    pub tenant_id: String,
    pub product_id: String,
    pub sdk_key: Option<String>,
    pub trusted_keys: HashMap<String, String>, // kid -> raw ed25519 pub (base64)
    pub max_offline_days: Option<u32>,
    pub trial_duration_days: u32,
    pub free_tier_enabled: bool,
    pub app_version: Option<String>,
    pub base_url: String, // default https://api.keylight.dev
    pub key_prefix: Option<String>,
}

impl KeylightConfig {
    pub fn builder(
        tenant_id: impl Into<String>,
        product_id: impl Into<String>,
    ) -> KeylightConfigBuilder {
        KeylightConfigBuilder {
            tenant_id: tenant_id.into(),
            product_id: product_id.into(),
            sdk_key: None,
            trusted_keys: HashMap::new(),
            max_offline_days: None,
            trial_duration_days: 14,
            free_tier_enabled: false,
            app_version: None,
            base_url: "https://api.keylight.dev".into(),
            key_prefix: None,
        }
    }
    /// Client-side key format check (parity with Swift validateKeyFormat).
    pub fn validate_key_format(&self, key: &str) -> bool {
        let k = key.trim();
        if k.is_empty() {
            return false;
        }
        if let Some(prefix) = &self.key_prefix {
            if !k.to_uppercase().starts_with(&prefix.to_uppercase()) {
                return false;
            }
        }
        k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }
}

pub struct KeylightConfigBuilder {
    tenant_id: String,
    product_id: String,
    sdk_key: Option<String>,
    trusted_keys: HashMap<String, String>,
    max_offline_days: Option<u32>,
    trial_duration_days: u32,
    free_tier_enabled: bool,
    app_version: Option<String>,
    base_url: String,
    key_prefix: Option<String>,
}

impl KeylightConfigBuilder {
    pub fn sdk_key(mut self, v: impl Into<String>) -> Self {
        self.sdk_key = Some(v.into());
        self
    }
    pub fn trusted_key(mut self, kid: impl Into<String>, pub_b64: impl Into<String>) -> Self {
        self.trusted_keys.insert(kid.into(), pub_b64.into());
        self
    }
    pub fn max_offline_days(mut self, d: u32) -> Self {
        self.max_offline_days = Some(d);
        self
    }
    pub fn trial_duration_days(mut self, d: u32) -> Self {
        self.trial_duration_days = d;
        self
    }
    pub fn free_tier_enabled(mut self, v: bool) -> Self {
        self.free_tier_enabled = v;
        self
    }
    pub fn app_version(mut self, v: impl Into<String>) -> Self {
        self.app_version = Some(v.into());
        self
    }
    pub fn base_url(mut self, v: impl Into<String>) -> Self {
        self.base_url = v.into();
        self
    }
    pub fn key_prefix(mut self, v: impl Into<String>) -> Self {
        self.key_prefix = Some(v.into());
        self
    }
    pub fn build(self) -> KeylightConfig {
        KeylightConfig {
            tenant_id: self.tenant_id,
            product_id: self.product_id,
            sdk_key: self.sdk_key,
            trusted_keys: self.trusted_keys,
            max_offline_days: self.max_offline_days,
            trial_duration_days: self.trial_duration_days,
            free_tier_enabled: self.free_tier_enabled,
            app_version: self.app_version,
            base_url: self.base_url,
            key_prefix: self.key_prefix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builder_defaults() {
        let c = KeylightConfig::builder("t", "p").build();
        assert_eq!(c.base_url, "https://api.keylight.dev");
        assert_eq!(c.trial_duration_days, 14);
        assert!(!c.free_tier_enabled);
    }
    #[test]
    fn key_format_respects_prefix() {
        let c = KeylightConfig::builder("t", "p")
            .key_prefix("NOTES")
            .build();
        assert!(c.validate_key_format("NOTES-PRO0-0000-0001"));
        assert!(!c.validate_key_format("WRONG-0000"));
        assert!(!c.validate_key_format(""));
    }
}
