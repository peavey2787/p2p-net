package io.github.peavey2787.p2pnet

import java.util.Base64
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFails

class MessageBoundsTest {
    @Test
    fun nativeBase64PayloadRoundTripsWithinDeclaredBound() {
        val payload = "hello android".toByteArray()
        val encoded = Base64.getEncoder().encodeToString(payload)
        assertContentEquals(payload, decodeNativePayload(encoded, payload.size))
    }

    @Test
    fun nativeBase64PayloadRejectsDeclaredLengthMismatch() {
        val encoded = Base64.getEncoder().encodeToString(byteArrayOf(1, 2, 3))
        assertFails { decodeNativePayload(encoded, 2) }
    }

    @Test
    fun uiMessageHistoryIsBoundedByCountAndPayloadBytes() {
        fun message(index: Int, bytes: Int) = ReceivedMessage(
            topic = "test",
            sourcePeerId = "peer-$index",
            targetPeerId = null,
            timestampNs = index.toLong(),
            payload = ByteArray(bytes) { index.toByte() },
        )

        val retained = appendBoundedMessages(
            current = (0 until 5).map { message(it, 4) },
            incoming = (5 until 10).map { message(it, 4) },
            maxMessages = 8,
            maxPayloadBytes = 20,
        )
        assertEquals(listOf(5L, 6L, 7L, 8L, 9L), retained.map { it.timestampNs })
        assertEquals(20, retained.sumOf { it.payload.size })
    }
}
