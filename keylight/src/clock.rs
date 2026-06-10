const BACKWARD_TOLERANCE: i64 = 3600;       // 1h
const FORWARD_JUMP_LIMIT: i64 = 30 * 24 * 60 * 60; // 30d

/// True if the (last_seen, now) pair indicates clock tampering (parity with Swift).
pub fn clock_manipulated(last_seen: i64, now: i64) -> bool {
    let drift = last_seen - now;        // positive => clock went backward
    if drift > BACKWARD_TOLERANCE { return true; }
    if now - last_seen > FORWARD_JUMP_LIMIT { return true; }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn normal_is_fine() { assert!(!clock_manipulated(1000, 1100)); }
    #[test] fn backward_beyond_tolerance_flags() { assert!(clock_manipulated(10_000, 10_000 - 4000)); }
    #[test] fn forward_beyond_30d_flags() { assert!(clock_manipulated(0, 31 * 24 * 60 * 60)); }
}
