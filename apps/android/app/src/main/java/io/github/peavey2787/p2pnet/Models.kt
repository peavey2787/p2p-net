package io.github.peavey2787.p2pnet

import org.json.JSONArray
import org.json.JSONObject
import java.util.Base64

data class NodeSnapshotView(
    val peerId: String = "",
    val networkLabel: String = "",
    val uptimeSeconds: Long = 0,
    val natStatus: String = "unknown",
    val reachability: String = "unknown",
    val publicAddress: String? = null,
    val activeTransports: List<String> = emptyList(),
    val connectedPeers: Int = 0,
    val infrastructurePeers: Int = 0,
    val knownPeers: Int = 0,
    val discoveredPeers: Int = 0,
    val pendingConnections: Int = 0,
    val relayReservations: Int = 0,
    val relayReservationAttempts: Long = 0,
    val relayReservationFailures: Long = 0,
    val relayActiveCircuits: Int = 0,
    val relayDeniedRequests: Long = 0,
    val dcutrAttempts: Long = 0,
    val dcutrSuccesses: Long = 0,
    val dcutrFailures: Long = 0,
    val dhtQueries: Long = 0,
    val dhtPeersDiscovered: Long = 0,
    val rendezvousDiscoveredPeers: Long = 0,
    val sentMessages: Long = 0,
    val receivedMessages: Long = 0,
    val rejectedMessages: Long = 0,
    val subscriptions: List<String> = emptyList(),
    val pulses: List<String> = emptyList(),
)

data class NodeMetricsView(
    val bytesSent: Long = 0,
    val bytesReceived: Long = 0,
    val bytesStored: Long = 0,
    val activeRequests: Int = 0,
    val chokedPeers: Int = 0,
)

data class BridgeStatsView(
    val runtimeWorkerThreads: Int = 0,
    val runtimeMaxBlockingThreads: Int = 0,
    val runtimeShutdownTimeoutMs: Long = 0,
    val messageQueueCapacity: Int = 0,
    val messageQueueMaxPayloadBytes: Int = 0,
    val pendingMessages: Int = 0,
    val queuedPayloadBytes: Int = 0,
    val droppedMessages: Long = 0,
    val subscriptions: Int = 0,
    val maxSubscriptions: Int = 0,
    val maxResponseJsonBytes: Int = 0,
    val maxBridgePeers: Int = 0,
)

data class PeerView(
    val peerId: String,
    val connected: Boolean,
    val addresses: List<String>,
    val sources: List<String>,
    val namespace: String?,
    val supportsRelay: Boolean?,
    val supportsRendezvous: Boolean?,
    val supportsDcutr: Boolean?,
)

data class ReceivedMessage(
    val topic: String,
    val sourcePeerId: String,
    val targetPeerId: String?,
    val timestampNs: Long,
    val payload: ByteArray,
) {
    val displayPayload: String
        get() = sanitizeDisplayText(String(payload, Charsets.UTF_8), MAX_MESSAGE_PREVIEW_CHARS)
}

data class NodeUiState(
    val running: Boolean = false,
    val busy: Boolean = false,
    val revision: Long = 0,
    val snapshot: NodeSnapshotView = NodeSnapshotView(),
    val peers: List<PeerView> = emptyList(),
    val metrics: NodeMetricsView = NodeMetricsView(),
    val bridgeStats: BridgeStatsView = BridgeStatsView(),
    val messages: List<ReceivedMessage> = emptyList(),
    val configJson: String = "",
    val localNetworkGranted: Boolean? = null,
    val error: String? = null,
)

internal object NativeJson {
    fun defaultConfig(response: String): String {
        return valueObject(response).getString("config")
    }

    fun requireSuccess(response: String) {
        valueObject(response)
    }

