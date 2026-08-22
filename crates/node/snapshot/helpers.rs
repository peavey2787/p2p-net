const MAX_SNAPSHOT_ADDRS: usize = 16;

pub(super) fn push_unique_recent(values: &mut Vec<String>, value: String) {
    values.retain(|existing| existing != &value);
    values.insert(0, value);
    if values.len() > MAX_SNAPSHOT_ADDRS {
        values.truncate(MAX_SNAPSHOT_ADDRS);
    }
}

pub(crate) fn network_label(network_id: u32) -> String {
    if network_id == 0 {
        "MAINNET".to_string()
    } else {
        format!("TESTNET-{network_id}")
    }
}
