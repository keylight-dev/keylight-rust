//! Heuristic detection of system-clock manipulation (backward/forward jumps).

const BACKWARD_TOLERANCE: i64 = 3600; // 1h
const FORWARD_JUMP_LIMIT: i64 = 30 * 24 * 60 * 60; // 30d

/// True if the (last_seen, now) pair indicates clock tampering (parity with Swift).
pub fn clock_manipulated(last_seen: i64, now: i64) -> bool {
    let drift = last_seen - now; // positive => clock went backward
    if drift > BACKWARD_TOLERANCE {
        return true;
    }
    if now - last_seen > FORWARD_JUMP_LIMIT {
        return true;
    }
    false
}

/// True if `now` is more than the backward tolerance behind `last_seen` — i.e. the
/// clock was rolled back since the last recorded contact. Unlike
/// [`clock_manipulated`], this omits the forward-jump/offline-ceiling component, so
/// it can gate the read-only [`crate::Keylight::state`] resolver without governing
/// offline duration (that stays with `max_offline_days`). Operates on UTC epoch
/// seconds, so timezone changes never trip it.
pub fn clock_rolled_back(last_seen: i64, now: i64) -> bool {
    last_seen - now > BACKWARD_TOLERANCE
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_is_fine() {
        assert!(!clock_manipulated(1000, 1100));
    }
    #[test]
    fn backward_beyond_tolerance_flags() {
        assert!(clock_manipulated(10_000, 10_000 - 4000));
    }
    #[test]
    fn forward_beyond_30d_flags() {
        assert!(clock_manipulated(0, 31 * 24 * 60 * 60));
    }

    #[test]
    fn rolled_back_only_flags_backward_jumps() {
        // Normal forward progress is fine.
        assert!(!clock_rolled_back(1000, 1100));
        // A long offline stretch (forward) is NOT a rollback — that is governed
        // by max_offline_days, not this guard.
        assert!(!clock_rolled_back(0, 60 * 24 * 60 * 60));
        // A backward jump beyond tolerance is a rollback.
        assert!(clock_rolled_back(10_000, 10_000 - 4000));
        // A small backward drift within tolerance is not.
        assert!(!clock_rolled_back(10_000, 10_000 - 100));
    }
}
