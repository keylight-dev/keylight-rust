#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseState {
    Trial { days_left: i64 },
    Licensed,
    Limited,
    FreeTier,
    Expired,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrialStatus {
    NotStarted,
    Active { days_left: i64 },
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeylessState {
    Trial,
    FreeTier,
    Expired,
}
impl KeylessState {
    pub fn wire(&self) -> &'static str {
        match self {
            Self::Trial => "trial",
            Self::FreeTier => "free_tier",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseLifecycleEvent {
    Renewed,
    Cancelled,
    Expired,
    Restored,
}

/// Resolve the high-level state from inputs (pure; mirrors Swift LicenseManager).
/// `lease_status`: Some("active"|"fallback"|"expired") if a *signature-valid* cached
/// lease exists; `lease_current`: whether it is within skew. `had_license`: a key is stored.
pub fn resolve_state(
    lease_status: Option<&str>,
    lease_current: bool,
    had_license: bool,
    trial: &TrialStatus,
    free_tier_enabled: bool,
) -> LicenseState {
    if let Some(status) = lease_status {
        match (status, lease_current) {
            ("active", true) => return LicenseState::Licensed,
            ("fallback", _) => return LicenseState::Limited,
            ("expired", _) => return LicenseState::Expired,
            (_, false) => {} // stale active lease falls through to offline/expired handling
            _ => {}
        }
    }
    if had_license {
        return LicenseState::Expired;
    }
    match trial {
        TrialStatus::Active { days_left } => LicenseState::Trial {
            days_left: *days_left,
        },
        _ if free_tier_enabled => LicenseState::FreeTier,
        _ => LicenseState::Invalid,
    }
}

pub fn lifecycle_event(
    prev: &LicenseState,
    next: &LicenseState,
    expiry_moved_later: bool,
) -> Option<LicenseLifecycleEvent> {
    use LicenseState::*;
    match (prev, next) {
        (Licensed, Licensed) if expiry_moved_later => Some(LicenseLifecycleEvent::Renewed),
        (Licensed, Expired) | (Licensed, Limited) => Some(LicenseLifecycleEvent::Cancelled),
        (Expired, Licensed) | (Limited, Licensed) | (Invalid, Licensed) => {
            Some(LicenseLifecycleEvent::Restored)
        }
        (_, Expired) if !matches!(prev, Expired) => Some(LicenseLifecycleEvent::Expired),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_current_lease_is_licensed() {
        assert_eq!(
            resolve_state(Some("active"), true, true, &TrialStatus::NotStarted, false),
            LicenseState::Licensed
        );
    }
    #[test]
    fn fallback_is_limited() {
        assert_eq!(
            resolve_state(
                Some("fallback"),
                true,
                true,
                &TrialStatus::NotStarted,
                false
            ),
            LicenseState::Limited
        );
    }
    #[test]
    fn no_license_trial_active_is_trial() {
        assert_eq!(
            resolve_state(
                None,
                false,
                false,
                &TrialStatus::Active { days_left: 5 },
                false
            ),
            LicenseState::Trial { days_left: 5 }
        );
    }
    #[test]
    fn no_license_free_tier_is_free_tier() {
        assert_eq!(
            resolve_state(None, false, false, &TrialStatus::NotStarted, true),
            LicenseState::FreeTier
        );
    }
    #[test]
    fn nothing_is_invalid() {
        assert_eq!(
            resolve_state(None, false, false, &TrialStatus::NotStarted, false),
            LicenseState::Invalid
        );
    }
    #[test]
    fn keyless_wire_strings() {
        assert_eq!(KeylessState::FreeTier.wire(), "free_tier");
    }

    use LicenseLifecycleEvent as E;
    use LicenseState as S;
    #[test]
    fn renewed_when_licensed_and_expiry_later() {
        assert_eq!(
            lifecycle_event(&S::Licensed, &S::Licensed, true),
            Some(E::Renewed)
        );
        assert_eq!(lifecycle_event(&S::Licensed, &S::Licensed, false), None);
    }
    #[test]
    fn cancelled_on_licensed_to_expired_or_limited() {
        assert_eq!(
            lifecycle_event(&S::Licensed, &S::Expired, false),
            Some(E::Cancelled)
        );
        assert_eq!(
            lifecycle_event(&S::Licensed, &S::Limited, false),
            Some(E::Cancelled)
        );
    }
    #[test]
    fn restored_on_recovery_to_licensed() {
        assert_eq!(
            lifecycle_event(&S::Expired, &S::Licensed, false),
            Some(E::Restored)
        );
        assert_eq!(
            lifecycle_event(&S::Limited, &S::Licensed, false),
            Some(E::Restored)
        );
        assert_eq!(
            lifecycle_event(&S::Invalid, &S::Licensed, false),
            Some(E::Restored)
        );
    }
    #[test]
    fn expired_when_crossing_into_expired_from_non_expired() {
        assert_eq!(
            lifecycle_event(&S::Trial { days_left: 1 }, &S::Expired, false),
            Some(E::Expired)
        );
    }
    #[test]
    fn no_event_on_noop_transitions() {
        assert_eq!(
            lifecycle_event(
                &S::Trial { days_left: 3 },
                &S::Trial { days_left: 2 },
                false
            ),
            None
        );
        assert_eq!(lifecycle_event(&S::Expired, &S::Expired, false), None);
    }
}
