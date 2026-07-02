//! Node observability metric renderers.

mod prometheus;

use super::snapshot::NodeSnapshot;

/// Export operator counters in Prometheus text exposition format without opening an HTTP port.
/// Embedders that want an HTTP endpoint can serve this string from their own trusted admin server.
pub fn snapshot_to_prometheus_metrics(snapshot: &NodeSnapshot) -> String {
    prometheus::snapshot_to_prometheus_metrics(snapshot)
}
