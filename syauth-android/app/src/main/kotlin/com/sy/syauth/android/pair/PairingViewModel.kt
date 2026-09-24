// Roadmap item S-016 — Pairing ViewModel.
//
// The state machine is the canonical SPEC §4.4 pairing workflow:
//
//   Idle ──[onStartScanTapped]──▶ Scanning
//   Scanning ──[onPeerPicked, adapter supports LESC]──▶ LescNegotiating(code)
//   Scanning ──[onPeerPicked, LESC unsupported]──▶ Failed("adapter $name …")
//   LescNegotiating ──[onLescBondCompleted(bondKey)]──▶ OobConfirming(code)
//   LescNegotiating ──[onLescBondFailed(reason)]──▶ Failed(reason)
//   OobConfirming ──[onOobYesTapped]──▶ Bonded(name) [backend commit]
//   OobConfirming ──[onOobNoTapped]──▶ Failed("OOB code did not match …")
//                                      [bondRemover.remove(peerId)]
//   Scanning|… ──[onCancelTapped]──▶ Idle
//
// Architectural notes (mirrors prrr-android's QRScanViewModel):
//   - `StateFlow<PairingState>` is the single source of truth.
//   - Side-effect dependencies (`backend`, `oobCalculator`,
//     `bondRemover`) are injected as interfaces so the unit test wires
//     hand-rolled fakes (no mockk, per the brief).
//   - All transitions happen synchronously on the caller's thread; async
//     I/O (BT scan, LESC handshake) is the backend's job. This keeps the
//     test on `UnconfinedTestDispatcher` and asserts behavior, not timing.
//
// On `viewModelScope.launch { ... }`:
//   For S-016 we did not need this; every transition was synchronous.
//   S-018 added a single suspend hop on the OOB-yes happy path —
//   `companionAssociator.associate(peer)` shows an OS dialog and
//   resolves on a callback — so the Yes path now ends with a
//   `viewModelScope.launch(associateDispatcher) { ... }`. Tests pass
//   `Dispatchers.Unconfined` so the assertions in
//   PairingViewModelTest stay synchronous; production uses
//   `Dispatchers.Main.immediate`.
package com.sy.syauth.android.pair

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.sy.syauth.android.pair.api.BluetoothBondRemover
import com.sy.syauth.android.pair.api.BondRecord
import com.sy.syauth.android.pair.api.CompanionAssociationError
import com.sy.syauth.android.pair.api.CompanionAssociator
import com.sy.syauth.android.pair.api.LescResult
import com.sy.syauth.android.pair.api.OobCalculator
import com.sy.syauth.android.pair.api.PairBackend
import com.sy.syauth.android.pair.api.PeerHandle
import com.sy.syauth.android.pair.api.PickPeerResult
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Reason strings are constants so the unit test can assert on them
 * without a fragile string-equality. They are also the user-visible
 * message in the Failed branch of the Compose screen.
 */
internal object PairingReasons {
    const val ADAPTER_NO_LESC_PREFIX: String = "adapter "
    const val ADAPTER_NO_LESC_SUFFIX: String = " does not support LE Secure Connections"
    const val OOB_MISMATCH: String =
        "OOB code did not match — peer might be a relay attacker"
    const val PERSIST_PREFIX: String = "could not persist bond: "

    /**
     * Prefix for the Failed reason emitted when [CompanionAssociator]
     * returns a failure (S-018). The full string is
     * `companion-device association rejected: <reason>`.
     */
    const val ASSOCIATE_PREFIX: String = "companion-device association rejected: "
}

/**
 * No-op fallback [CompanionAssociator]. Used by callers that have not
 * been migrated to S-018 yet (the S-016-era test wiring); production
 * code never installs this — it injects [com.sy.syauth.android.pair.impl.RealCompanionAssociator].
 *
 * Returns a synthetic [com.sy.syauth.android.pair.api.AssociationHandle]
 * so the happy path still ends in [PairingState.Bonded] for tests that
 * don't exercise the associator seam directly.
 */
internal class NoopCompanionAssociator : CompanionAssociator {
    override suspend fun associate(
        peer: com.sy.syauth.android.pair.api.PeerHandle,
    ): Result<com.sy.syauth.android.pair.api.AssociationHandle> =
        Result.success(
            com.sy.syauth.android.pair.api.AssociationHandle(
                associationId = NOOP_ASSOCIATION_ID,
                peerId = peer.id,
            ),
        )

    private companion object {
        const val NOOP_ASSOCIATION_ID: Long = -1L
    }
}