    fun snapshot(response: String): Pair<Long, NodeSnapshotView> {
        val value = valueObject(response)
        val revision = value.optLong("revision", 0)
        val snapshot = value.getJSONObject("snapshot")
        return revision to NodeSnapshotView(
            peerId = snapshot.optSafeString("peer_id"),
            networkLabel = snapshot.optSafeString("network_label"),
            uptimeSeconds = snapshot.optLong("uptime_secs"),
            natStatus = snapshot.optSafeString("nat_status", "unknown"),
            reachability = snapshot.optSafeString("environment_reachability", "unknown"),
            publicAddress = snapshot.optNullableSafeString("public_addr"),
            activeTransports = snapshot.optJSONArray("active_transports").safeStringList(),
            connectedPeers = snapshot.optInt("application_peer_connections"),
            infrastructurePeers = snapshot.optInt("infrastructure_peer_connections"),
            knownPeers = snapshot.optInt("peer_book_known_peers"),
            discoveredPeers = snapshot.optInt("peer_book_discovered_peers"),
            pendingConnections = snapshot.optInt("connection_plan_pending_peers"),
            relayReservations = snapshot.optInt("relay_client_reservations"),
            relayReservationAttempts = snapshot.optLong("relay_client_reservation_attempts"),
            relayReservationFailures = snapshot.optLong("relay_client_reservation_failures"),
            relayActiveCircuits = snapshot.optInt("relay_active_circuits"),
            relayDeniedRequests = snapshot.optLong("relay_denied_requests"),
            dcutrAttempts = snapshot.optLong("dcutr_attempts"),
            dcutrSuccesses = snapshot.optLong("dcutr_successes"),
            dcutrFailures = snapshot.optLong("dcutr_failures"),
            dhtQueries = snapshot.optLong("dht_provider_queries"),
            dhtPeersDiscovered = snapshot.optLong("dht_provider_peers_discovered"),
            rendezvousDiscoveredPeers = snapshot.optLong("rendezvous_discovered_peers"),
            sentMessages = snapshot.optLong("app_messages_sent"),
            receivedMessages = snapshot.optLong("app_messages_received"),
            rejectedMessages = snapshot.optLong("app_messages_rejected"),
            subscriptions = snapshot.optJSONArray("app_subscriptions").safeStringList(),
            pulses = snapshot
                .optJSONArray("pulses")
                .safeStringList(MAX_PULSES)
                .takeLast(MAX_PULSES),
        )
    }

    fun metrics(response: String): NodeMetricsView {
        val value = valueObject(response)
        return NodeMetricsView(
            bytesSent = value.optLong("total_bytes_sent"),
            bytesReceived = value.optLong("total_bytes_received"),
            bytesStored = value.optLong("total_bytes_stored"),
            activeRequests = value.optInt("active_request_count"),
            chokedPeers = value.optInt("choked_peers_count"),
        )
    }

    fun bridgeStats(response: String): BridgeStatsView {
        val value = valueObject(response)
        return BridgeStatsView(
            runtimeWorkerThreads = value.optInt("runtime_worker_threads"),
            runtimeMaxBlockingThreads = value.optInt("runtime_max_blocking_threads"),
            runtimeShutdownTimeoutMs = value.optLong("runtime_shutdown_timeout_ms"),
            messageQueueCapacity = value.optInt("message_queue_capacity"),
            messageQueueMaxPayloadBytes = value.optInt("message_queue_max_payload_bytes"),
            pendingMessages = value.optInt("pending_messages"),
            queuedPayloadBytes = value.optInt("queued_payload_bytes"),
            droppedMessages = value.optLong("dropped_messages"),
            subscriptions = value.optInt("subscriptions"),
            maxSubscriptions = value.optInt("max_subscriptions"),
            maxResponseJsonBytes = value.optInt("max_response_json_bytes"),
            maxBridgePeers = value.optInt("max_bridge_peers"),
        )
    }

    fun peers(response: String): List<PeerView> {
        val peers = valueObject(response).optJSONArray("peers") ?: return emptyList()
        return buildList(peers.length().coerceAtMost(MAX_PEERS)) {
            for (index in 0 until peers.length().coerceAtMost(MAX_PEERS)) {
                val peer = peers.optJSONObject(index) ?: continue
                add(
                    PeerView(
                        peerId = peer.optSafeString("peer_id"),
                        connected = peer.optBoolean("connected"),
                        addresses = peer.optJSONArray("addresses").safeStringList(MAX_PEER_ADDRESSES),
                        sources = peer.optJSONArray("sources").safeStringList(MAX_PEER_SOURCES),
                        namespace = peer.optNullableSafeString("namespace"),
                        supportsRelay = peer.optNullableBoolean("supports_relay"),
                        supportsRendezvous = peer.optNullableBoolean("supports_rendezvous"),
                        supportsDcutr = peer.optNullableBoolean("supports_dcutr"),
                    ),
                )
            }
        }
    }

    fun messages(response: String): List<ReceivedMessage> {
        val messages = valueObject(response).optJSONArray("messages") ?: return emptyList()
        return buildList(messages.length().coerceAtMost(MAX_DRAIN_MESSAGES)) {
            for (index in 0 until messages.length().coerceAtMost(MAX_DRAIN_MESSAGES)) {
                val message = messages.optJSONObject(index) ?: continue
                add(
                    ReceivedMessage(
                        topic = message.optSafeString("topic"),
                        sourcePeerId = message.optSafeString("source_peer_id"),
                        targetPeerId = message.optNullableSafeString("target_peer_id"),
                        timestampNs = message.optLong("timestamp_ns"),
                        payload = decodeNativePayload(
                            encoded = message.optString("payload_base64", ""),
                            declaredBytes = message.optInt("payload_len", -1),
                        ),
                    ),
                )
            }
        }
    }

    private fun valueObject(response: String): JSONObject {
        check(response.toByteArray(Charsets.UTF_8).size <= MAX_NATIVE_RESPONSE_BYTES) {
            "native response exceeds 4 MiB"
        }
        val root = JSONObject(response)
        if (!root.optBoolean("ok")) {
            throw IllegalStateException(
                sanitizeDisplayText(root.optString("error", "native operation failed"), MAX_ERROR_CHARS),
            )
        }
        return root.optJSONObject("value") ?: JSONObject()
    }

