// Provisional CDM association tracking for one pair session.
//
// A CDM association is NOT the DeskUnlock bond. The OS creates it when
// the user picks a device in the CDM picker, which happens *before* the
// Pairing V2 transaction reaches BONDED. If the user cancels, rejects,
// times out, or the link fails before BONDED, that association must be
// removed. Otherwise the OS keeps one stale row per attempt until it
// refuses new associations ("Too many associations: ... already
// associated N devices within the last 3600000ms").
//
// This class owns exactly the associations recorded during the current
// session. It never enumerates or touches associations created by other
// sessions, other devices, or other apps. Cleanup is idempotent.
package com.sy.syauth.android.pair.impl

/** Association id placeholder used on pre-API-33 hosts that never surface one. */
internal const val UNKNOWN_ASSOCIATION_ID: Int = 0

/**
 * One CDM association created during the current pair session.
 *
 * @property id the OS-assigned association id, or [UNKNOWN_ASSOCIATION_ID]
 *   on hosts that predate `onAssociationCreated(AssociationInfo)`.
 * @property mac the uppercase device MAC, or `null` when the OS did not
 *   surface one. Used as the disassociate key on pre-API-33 hosts.
 */
internal data class ProvisionalAssociation(
    val id: Int,
    val mac: String?,
)

/**
 * Records the CDM associations of one pair session and disassociates
 * them on [abandon].
 *
 * - [begin] starts a new session (called when a new CDM request is made).
 * - [record] notes an association the OS created for the session.
 * - [commit] freezes the session after BONDED; a later [abandon] then
 *   never removes the committed association.
 * - [abandon] removes exactly the recorded, uncommitted associations and
 *   is idempotent.
 *
 * @param disassociate callback invoked once per association to remove.
 *   The production scanner wires it to `CompanionDeviceManager`; tests
 *   inject a recorder.
 */
internal class ProvisionalAssociationSession(
    private val disassociate: (id: Int, mac: String?) -> Unit,
) {
    private val associations = mutableListOf<ProvisionalAssociation>()
    private var committed = false

    /** Start a new session, discarding any previous session's bookkeeping. */
    @Synchronized
    fun begin() {
        associations.clear()
        committed = false
    }

    /** Record an association the OS created for the current session. */
    @Synchronized
    fun record(id: Int, mac: String?) {
        if (id == UNKNOWN_ASSOCIATION_ID && mac == null) return
        if (mac != null && associations.any { it.mac == mac }) return
        if (id != UNKNOWN_ASSOCIATION_ID && associations.any { it.id == id }) return
        associations.add(ProvisionalAssociation(id, mac))
    }

    /**
     * Mark the session BONDED; its association must never be removed.
     *
     * Returns the association ids that were committed, so the caller can prune
     * the associations superseded by this pairing while keeping this one.
     */
    @Synchronized
    fun commit(): List<Int> {
        committed = true
        val kept = associations.map { it.id }.filter { it != UNKNOWN_ASSOCIATION_ID }
        associations.clear()
        return kept
    }

    /** Remove every association recorded for the current session. */
    @Synchronized
    fun abandon() {
        if (committed) return
        val pending = associations.toList()
        associations.clear()
        for (association in pending) {
            disassociate(association.id, association.mac)
        }
    }
}
