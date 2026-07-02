//! Snapshot-to-metrics rendering for operator observability.

mod values;

use crate::node::snapshot::NodeSnapshot;

pub(super) const CONNECTED_PEERS_METRIC: &str = "p2p_connected_peers";

/// Export operator counters in Prometheus text exposition format without opening an HTTP port.
/// Embedders that want an HTTP endpoint can serve this string from their own trusted admin server.
pub(crate) fn snapshot_to_prometheus_metrics(snapshot: &NodeSnapshot) -> String {
    let mut out = String::new();
    for (name, value) in values::snapshot_metric_values(snapshot) {
        emit(&mut out, name, &value);
    }
    out
}

fn emit(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push(' ');
    out.push_str(value);
    out.push('\n');
}