class PairingViewModel(
    private val backend: PairBackend,
    private val oobCalculator: OobCalculator,
    private val bondRemover: BluetoothBondRemover,
    private val companionAssociator: CompanionAssociator = NoopCompanionAssociator(),
    private val associateDispatcher: CoroutineDispatcher = Dispatchers.Main.immediate,
    private val transactionDispatcher: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {

    private val _state: MutableStateFlow<PairingState> =
        MutableStateFlow(PairingState.Idle)

    /** Observable state for the screen. */
    val state: StateFlow<PairingState> = _state.asStateFlow()

    init {
        // A persisted transaction is evidence that the previous process did
        // not finish. Ask the backend to reconnect and reconcile it; any
        // transport or status ambiguity remains recoverable, never success.
        viewModelScope.launch(transactionDispatcher) {
            backend.recoverTransaction().onSuccess { bonded ->
                _state.value = if (bonded) PairingState.Bonded("") else PairingState.Uncertain("authenticated recovery is incomplete")
            }
        }
    }

    /**
     * Provisional pick: the peer the user tapped in the scan list. Held
     * here so the [LescNegotiating] → [OobConfirming] / [Bonded]
     * transitions can carry the peer identity forward without leaking
     * it into the state-class payload (which would force every state
     * variant to carry it).
     */
    private var pickedPeer: PeerHandle? = null

    /**
     * Transition Idle → Scanning. Called when the user taps the
     * "Pair with computer" CTA.
     */
    fun onStartScanTapped() {
        if (_state.value !is PairingState.Idle) return
        backend.startScan()
        _state.value = PairingState.Scanning
    }

    /**
     * Cancel from [Scanning] or [LescNegotiating] back to [Idle]. Stops
     * any in-flight scan; does NOT remove a BT bond (we are not bonded
     * yet at this point).
     */
    fun onCancelTapped() {
        when (_state.value) {
            is PairingState.Scanning, is PairingState.LescNegotiating, is PairingState.OobConfirming -> {
                backend.stopScan()
                backend.abortTransaction(cancel = true)
                // The CDM association created by the picker is provisional
                // until BONDED. Drop it so cancelling does not leave a
                // stale OS association behind.
                backend.abandonProvisionalAssociation()
                pickedPeer = null
                _state.value = PairingState.Idle
            }
            else -> Unit
        }
    }

    /**
     * Transition [Scanning] → [LescNegotiating] (or [Failed] if the
     * adapter lacks LESC). Called when the user picks a peer from the
     * scan results.
     */
    fun onPeerPicked(peer: PeerHandle) {
        if (_state.value !is PairingState.Scanning) return
        pickedPeer = peer
        when (val result = backend.pickPeer(peer)) {
            is PickPeerResult.LescStarted -> {
                _state.value = PairingState.LescNegotiating(result.code)
            }
            is PickPeerResult.LescUnsupported -> {
                backend.abandonProvisionalAssociation()
                _state.value = PairingState.Failed(
                    PairingReasons.ADAPTER_NO_LESC_PREFIX +
                        result.adapterName +
                        PairingReasons.ADAPTER_NO_LESC_SUFFIX,
                )
            }
            is PickPeerResult.Failed -> {
                backend.abandonProvisionalAssociation()
                _state.value = PairingState.Failed(result.reason)
            }
        }
    }

    /**
     * Update the displayed Bluetooth numeric-comparison code while the OS
     * pairing request is pending. Called by the backend (in production) or
     * directly by the test.
     */
    fun onPairingCode(code: String) {
        val current = _state.value
        if (current is PairingState.LescNegotiating) {
            _state.value = current.copy(code = code)
        }
    }

    /**
     * Drive the LESC outcome into the state machine. Called by the backend (in
     * production) or directly by the test. On success: compute the OOB via
     * UniFFI and transition to [OobConfirming]. On failure the LESC bond never
     * completed, so the transport bond is rolled back too.
     */
    fun onLescResult(result: LescResult) {
        if (_state.value !is PairingState.LescNegotiating) return
        when (result) {
            is LescResult.Bonded -> {
                val peer = pickedPeer ?: run {
                    backend.abandonProvisionalAssociation()
                    _state.value = PairingState.Failed("pairing peer selection was lost")
                    return
                }
                pickedPeer = peer.copy(name = result.peerName)
                val code = oobCalculator.compute(result.bondKey)
                // Stash the bond key + Keystore fields for the eventual
                // persist() call BEFORE emitting the new state, so a
                // same-thread observer who immediately reacts to
                // OobConfirming sees consistent stash on the subsequent
                // onOobYesTapped().
                stashedBondKey = result.bondKey
                stashedKeystoreAlias = result.keystoreAlias
                stashedPhonePubkey = result.phonePubkey
                _state.value = PairingState.OobConfirming(code)
            }
            is LescResult.Failed -> {
                backend.abandonProvisionalAssociation()
                removeBondBestEffort()
                _state.value = PairingState.Failed(result.reason)
            }
        }
    }

    /** Bond key carried from LescResult.Bonded to onOobYesTapped. */
    private var stashedBondKey: ByteArray? = null

    /** Keystore alias carried from LescResult.Bonded to onOobYesTapped (DEV-002). */
    private var stashedKeystoreAlias: String = ""

    /** Phone Ed25519 pubkey carried from LescResult.Bonded to onOobYesTapped (DEV-002). */
    private var stashedPhonePubkey: ByteArray = ByteArray(0)

    /**
     * User tapped Yes on the OOB-match question. Commit the bond through
     * the backend's bilateral transaction and transition to [Bonded]. The
     * backend persists only at its commit boundary; the terminal UI action
     * never writes storage. CDM association is completed by the picker path
     * before this point. The blocking transaction runs on the injected dispatcher.
     *
     * On a pre-commit failure the OS-level Bluetooth bond is PRESERVED: that
     * bond is transport, not DeskUnlock authorization, and no trust was created
     * (nothing was persisted and the transaction aborted before its commit
     * boundary). Dropping it would force the next attempt back through LESC.
     */
    fun onOobYesTapped() {
        if (_state.value !is PairingState.OobConfirming) return
        val peer = pickedPeer ?: return
        val bondKey = stashedBondKey ?: return
        _state.value = PairingState.Finalizing
        viewModelScope.launch(transactionDispatcher) {
            var persistFailure: String? = null
            val transaction = backend.coordinateTransaction {
                backend.persistBond(
                    BondRecord(
                        peerId = peer.id,
                        peerName = peer.name,
                        bondKey = bondKey,
                        keystoreAlias = stashedKeystoreAlias,
                        phonePubkey = stashedPhonePubkey,
                    ),
                ).onFailure { error ->
                    persistFailure = error.message ?: "unknown"
                }.isSuccess
            }
            if (transaction.isFailure) {
                val transactionReason = transaction.exceptionOrNull()?.message
                val reason = if (transactionReason?.startsWith("UNCERTAIN:") == true) transactionReason else if (persistFailure != null) PairingReasons.PERSIST_PREFIX + persistFailure else transactionReason ?: PairingReasons.PERSIST_PREFIX + "unknown"
                if (reason.startsWith("UNCERTAIN:")) {
                    // The remote commit decision was made; a durable local
                    // bond may already exist. Keep the CDM association so a
                    // committed bond is never orphaned.
                    _state.value = PairingState.Uncertain(reason.removePrefix("UNCERTAIN: "))
                } else {
                    // No DeskUnlock trust was created: the transaction aborted
                    // before its commit boundary and nothing was persisted. The
                    // OS-level Bluetooth bond is deliberately KEPT so the next
                    // "Associa telefono" can take the already-bonded path
                    // instead of a fresh LESC pairing.
                    backend.abandonProvisionalAssociation()
                    _state.value = PairingState.Failed(reason)
                }
                return@launch
            }
            // CDM association is completed by the scanner/picker before
            // this callback. Do not launch a second association after the
            // bilateral commit: that would create a false-success seam.
            // BONDED is the commit point for the provisional association.
            backend.commitProvisionalAssociation()
            _state.value = PairingState.Bonded(peer.name)
        }
    }

    /**
     * User tapped No on the OOB-match question: the four words shown by the
     * phone and by the desktop differ, which is exactly the MitM signal the OOB
     * step exists to catch. The transport bond is suspect, so it is rolled back
     * together with the transaction. The committed-bond backend is NEVER called
     * on this path.
     */
    fun onOobNoTapped() {
        if (_state.value !is PairingState.OobConfirming) return
        backend.abortTransaction(cancel = false)
        backend.abandonProvisionalAssociation()
        removeBondBestEffort()
        _state.value = PairingState.Failed(PairingReasons.OOB_MISMATCH)
    }

    /**
     * Best-effort BT bond cleanup, used on the two paths where unbonding is
     * correct: an LESC failure (no successful bond exists yet) and an explicit
     * OOB-mismatch rejection (the bond is suspect). It is deliberately NOT
     * called when a successful bond is followed by an application-level
     * failure. Returns silently on any failure; the committed-bond backend is
     * intentionally NOT consulted here.
     */
    private fun removeBondBestEffort() {
        val peer = pickedPeer ?: return
        @Suppress("UNUSED_VARIABLE")
        val removed: Boolean = bondRemover.remove(peer.id)
        // We deliberately ignore `removed`. The journey doc and SPEC §6
        // T-004 note both call out that BT cleanup is best-effort; the
        // app-level non-persistence is what matters.
    }

    /**
     * The pair route can be left with the system back gesture without
     * tapping Cancel. A session that never reached BONDED must drop its
     * provisional CDM association or it leaks in the OS. `Finalizing`,
     * `Uncertain`, and `Bonded` have already made the commit decision,
     * so their association is kept. The backend then releases its
     * receivers/session.
     */
    override fun onCleared() {
        super.onCleared()
        when (_state.value) {
            is PairingState.Idle,
            is PairingState.Scanning,
            is PairingState.LescNegotiating,
            is PairingState.OobConfirming,
            is PairingState.Failed,
            -> backend.abandonProvisionalAssociation()
            is PairingState.Finalizing,
            is PairingState.Uncertain,
            is PairingState.Bonded,
            -> Unit
        }
        backend.cleanup()
    }
}
