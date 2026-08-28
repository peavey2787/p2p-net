package io.github.peavey2787.p2pnet.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.peavey2787.p2pnet.MainViewModel
import io.github.peavey2787.p2pnet.NodeUiState
import io.github.peavey2787.p2pnet.PeerView
import io.github.peavey2787.p2pnet.ReceivedMessage

private const val MAX_MULTIADDR_INPUT_CHARS = 4_096
private const val MAX_PEER_ID_INPUT_CHARS = 256
private const val MAX_TOPIC_INPUT_CHARS = 128
private const val MAX_MESSAGE_INPUT_CHARS = 4_096
private const val MAX_CONFIG_INPUT_BYTES = 256 * 1024

@Composable
fun P2PNodeApp(
    viewModel: MainViewModel,
    onStartService: () -> Unit,
    onStopService: () -> Unit,
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var tab by remember { mutableIntStateOf(0) }

    MaterialTheme {
        Scaffold(
            bottomBar = {
                NavigationBar {
                    listOf("Node", "Peers", "Messages", "Settings").forEachIndexed { index, label ->
                        NavigationBarItem(
                            selected = tab == index,
                            onClick = { tab = index },
                            icon = {},
                            label = { Text(label) },
                        )
                    }
                }
            },
        ) { padding ->
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Header(state, onStartService, onStopService, viewModel::clearError)
                Box(modifier = Modifier.weight(1f)) {
                    when (tab) {
                        0 -> Dashboard(state)
                        1 -> Peers(
                            state = state,
                            connect = viewModel::connect,
                            disconnect = viewModel::disconnect,
                            refresh = viewModel::refresh,
                        )
                        2 -> Messages(
                            state = state,
                            subscribe = viewModel::subscribe,
                            broadcast = viewModel::broadcast,
                            send = viewModel::send,
                        )
                        else -> Settings(
                            configJson = state.configJson,
                            running = state.running,
                            busy = state.busy,
                        ) { config ->
                            viewModel.saveConfigAndRestart(context, config)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Header(
    state: NodeUiState,
    onStartService: () -> Unit,
    onStopService: () -> Unit,
    clearError: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = if (state.running) "ONLINE" else "OFFLINE",
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.weight(1f),
            )
            Text(if (state.busy) "Working…" else "Full node")
            if (state.running) {
                Button(onClick = onStopService, enabled = !state.busy) { Text("Stop") }
            } else {
                Button(onClick = onStartService, enabled = !state.busy) { Text("Start") }
            }
        }

        if (state.localNetworkGranted == false) {
            MetricCard(
                "Local-network permission",
                "Denied. Internet and relay paths can still operate, but direct LAN discovery/dials may be limited on Android 17+.",
            )
        }

        state.error?.let { error ->
            Card(modifier = Modifier.fillMaxWidth()) {
                Row(
                    modifier = Modifier.padding(12.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(error, modifier = Modifier.weight(1f))
                    Button(onClick = clearError) { Text("Dismiss") }
                }
            }
        }
    }
}

@Composable
private fun Dashboard(state: NodeUiState) {
    val snapshot = state.snapshot
    val bridge = state.bridgeStats
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item { MetricCard("Identity", snapshot.peerId.ifBlank { "Starting…" }) }
        item { MetricCard("Network", snapshot.networkLabel.ifBlank { "unknown" }) }
        item { MetricCard("Uptime", formatUptime(snapshot.uptimeSeconds)) }
        item { MetricCard("Reachability", "${snapshot.reachability} · NAT ${snapshot.natStatus}") }
        item { MetricCard("Public address", snapshot.publicAddress ?: "not discovered") }
        item { MetricCard("Transports", snapshot.activeTransports.joinToString().ifBlank { "none" }) }
        item {
            MetricCard(
                "Peers",
                "application ${snapshot.connectedPeers} · infrastructure ${snapshot.infrastructurePeers} · known ${snapshot.knownPeers} · discovered ${snapshot.discoveredPeers} · pending ${snapshot.pendingConnections}",
            )
        }
        item {
            MetricCard(
                "Relay",
                "reservations ${snapshot.relayReservations} · attempts ${snapshot.relayReservationAttempts} · failures ${snapshot.relayReservationFailures} · circuits ${snapshot.relayActiveCircuits} · denied ${snapshot.relayDeniedRequests}",
            )
        }
        item {
            MetricCard(
                "DCUtR",
                "attempts ${snapshot.dcutrAttempts} · successes ${snapshot.dcutrSuccesses} · failures ${snapshot.dcutrFailures}",
            )
        }
        item {
            MetricCard(
                "Discovery",
                "DHT queries ${snapshot.dhtQueries} · DHT peers ${snapshot.dhtPeersDiscovered} · rendezvous peers ${snapshot.rendezvousDiscoveredPeers}",
            )
        }
        item {
            MetricCard(
                "Application traffic",
                "messages tx ${snapshot.sentMessages} · rx ${snapshot.receivedMessages} · rejected ${snapshot.rejectedMessages} · bytes tx ${state.metrics.bytesSent} · rx ${state.metrics.bytesReceived}",
            )
        }
        item {
            MetricCard(
                "Runtime pressure",
                "active requests ${state.metrics.activeRequests} · choked peers ${state.metrics.chokedPeers} · stored ${state.metrics.bytesStored} bytes",
            )
        }
        item {
            MetricCard(
                "Android bridge",
                "workers ${bridge.runtimeWorkerThreads} · blocking max ${bridge.runtimeMaxBlockingThreads} · queue ${bridge.pendingMessages}/${bridge.messageQueueCapacity} (${bridge.queuedPayloadBytes}/${bridge.messageQueueMaxPayloadBytes} bytes) · dropped ${bridge.droppedMessages} · subscriptions ${bridge.subscriptions}/${bridge.maxSubscriptions} · peer detail cap ${bridge.maxBridgePeers}",
            )
        }
        if (snapshot.subscriptions.isNotEmpty()) {
            item { MetricCard("Subscriptions", snapshot.subscriptions.joinToString()) }
        }
        items(snapshot.pulses.asReversed()) { pulse -> MetricCard("Event", pulse) }
    }
}

@Composable
private fun Peers(
    state: NodeUiState,
    connect: (String) -> Unit,
    disconnect: (String) -> Unit,
    refresh: () -> Unit,
) {
    var address by remember { mutableStateOf("") }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(
            value = address,
            onValueChange = { address = it.take(MAX_MULTIADDR_INPUT_CHARS) },
            label = { Text("Peer multiaddr") },
            modifier = Modifier.fillMaxWidth(),
            enabled = state.running && !state.busy,
            singleLine = true,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = { connect(address) },
                enabled = state.running && !state.busy && address.isNotBlank(),
            ) { Text("Connect") }
            Button(
                onClick = refresh,
                enabled = state.running && !state.busy,
            ) { Text("Refresh") }
        }
        LazyColumn(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            items(state.peers, key = { it.peerId }) { peer ->
                PeerCard(peer = peer, canDisconnect = state.running && !state.busy, disconnect = disconnect)
            }
        }
    }
}

@Composable
private fun PeerCard(
    peer: PeerView,
    canDisconnect: Boolean,
    disconnect: (String) -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(10.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(peer.peerId, style = MaterialTheme.typography.bodySmall)
            Text(if (peer.connected) "Connected" else "Known")
            if (peer.sources.isNotEmpty()) Text("Sources: ${peer.sources.joinToString()}")
            peer.namespace?.let { Text("Namespace: $it") }
            peer.addresses.take(3).forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
            val capabilities = buildList {
                if (peer.supportsRelay == true) add("relay")
                if (peer.supportsRendezvous == true) add("rendezvous")
                if (peer.supportsDcutr == true) add("DCUtR")
            }
            if (capabilities.isNotEmpty()) Text("Capabilities: ${capabilities.joinToString()}")
            if (peer.connected) {
                Button(
                    onClick = { disconnect(peer.peerId) },
                    enabled = canDisconnect,
                ) { Text("Disconnect") }
            }
        }
    }
}

