package io.github.peavey2787.p2pnet

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class DisplaySanitizationTest {
    @Test
    fun networkControlledTextCannotInjectControlsOrBidiOverrides() {
        val sanitized = sanitizeDisplayText("peer\u001B[2J\u202Ehidden\nnext", 128)

        assertFalse(sanitized.contains('\u001B'))
        assertFalse(sanitized.contains('\u202E'))
        assertTrue(sanitized.contains('�'))
        assertTrue(sanitized.endsWith("\nnext"))
    }

    @Test
    fun displayTextIsBoundedBeforeComposeRendersIt() {
        val sanitized = sanitizeDisplayText("abcdef", 4)

        assertEquals("abcd…", sanitized)
    }
}
