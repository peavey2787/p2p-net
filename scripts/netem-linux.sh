#!/usr/bin/env bash
set -euo pipefail

iface="${1:-lo}"
mode="${2:-status}"

case "$mode" in
  start)
    sudo tc qdisc replace dev "$iface" root netem delay 120ms 40ms loss 2% rate 10mbit
    ;;
  stop)
    sudo tc qdisc del dev "$iface" root || true
    ;;
  status)
    tc qdisc show dev "$iface"
    ;;
  *)
    echo "usage: $0 [iface] [start|stop|status]" >&2
    exit 2
    ;;
esac
