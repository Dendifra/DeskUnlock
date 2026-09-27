// Tests for [ProvisionalAssociationSession] — the bookkeeping that keeps a
// CDM association provisional until the pair reaches BONDED.
//
// These cover the disassociate semantics the ViewModel wiring relies on:
// only the current session's association is removed, commit freezes it,
// and abandon is idempotent.
package com.sy.syauth.android.pair.impl

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Records every disassociate callback the session makes. */
private class RecordingDisassociator {
    val calls: MutableList<Pair<Int, String?>> = mutableListOf()

    fun onDisassociate(id: Int, mac: String?) {
        calls.add(id to mac)
    }
}

class ProvisionalAssociationSessionTest {
    private val mac: String = "AA:BB:CC:DD:EE:FF"

    @Test
    fun abandon_disassociates_only_the_recorded_association() {
        val recorder = RecordingDisassociator()
        val session = ProvisionalAssociationSession(recorder::onDisassociate)
        session.begin()
        session.record(45, mac)

        session.abandon()

        assertEquals(listOf(45 to mac), recorder.calls)
    }

    @Test
    fun abandon_never_touches_associations_it_did_not_record() {
        val recorder = RecordingDisassociator()
        val session = ProvisionalAssociationSession(recorder::onDisassociate)
        session.begin()
        session.record(45, mac)

        session.abandon()

        // id 44 belongs to another session/device: never enumerated, never removed.
        assertTrue(recorder.calls.none { it.first == 44 })
        assertEquals(1, recorder.calls.size)
    }

    @Test
    fun abandon_is_idempotent() {
        val recorder = RecordingDisassociator()
        val session = ProvisionalAssociationSession(recorder::onDisassociate)
        session.begin()
        session.record(45, mac)

        session.abandon()
        session.abandon()

        assertEquals(1, recorder.calls.size)
    }

    @Test
    fun commit_prevents_a_later_abandon() {
        val recorder = RecordingDisassociator()
        val session = ProvisionalAssociationSession(recorder::onDisassociate)
        session.begin()
        session.record(45, mac)

        session.commit()
        session.abandon()

        assertTrue(recorder.calls.isEmpty())
    }

    @Test
    fun begin_starts_a_clean_session_so_retries_do_not_accumulate() {
        val recorder = RecordingDisassociator()
        val session = ProvisionalAssociationSession(recorder::onDisassociate)
        session.begin()
        session.record(45, mac)
        session.abandon()
        session.begin()
        session.record(46, mac)
        session.abandon()

        assertEquals(listOf(45 to mac, 46 to mac), recorder.calls)
    }

    @Test
    fun record_dedupes_repeated_observations_of_the_same_association() {
        val recorder = RecordingDisassociator()
        val session = ProvisionalAssociationSession(recorder::onDisassociate)
        session.begin()
        session.record(45, mac)
        session.record(45, mac)

        session.abandon()

        assertEquals(1, recorder.calls.size)
    }

    @Test
    fun record_dedupes_a_later_id_for_the_same_mac() {
        val recorder = RecordingDisassociator()
        val session = ProvisionalAssociationSession(recorder::onDisassociate)
        session.begin()
        session.record(UNKNOWN_ASSOCIATION_ID, mac)
        session.record(45, mac)

        session.abandon()

        assertEquals(1, recorder.calls.size)
    }

    @Test
    fun abandon_without_any_recorded_association_is_a_noop() {
        val recorder = RecordingDisassociator()
        val session = ProvisionalAssociationSession(recorder::onDisassociate)
        session.begin()

        session.abandon()

        assertTrue(recorder.calls.isEmpty())
    }
}