@Composable
private fun Messages(
    state: NodeUiState,
    subscribe: (String) -> Unit,
    broadcast: (String, String) -> Unit,
    send: (String, String, String) -> Unit,
) {
    var topic by remember { mutableStateOf("chat/general") }
    var peerId by remember { mutableStateOf("") }
    var payload by remember { mutableStateOf("") }
    val actionsEnabled = state.running && !state.busy

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(
            value = topic,
            onValueChange = { topic = it.take(MAX_TOPIC_INPUT_CHARS) },
            label = { Text("Topic") },
            modifier = Modifier.fillMaxWidth(),
            enabled = actionsEnabled,
            singleLine = true,
        )
        OutlinedTextField(
            value = peerId,
            onValueChange = { peerId = it.take(MAX_PEER_ID_INPUT_CHARS) },
            label = { Text("Peer ID (optional)") },
            modifier = Modifier.fillMaxWidth(),
            enabled = actionsEnabled,
            singleLine = true,
        )
        OutlinedTextField(
            value = payload,
            onValueChange = { payload = it.take(MAX_MESSAGE_INPUT_CHARS) },
            label = { Text("Message (${payload.length}/$MAX_MESSAGE_INPUT_CHARS chars)") },
            modifier = Modifier.fillMaxWidth(),
            enabled = actionsEnabled,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = { subscribe(topic) },
                enabled = actionsEnabled && topic.isNotBlank(),
            ) { Text("Subscribe") }
            Button(
                onClick = { broadcast(topic, payload) },
                enabled = actionsEnabled && topic.isNotBlank() && payload.isNotEmpty(),
            ) { Text("Broadcast") }
            Button(
                onClick = { send(peerId, topic, payload) },
                enabled = actionsEnabled && peerId.isNotBlank() && topic.isNotBlank() && payload.isNotEmpty(),
            ) { Text("Send") }
        }
        LazyColumn(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            items(state.messages.asReversed()) { message ->
                MetricCard(
                    message.topic,
                    "${message.sourcePeerId.take(32)} · ${message.displayPayload}",
                )
            }
        }
    }
}