    private fun JSONObject.optSafeString(name: String, fallback: String = ""): String {
        return sanitizeDisplayText(optString(name, fallback), MAX_FIELD_CHARS)
    }

    private fun JSONObject.optNullableSafeString(name: String): String? {
        if (isNull(name)) return null
        return optSafeString(name).takeIf { it.isNotBlank() }
    }

    private fun JSONObject.optNullableBoolean(name: String): Boolean? {
        if (!has(name) || isNull(name)) return null
        return optBoolean(name)
    }

    private fun JSONArray?.safeStringList(maxItems: Int = MAX_STRING_LIST_ITEMS): List<String> {
        if (this == null) return emptyList()
        val bounded = length().coerceAtMost(maxItems)
        return buildList(bounded) {
            for (index in 0 until bounded) {
                sanitizeDisplayText(optString(index), MAX_FIELD_CHARS)
                    .takeIf { it.isNotBlank() }
                    ?.let(::add)
            }
        }
    }

    private const val MAX_PEERS = 512
    private const val MAX_PEER_ADDRESSES = 32
    private const val MAX_PEER_SOURCES = 16
    private const val MAX_PULSES = 64
    private const val MAX_DRAIN_MESSAGES = 64
    private const val MAX_STRING_LIST_ITEMS = 128
    private const val MAX_FIELD_CHARS = 4_096
    private const val MAX_ERROR_CHARS = 2_048
}

internal fun decodeNativePayload(
    encoded: String,
    declaredBytes: Int,
    maxBytes: Int = MAX_NATIVE_MESSAGE_PAYLOAD_BYTES,
): ByteArray {
    require(declaredBytes in 0..maxBytes) { "native message payload length is invalid" }
    val maxBase64Chars = ((maxBytes + 2) / 3) * 4
    require(encoded.length <= maxBase64Chars) { "native message payload encoding is oversized" }
    val decoded = try {
        Base64.getDecoder().decode(encoded)
    } catch (_: IllegalArgumentException) {
        throw IllegalStateException("native message payload is not valid base64")
    }
    check(decoded.size == declaredBytes) { "native message payload length mismatch" }
    return decoded
}

internal fun appendBoundedMessages(
    current: List<ReceivedMessage>,
    incoming: List<ReceivedMessage>,
    maxMessages: Int = MAX_UI_MESSAGE_HISTORY,
    maxPayloadBytes: Int = MAX_UI_MESSAGE_PAYLOAD_BYTES,
): List<ReceivedMessage> {
    require(maxMessages >= 0)
    require(maxPayloadBytes >= 0)
    if (maxMessages == 0 || maxPayloadBytes == 0) return emptyList()

    val combined = current + incoming
    var retainedPayloadBytes = 0L
    val retainedNewestFirst = ArrayList<ReceivedMessage>(maxMessages.coerceAtMost(combined.size))
    for (index in combined.indices.reversed()) {
        if (retainedNewestFirst.size >= maxMessages) break
        val message = combined[index]
        val messageBytes = message.payload.size.toLong()
        if (messageBytes > maxPayloadBytes.toLong()) continue
        if (retainedPayloadBytes + messageBytes > maxPayloadBytes.toLong()) break
        retainedPayloadBytes += messageBytes
        retainedNewestFirst += message
    }
    retainedNewestFirst.reverse()
    return retainedNewestFirst
}

internal fun sanitizeDisplayText(value: String, maxChars: Int): String {
    if (maxChars <= 0) return ""
    val safe = buildString(value.length.coerceAtMost(maxChars)) {
        var emitted = 0
        for (char in value) {
            if (emitted >= maxChars) break
            when {
                char == '\n' || char == '\t' -> append(char)
                char.isISOControl() || char in BIDI_CONTROL_CHARS -> append('�')
                else -> append(char)
            }
            emitted += 1
        }
    }
    return if (value.length > maxChars) "$safe…" else safe
}

private val BIDI_CONTROL_CHARS = setOf(
    '\u061C',
    '\u200E',
    '\u200F',
    '\u202A',
    '\u202B',
    '\u202C',
    '\u202D',
    '\u202E',
    '\u2066',
    '\u2067',
    '\u2068',
    '\u2069',
)

internal const val MAX_NATIVE_MESSAGE_PAYLOAD_BYTES = 1024 * 1024
internal const val MAX_NATIVE_RESPONSE_BYTES = 4 * 1024 * 1024
internal const val MAX_UI_MESSAGE_HISTORY = 100
internal const val MAX_UI_MESSAGE_PAYLOAD_BYTES = 4 * 1024 * 1024
private const val MAX_MESSAGE_PREVIEW_CHARS = 512
