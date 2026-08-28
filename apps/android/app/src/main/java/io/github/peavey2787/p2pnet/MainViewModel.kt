package io.github.peavey2787.p2pnet

import android.content.Context
import androidx.lifecycle.ViewModel

class MainViewModel : ViewModel() {
    val state = NodeRepository.state

    fun connect(address: String) = NodeRepository.connect(address)
    fun disconnect(peerId: String) = NodeRepository.disconnect(peerId)
    fun subscribe(topic: String) = NodeRepository.subscribe(topic)
    fun broadcast(topic: String, payload: String) =
        NodeRepository.broadcast(topic, payload.toByteArray(Charsets.UTF_8))

    fun send(peerId: String, topic: String, payload: String) =
        NodeRepository.send(peerId, topic, payload.toByteArray(Charsets.UTF_8))

    fun refresh() = NodeRepository.requestRefresh()
    fun clearError() = NodeRepository.clearError()
    fun updateLocalNetworkPermission(granted: Boolean?) =
        NodeRepository.updateLocalNetworkPermission(granted)

    fun saveConfigAndRestart(context: Context, config: String) =
        NodeRepository.saveConfigAndRestart(context.applicationContext, config)
}
