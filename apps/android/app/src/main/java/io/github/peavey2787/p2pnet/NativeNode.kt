package io.github.peavey2787.p2pnet

/** Thin JNI surface. UI and lifecycle code must go through [NodeRepository]. */
internal object NativeNode {
    init {
        System.loadLibrary("p2p_net_android")
        System.loadLibrary("p2p_android_jni")
    }

    external fun defaultConfig(): String
    external fun validateConfig(config: String): String
    external fun start(config: String, dataDir: String): String
    external fun stop(): String
    external fun revision(): Long
    external fun snapshot(): String
    external fun peers(): String
    external fun metrics(): String
    external fun bridgeStats(): String
    external fun connect(addr: String): String
    external fun disconnect(peerId: String): String
    external fun broadcast(topic: String, payload: ByteArray): String
    external fun send(peerId: String, topic: String, payload: ByteArray): String
    external fun subscribe(topic: String): String
    external fun pendingMessageCount(): Int
    external fun drainMessages(maxMessages: Int): String
}
