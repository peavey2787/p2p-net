package io.github.peavey2787.p2pnet

import android.content.Context
import android.os.SystemClock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Process-wide owner for the Rust node.
 *
 * The repository deliberately mirrors the desktop dashboard's low-overhead
 * strategy: poll only the atomic revision once per second, fetch/parse the full
 * snapshot only when that revision changes, and drain the bounded message queue
 * only when non-empty. Peer/metric/bridge detail has a slower cadence.
 *
 * Native calls are serialized so service shutdown cannot tear down Tokio while
 * another coroutine is executing a JNI operation. Refreshes are conflated by a
 * non-blocking mutex so connectivity churn cannot build an unbounded poll queue.
 */
object NodeRepository {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val lifecycleMutex = Mutex()
    private val nativeOperationMutex = Mutex()
    private val refreshMutex = Mutex()
    private val refreshRequested = AtomicBoolean(false)
    private val _state = MutableStateFlow(NodeUiState())
    val state: StateFlow<NodeUiState> = _state.asStateFlow()

    private var samplerJob: Job? = null
    private var lastPeerRefreshElapsed = 0L
    private var lastBridgeRefreshElapsed = 0L

    suspend fun ensureStarted(context: Context): Boolean {
        return lifecycleMutex.withLock {
            if (_state.value.running) return@withLock true
            updateState { it.copy(busy = true, error = null) }
            try {
                val appContext = context.applicationContext
                val config = readConfig(appContext)
                updateState { it.copy(configJson = config) }
                require(config.toByteArray(Charsets.UTF_8).size <= MAX_CONFIG_BYTES) {
                    "Persisted NodeConfig exceeds 256 KiB"
                }
                val dataDir = ensureDataDir(appContext)
                nativeOperationMutex.withLock {
                    withContext(Dispatchers.IO) {
                        NativeJson.requireSuccess(NativeNode.validateConfig(config))
                        NativeJson.requireSuccess(NativeNode.start(config, dataDir.absolutePath))
                    }
                }
                updateState {
                    it.copy(
                        running = true,
                        busy = false,
                        configJson = config,
                    )
                }
                resetSamplingDeadlines()
                startSampler()
                refresh(forceDetails = true)
                true
            } catch (error: Throwable) {
                updateState {
                    it.copy(
                        running = false,
                        busy = false,
                        error = error.safeMessage(),
                    )
                }
                false
            }
        }
    }

    suspend fun stop() {
        lifecycleMutex.withLock {
            if (!_state.value.running) {
                stopSamplerAndJoin()
                return
            }
            updateState { it.copy(busy = true) }
            stopSamplerAndJoin()
            try {
                nativeOperationMutex.withLock {
                    withContext(Dispatchers.IO) {
                        NativeJson.requireSuccess(NativeNode.stop())
                    }
                }
                val current = _state.value
                _state.value = NodeUiState(
                    configJson = current.configJson,
                    localNetworkGranted = current.localNetworkGranted,
                )
            } catch (error: Throwable) {
                updateState {
                    it.copy(
                        busy = false,
                        error = error.safeMessage(),
                    )
                }
                if (_state.value.running) startSampler()
            }
        }
    }

    fun connect(address: String) = nativeAction(forceDetails = true) {
        NativeJson.requireSuccess(NativeNode.connect(address.trim()))
    }

    fun disconnect(peerId: String) = nativeAction(forceDetails = true) {
        NativeJson.requireSuccess(NativeNode.disconnect(peerId.trim()))
    }

    fun subscribe(topic: String) = nativeAction(forceDetails = true) {
        NativeJson.requireSuccess(NativeNode.subscribe(topic.trim()))
    }

    fun broadcast(topic: String, payload: ByteArray) = nativeAction {
        NativeJson.requireSuccess(NativeNode.broadcast(topic.trim(), payload))
    }

    fun send(peerId: String, topic: String, payload: ByteArray) = nativeAction {
        NativeJson.requireSuccess(NativeNode.send(peerId.trim(), topic.trim(), payload))
    }

    fun requestRefresh() {
        if (!refreshRequested.compareAndSet(false, true)) return
        scope.launch {
            try {
                refresh(forceDetails = true)
            } finally {
                refreshRequested.set(false)
            }
        }
    }

    fun requestStop() {
        scope.launch { stop() }
    }

    fun updateLocalNetworkPermission(granted: Boolean?) {
        updateState { it.copy(localNetworkGranted = granted) }
    }

