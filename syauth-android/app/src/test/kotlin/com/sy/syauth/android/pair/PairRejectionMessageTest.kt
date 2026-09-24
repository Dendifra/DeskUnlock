package com.sy.syauth.android.pair

import com.sy.syauth.android.pair.api.PEER_REJECTED_REASON
import com.sy.syauth.android.pair.impl.confirmationFailureReason
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The two halves of one promise: a refusal by the computer must reach the
 * operator as an instruction.
 *
 * The computer only accepts an inbound bond request while its pairing is armed
 * (SPEC §6 T-004), so a refusal is the normal outcome of an un-armed computer —
 * not a silent no-op and not a timeout (BUG-20260924: the operator saw "non
 * succede nulla").
 */
class PairRejectionMessageTest {
    /** Wire operation the desktop publishes when it refuses the transaction. */
    private val opReject: Byte = 3

    /** Wire operation for "the operator already confirmed". */
    private val opConfirm: Byte = 2

    @Test
    fun a_refusal_keeps_its_meaning_out_of_the_transaction() {
        assertEquals(PEER_REJECTED_REASON, confirmationFailureReason(opReject))
    }

    @Test
    fun a_timeout_is_not_reported_as_a_refusal() {
        assertEquals("remote confirmation failed", confirmationFailureReason(opConfirm))
        assertEquals("remote confirmation failed", confirmationFailureReason(null))
    }

    @Test
    fun a_refusal_is_turned_into_the_next_action_on_screen() {
        assertEquals(
            "open DeskUnlock on the computer and try again",
            failure_message(
                PEER_REJECTED_REASON,
                help = "open DeskUnlock on the computer and try again",
                prefix = "Pairing failed: ",
            ),
        )
    }

    @Test
    fun every_other_failure_still_names_itself() {
        assertEquals(
            "Pairing failed: boom",
            failure_message("boom", help = "help", prefix = "Pairing failed: "),
        )
    }
}
