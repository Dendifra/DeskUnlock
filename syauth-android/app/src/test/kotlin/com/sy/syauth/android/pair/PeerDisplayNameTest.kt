package com.sy.syauth.android.pair

import com.sy.syauth.android.pair.impl.decodePeerDisplayName
import java.nio.charset.CharacterCodingException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class PeerDisplayNameTest {
    @Test
    fun hostname_from_protocol_is_not_replaced_with_brand_or_personal_machine_name() {
        for (name in listOf("office-workstation", "Computer di Léa", "Galaxy S26", "syauth-laptop")) {
            assertEquals(name, decodePeerDisplayName(byteArrayOf(1) + name.toByteArray(Charsets.UTF_8)))
        }
    }

    @Test
    fun unsupported_empty_oversized_and_control_character_metadata_is_rejected() {
        for (bytes in listOf(
            byteArrayOf(), byteArrayOf(1), byteArrayOf(2, 65),
            byteArrayOf(1) + "a".repeat(129).toByteArray(),
            byteArrayOf(1) + "host\nname".toByteArray(),
            byteArrayOf(1, 32),
        )) {
            assertThrows(IllegalArgumentException::class.java) { decodePeerDisplayName(bytes) }
        }
        assertThrows(CharacterCodingException::class.java) {
            decodePeerDisplayName(byteArrayOf(1, 0xC3.toByte(), 0x28))
        }
    }
}