    fun saveConfigAndRestart(context: Context, configJson: String) {
        scope.launch {
            lifecycleMutex.withLock {
                val appContext = context.applicationContext
                val previousConfig = _state.value.configJson.ifBlank { readConfig(appContext) }
                val wasRunning = _state.value.running
                var stoppedForRestart = false
                var replacementStarted = false
                try {
                    require(configJson.isNotBlank()) { "NodeConfig must not be blank" }
                    require(configJson.toByteArray(Charsets.UTF_8).size <= MAX_CONFIG_BYTES) {
                        "NodeConfig exceeds 256 KiB"
                    }
                    nativeOperationMutex.withLock {
                        withContext(Dispatchers.IO) {
                            NativeJson.requireSuccess(NativeNode.validateConfig(configJson))
                        }
                    }

                    if (!wasRunning) {
                        persistConfig(appContext, configJson)
                        updateState { it.copy(configJson = configJson, error = null) }
                        return@withLock
                    }

                    updateState { it.copy(busy = true, error = null) }
                    stopSamplerAndJoin()
                    nativeOperationMutex.withLock {
                        withContext(Dispatchers.IO) {
                            NativeJson.requireSuccess(NativeNode.stop())
                        }
                    }
                    stoppedForRestart = true

                    val dataDir = ensureDataDir(appContext)
                    nativeOperationMutex.withLock {
                        withContext(Dispatchers.IO) {
                            NativeJson.requireSuccess(
                                NativeNode.start(configJson, dataDir.absolutePath),
                            )
                        }
                    }
                    replacementStarted = true
                    persistConfig(appContext, configJson)
                    _state.value = NodeUiState(
                        running = true,
                        configJson = configJson,
                        localNetworkGranted = _state.value.localNetworkGranted,
                    )
                    resetSamplingDeadlines()
                    startSampler()
                    refresh(forceDetails = true)
                } catch (error: Throwable) {
                    var rollbackNodeStarted = false
                    val rollbackError = if (wasRunning && stoppedForRestart) {
                        runCatching {
                            nativeOperationMutex.withLock {
                                withContext(Dispatchers.IO) {
                                    if (replacementStarted) {
                                        NativeJson.requireSuccess(NativeNode.stop())
                                    }
                                    val dataDir = ensureDataDir(appContext)
                                    NativeJson.requireSuccess(
                                        NativeNode.start(previousConfig, dataDir.absolutePath),
                                    )
                                }
                            }
                            rollbackNodeStarted = true
                            persistConfig(appContext, previousConfig)
                            _state.value = NodeUiState(
                                running = true,
                                configJson = previousConfig,
                                localNetworkGranted = _state.value.localNetworkGranted,
                            )
                            resetSamplingDeadlines()
                            startSampler()
                            refresh(forceDetails = true)
                        }.exceptionOrNull()
                    } else {
                        null
                    }

                    updateState {
                        it.copy(
                            running = if (stoppedForRestart) rollbackNodeStarted else wasRunning,
                            busy = false,
                            configJson = previousConfig,
                            error = buildString {
                                append(error.safeMessage())
                                if (rollbackError != null) {
                                    append("; rollback failed: ")
                                    append(rollbackError.safeMessage())
                                }
                            },
                        )
                    }
                    if (wasRunning && !stoppedForRestart) startSampler()
                    if (rollbackNodeStarted && samplerJob?.isActive != true) startSampler()
                }
            }
        }
    }

    fun clearError() {
        updateState { it.copy(error = null) }
    }

    fun trimTransientUiState() {
        updateState { state ->
            state.copy(
                peers = state.peers.filter { it.connected }.take(MAX_TRIMMED_PEERS),
                messages = appendBoundedMessages(
                    current = emptyList(),
                    incoming = state.messages.takeLast(MAX_TRIMMED_MESSAGES),
                    maxMessages = MAX_TRIMMED_MESSAGES,
                    maxPayloadBytes = MAX_TRIMMED_MESSAGE_PAYLOAD_BYTES,
                ),
            )
        }
        if (_state.value.running) requestRefresh()
    }

    private fun nativeAction(forceDetails: Boolean = false, action: () -> Unit) {
        scope.launch {
            try {
                nativeOperationMutex.withLock {
                    withContext(Dispatchers.IO) {
                        requireRunningAndIdle()
                        action()
                    }
                }
                if (forceDetails) refresh(forceDetails = true)
            } catch (error: Throwable) {
                updateState { it.copy(error = error.safeMessage()) }
            }
        }
    }

    private fun startSampler() {
        if (samplerJob?.isActive == true) return
        samplerJob = scope.launch {
            while (isActive && _state.value.running) {
                refresh(forceDetails = false)
                delay(POLL_INTERVAL_MS)
            }
        }
    }

    private suspend fun stopSamplerAndJoin() {
        val job = samplerJob
        samplerJob = null
        job?.cancelAndJoin()
    }

