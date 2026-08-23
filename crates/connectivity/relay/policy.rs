use super::state::RelayServiceHealth;

pub fn classify_relay_denial(status_debug: &str) -> RelayServiceHealth {
    let lower = status_debug.to_ascii_lowercase();
    if lower.contains("rate") || lower.contains("thrott") {
        RelayServiceHealth::RateLimited
    } else if lower.contains("resource")
        || lower.contains("limit")
        || lower.contains("capacity")
        || lower.contains("too")
        || lower.contains("no reservation")
    {
        RelayServiceHealth::AtCapacity
    } else {
        RelayServiceHealth::Error
    }
}
