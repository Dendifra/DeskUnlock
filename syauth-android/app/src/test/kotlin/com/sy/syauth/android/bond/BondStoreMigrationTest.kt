package com.sy.syauth.android.bond

import java.io.File
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class BondStoreMigrationTest {
    @get:Rule val temporary = TemporaryFolder()

    private fun record(name: String) = BondRecord(
        peerId = "fixture-peer",
        hostName = name,
        bondKey = ByteArray(32) { 1 },
        keystoreAlias = "test-only-alias",
        phonePubkey = ByteArray(32) { 2 },
    )

    @Test
    fun only_exact_legacy_brand_is_migrated_and_persisted() {
        val store = BondStore(temporary.root)
        val old = record("syauth")
        store.save(old)
        val expected = old.copy(hostName = "DeskUnlock")
        assertEquals(expected, loadPersistedBond(temporary.root))
        assertEquals(expected, store.load())
        assertEquals(expected, loadPersistedBond(temporary.root))
    }

    @Test
    fun arbitrary_hostnames_are_never_renamed() {
        val store = BondStore(temporary.root)
        for (name in listOf("alex-desktop", "syauth-laptop", "Syauth", " syauth", "DeskUnlock")) {
            val expected = record(name)
            store.save(expected)
            assertEquals(expected, loadPersistedBond(temporary.root))
            assertEquals(expected, store.load())
        }
    }

    @Test
    fun failed_staging_write_preserves_previous_bond() {
        val store = BondStore(temporary.root)
        val previous = record("previous-computer")
        store.save(previous)
        File(temporary.root, "$BOND_RECORD_FILE_NAME.tmp").mkdir()
        assertThrows(IOException::class.java) { store.save(record("new-computer")) }
        assertEquals(previous, store.load())
    }

    @Test
    fun successful_replacement_is_readable_and_leaves_no_staging_file() {
        val store = BondStore(temporary.root)
        store.save(record("previous-computer"))
        val next = record("new-computer")
        store.save(next)
        assertEquals(next, store.load())
        assertEquals(false, File(temporary.root, "$BOND_RECORD_FILE_NAME.tmp").exists())
    }
}