    private suspend fun refresh(forceDetails: Boolean) {
        if (!_state.value.running || _state.value.busy) return
        if (!refreshMutex.tryLock()) return
        try {
            nativeOperationMutex.withLock {
                if (!_state.value.running || _state.value.busy) return@withLock

                val revision = withContext(Dispatchers.IO) { NativeNode.revision() }
                if (revision != 0L && revision != _state.value.revision) {
                    val (snapshotRevision, snapshot) = withContext(Dispatchers.IO) {
                        NativeJson.snapshot(NativeNode.snapshot())
                    }
                    updateState {
                        it.copy(
                            revision = snapshotRevision,
                            snapshot = snapshot,
                            error = null,
                        )
                    }
                }

                val now = SystemClock.elapsedRealtime()
                if (forceDetails || now - lastPeerRefreshElapsed >= PEER_REFRESH_INTERVAL_MS) {
                    val (peers, metrics) = withContext(Dispatchers.IO) {
                        NativeJson.peers(NativeNode.peers()) to
                            NativeJson.metrics(NativeNode.metrics())
                    }
                    updateState { it.copy(peers = peers, metrics = metrics) }
                    lastPeerRefreshElapsed = now
                }

                if (forceDetails || now - lastBridgeRefreshElapsed >= BRIDGE_REFRESH_INTERVAL_MS) {
                    val bridgeStats = withContext(Dispatchers.IO) {
                        NativeJson.bridgeStats(NativeNode.bridgeStats())
                    }
                    updateState { it.copy(bridgeStats = bridgeStats) }
                    lastBridgeRefreshElapsed = now
                }

                if (withContext(Dispatchers.IO) { NativeNode.pendingMessageCount() } > 0) {
                    val incoming = withContext(Dispatchers.IO) {
                        NativeJson.messages(NativeNode.drainMessages(MAX_DRAIN_MESSAGES))
                    }
                    if (incoming.isNotEmpty()) {
                        updateState {
                            it.copy(
                                messages = appendBoundedMessages(
                                    current = it.messages,
                                    incoming = incoming,
                                ),
                            )
                        }
                    }
                }
            }
        } catch (error: Throwable) {
            updateState { it.copy(error = error.safeMessage()) }
        } finally {
            refreshMutex.unlock()
        }
    }

    private suspend fun readConfig(context: Context): String {
        val preferences = context.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
        preferences.getString(CONFIG_KEY, null)?.let { return it }
        val config = nativeOperationMutex.withLock {
            withContext(Dispatchers.IO) {
                NativeJson.defaultConfig(NativeNode.defaultConfig())
            }
        }
        persistConfig(context, config)
        return config
    }

    private suspend fun persistConfig(context: Context, config: String) {
        val persisted = withContext(Dispatchers.IO) {
            context
                .getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
                .edit()
                .putString(CONFIG_KEY, config)
                .commit()
        }
        check(persisted) { "failed to persist NodeConfig" }
    }

    private fun ensureDataDir(context: Context): File {
        val dataDir = File(context.filesDir, DATA_DIR_NAME)
        check((dataDir.isDirectory || dataDir.mkdirs()) && dataDir.isDirectory) {
            "failed to create Android app-private node directory"
        }
        return dataDir
    }

    private fun requireRunningAndIdle() {
        check(_state.value.running) { "P2P node is not running" }
        check(!_state.value.busy) { "P2P node lifecycle transition is in progress" }
    }

    private fun resetSamplingDeadlines() {
        lastPeerRefreshElapsed = 0L
        lastBridgeRefreshElapsed = 0L
    }

    private fun updateState(transform: (NodeUiState) -> NodeUiState) {
        synchronized(_state) {
            _state.value = transform(_state.value)
        }
    }

    private fun Throwable.safeMessage(): String {
        return sanitizeDisplayText(message ?: javaClass.simpleName, MAX_ERROR_CHARS)
    }

    private const val POLL_INTERVAL_MS = 1_000L
    private const val PEER_REFRESH_INTERVAL_MS = 5_000L
    private const val BRIDGE_REFRESH_INTERVAL_MS = 10_000L
    private const val MAX_DRAIN_MESSAGES = 64
    private const val MAX_TRIMMED_MESSAGES = 20
    private const val MAX_TRIMMED_MESSAGE_PAYLOAD_BYTES = 1024 * 1024
    private const val MAX_TRIMMED_PEERS = 64
    private const val MAX_CONFIG_BYTES = 256 * 1024
    private const val MAX_ERROR_CHARS = 2_048
    private const val DATA_DIR_NAME = "p2p-net"
    private const val PREFERENCES_NAME = "p2p-net"
    private const val CONFIG_KEY = "node_config_json"
}