@Composable
private fun Settings(
    configJson: String,
    running: Boolean,
    busy: Boolean,
    save: (String) -> Unit,
) {
    var config by remember(configJson) { mutableStateOf(configJson) }
    val configBytes = config.toByteArray(Charsets.UTF_8).size
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("NodeConfig JSON", style = MaterialTheme.typography.titleMedium)
        Text("The same validated Rust NodeConfig is used by desktop and Android.")
        Text("$configBytes / $MAX_CONFIG_INPUT_BYTES bytes")
        OutlinedTextField(
            value = config,
            onValueChange = { candidate ->
                if (candidate.toByteArray(Charsets.UTF_8).size <= MAX_CONFIG_INPUT_BYTES) {
                    config = candidate
                }
            },
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            minLines = 12,
            enabled = !busy,
        )
        Button(
            onClick = { save(config) },
            enabled = !busy && config.isNotBlank() && configBytes <= MAX_CONFIG_INPUT_BYTES,
        ) {
            Text(if (running) "Save & Restart" else "Save")
        }
    }
}

@Composable
private fun MetricCard(label: String, value: String) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(10.dp)) {
            Text(label, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold)
            Text(value)
        }
    }
}

private fun formatUptime(seconds: Long): String {
    val hours = seconds / 3600
    val minutes = (seconds % 3600) / 60
    val remaining = seconds % 60
    return "%02d:%02d:%02d".format(hours, minutes, remaining)
}
