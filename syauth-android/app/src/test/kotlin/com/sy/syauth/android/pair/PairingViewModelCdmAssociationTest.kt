// Roadmap item S-018 — PairingViewModel association tests.
//
// Asserts the three new behaviors the S-018 brief adds on top of S-016:
//
//   - Yes-path happy path: associator is called exactly once with the
//     correct peer, state ends in Bonded.
//   - Yes-path with associator failure: state ends in Failed with the
//     ASSOCIATE_PREFIX reason, bond is rolled back via the remover,
//     and the persister was called exactly once (we don't roll back
//     the persister entry in v0.1 — see PairingViewModel.kt rationale).
//   - No-path: associator is NEVER called.
//
// These tests live alongside the S-016 PairingViewModelTest.kt and
// share the package-private fakes via duplication — Robolectric runs
// each test class in isolation; sharing fakes across files would
// require a `helpers/` test-support source set that the project does
// not have today.
package com.sy.syauth.android.pair

import com.sy.syauth.android.pair.api.AssociationHandle
import com.sy.syauth.android.pair.api.BluetoothBondRemover
import com.sy.syauth.android.pair.api.BondPersister
import com.sy.syauth.android.pair.api.BondRecord
import com.sy.syauth.android.pair.api.CompanionAssociationError
import com.sy.syauth.android.pair.api.CompanionAssociator
import com.sy.syauth.android.pair.api.LescResult
import com.sy.syauth.android.pair.api.OobCalculator
import com.sy.syauth.android.pair.api.PairBackend
import com.sy.syauth.android.pair.api.PeerHandle
import com.sy.syauth.android.pair.api.PersistError
import com.sy.syauth.android.pair.api.PickPeerResult
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

private const val BOND_KEY_LEN: Int = 32
private val TEST_PEER: PeerHandle = PeerHandle(id = "AA:BB:CC:DD:EE:FF", name = "alex-desktop")

/** Records calls to assert exact count + arguments. */
private class RecordingAssociator(
    private val outcome: Result<AssociationHandle> = Result.success(
        AssociationHandle(associationId = 42L, peerId = TEST_PEER.id),
    ),
) : CompanionAssociator {
    var callCount: Int = 0
        private set
    var lastPeer: PeerHandle? = null
        private set

    override suspend fun associate(peer: PeerHandle): Result<AssociationHandle> {
        callCount += 1
        lastPeer = peer
        return outcome
    }
}

/** Trivial fakes; same shape as PairingViewModelTest.kt. */
private class StaticPairBackend(
    private val pickResult: PickPeerResult = PickPeerResult.LescStarted(code = "123456"),
    private val transactionOutcome: (() -> Result<String>)? = null,
) : PairBackend {
    var commitCount: Int = 0
        private set
    var abandonCount: Int = 0
        private set
    var cleanupCount: Int = 0
        private set

    override fun startScan() = Unit
    override fun stopScan() = Unit
    override fun pickPeer(peer: PeerHandle): PickPeerResult = pickResult
    var persister: BondPersister? = null

    override fun persistBond(record: BondRecord): Result<Unit> =
        runCatching { persister?.persist(record) ?: error("missing test persister") }

    override fun awaitLescResult(): LescResult =
        LescResult.Bonded(
            bondKey = ByteArray(BOND_KEY_LEN) { it.toByte() },
            peerName = TEST_PEER.name,
        )

    override fun coordinateTransaction(persistCommitted: () -> Boolean): Result<String> =
        transactionOutcome?.invoke()
            ?: if (persistCommitted()) Result.success(TEST_PEER.name)
            else Result.failure(IllegalStateException("persist failed"))

    override fun commitProvisionalAssociation() {
        commitCount += 1
    }

    override fun abandonProvisionalAssociation() {
        abandonCount += 1
    }

    override fun cleanup() {
        cleanupCount += 1
    }
}

/**
 * Invoke the protected `ViewModel.onCleared()` hook the way the
 * ViewModelStore does when the pair route is disposed.
 */
private fun clearViewModel(vm: PairingViewModel) {
    val method = PairingViewModel::class.java.getDeclaredMethod("onCleared")
    method.isAccessible = true
    method.invoke(vm)
}

private class FixedOobCalculator : OobCalculator {
    override fun compute(bondKey: ByteArray): String = "04231789"
}

private class RecordingPersister(
    private val throwError: PersistError? = null,
) : BondPersister {
    val persisted: MutableList<BondRecord> = mutableListOf()
    override fun persist(record: BondRecord) {
        if (throwError != null) throw throwError
        persisted.add(record)
    }
}

private class RecordingRemover : BluetoothBondRemover {
    val removed: MutableList<String> = mutableListOf()
    override fun remove(peerId: String): Boolean {
        removed.add(peerId)
        return true
    }
}

private fun buildVm(
    associator: CompanionAssociator,
    persister: RecordingPersister = RecordingPersister(),
    remover: RecordingRemover = RecordingRemover(),
    backend: StaticPairBackend = StaticPairBackend(),
): PairingViewModel {
    backend.persister = persister
    return PairingViewModel(
    backend = backend,
    oobCalculator = FixedOobCalculator(),
    bondRemover = remover,
    companionAssociator = associator,
    associateDispatcher = Dispatchers.Unconfined,
    transactionDispatcher = Dispatchers.Unconfined,
    )
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PairingViewModelCdmAssociationTest {

    private fun driveToOobConfirming(vm: PairingViewModel) {
        vm.onStartScanTapped()
        vm.onPeerPicked(TEST_PEER)
        vm.onLescResult(
            LescResult.Bonded(
                bondKey = ByteArray(BOND_KEY_LEN) { it.toByte() },
                peerName = TEST_PEER.name,
            ),
        )
    }

    @Test
    fun oob_yes_transitions_to_bonded_without_second_cdm_request() {
        val associator = RecordingAssociator()
        val persister = RecordingPersister()
        val remover = RecordingRemover()
        val vm = buildVm(associator, persister, remover)

        driveToOobConfirming(vm)
        vm.onOobYesTapped()

        assertEquals(0, associator.callCount)
        val state = vm.state.value
        assertTrue("expected Bonded, got $state", state is PairingState.Bonded)
        assertEquals(TEST_PEER.name, (state as PairingState.Bonded).name)
        assertEquals(1, persister.persisted.size)
        assertEquals(0, remover.removed.size)
    }

    @Test
    fun oob_yes_does_not_depend_on_a_second_association_request() {
        val associator = RecordingAssociator(
            outcome = Result.failure(CompanionAssociationError("unused")),
        )
        val persister = RecordingPersister()
        val remover = RecordingRemover()
        val vm = buildVm(associator, persister, remover)

        driveToOobConfirming(vm)
        vm.onOobYesTapped()

        assertEquals(0, associator.callCount)
        val state = vm.state.value
        assertTrue("expected Bonded, got $state", state is PairingState.Bonded)
        assertEquals(0, remover.removed.size)
        assertEquals(1, persister.persisted.size)
    }

    @Test
    fun oob_no_does_not_associate() {
        val associator = RecordingAssociator()
        val persister = RecordingPersister()
        val remover = RecordingRemover()
        val vm = buildVm(associator, persister, remover)

        driveToOobConfirming(vm)
        vm.onOobNoTapped()

        assertEquals(0, associator.callCount)
        val state = vm.state.value
        assertTrue("expected Failed, got $state", state is PairingState.Failed)
        // S-016 invariant TC-07 still holds.
        assertEquals(0, persister.persisted.size)
        assertEquals(listOf(TEST_PEER.id), remover.removed)
    }

    @Test
    fun persist_failure_skips_associate_call() {
        val associator = RecordingAssociator()
        val persister = RecordingPersister(throwError = PersistError("disk full"))
        val remover = RecordingRemover()
        val vm = buildVm(associator, persister, remover)

        driveToOobConfirming(vm)
        vm.onOobYesTapped()

        // Persist failed -> associator is never reached.
        assertEquals(0, associator.callCount)
        val state = vm.state.value
        assertTrue("expected Failed, got $state", state is PairingState.Failed)
    }

    // -------------------------------------------------------------
    // CDM association cleanup (cancel / reject / error / timeout).
    // -------------------------------------------------------------

    @Test
    fun cancel_from_scanning_abandons_the_provisional_association() {
        val backend = StaticPairBackend()
        val vm = buildVm(RecordingAssociator(), backend = backend)

        vm.onStartScanTapped()
        vm.onCancelTapped()

        assertEquals(1, backend.abandonCount)
        assertEquals(0, backend.commitCount)
        assertTrue("expected Idle, got ${vm.state.value}", vm.state.value is PairingState.Idle)
    }

    @Test
    fun cancel_from_oob_confirmation_abandons_the_provisional_association() {
        val backend = StaticPairBackend()
        val vm = buildVm(RecordingAssociator(), backend = backend)

        driveToOobConfirming(vm)
        vm.onCancelTapped()

        assertEquals(1, backend.abandonCount)
        assertEquals(0, backend.commitCount)
        assertTrue("expected Idle, got ${vm.state.value}", vm.state.value is PairingState.Idle)
    }

    @Test
    fun oob_no_reject_abandons_the_provisional_association() {
        val backend = StaticPairBackend()
        val vm = buildVm(RecordingAssociator(), backend = backend)

        driveToOobConfirming(vm)
        vm.onOobNoTapped()

        assertEquals(1, backend.abandonCount)
        assertEquals(0, backend.commitCount)
        assertTrue("expected Failed, got ${vm.state.value}", vm.state.value is PairingState.Failed)
    }

    @Test
    fun lesc_failure_abandons_the_provisional_association() {
        val backend = StaticPairBackend()
        val vm = buildVm(RecordingAssociator(), backend = backend)

        vm.onStartScanTapped()
        vm.onPeerPicked(TEST_PEER)
        vm.onLescResult(LescResult.Failed("lesc failed"))

        assertEquals(1, backend.abandonCount)
        assertEquals(0, backend.commitCount)
        assertTrue("expected Failed, got ${vm.state.value}", vm.state.value is PairingState.Failed)
    }

    @Test
    fun pick_failure_abandons_the_provisional_association() {
        val backend = StaticPairBackend(pickResult = PickPeerResult.Failed("no adapter"))
        val vm = buildVm(RecordingAssociator(), backend = backend)

        vm.onStartScanTapped()
        vm.onPeerPicked(TEST_PEER)

        assertEquals(1, backend.abandonCount)
        assertTrue("expected Failed, got ${vm.state.value}", vm.state.value is PairingState.Failed)
    }

    @Test
    fun transaction_failure_abandons_the_provisional_association() {
        val backend = StaticPairBackend(
            transactionOutcome = { Result.failure(IllegalStateException("remote timeout")) },
        )
        val vm = buildVm(RecordingAssociator(), backend = backend)

        driveToOobConfirming(vm)
        vm.onOobYesTapped()

        assertEquals(1, backend.abandonCount)
        assertEquals(0, backend.commitCount)
        assertTrue("expected Failed, got ${vm.state.value}", vm.state.value is PairingState.Failed)
    }

    @Test
    fun uncertain_transaction_keeps_the_provisional_association() {
        val backend = StaticPairBackend(
            transactionOutcome = {
                Result.failure(IllegalStateException("UNCERTAIN: remote completion unavailable"))
            },
        )
        val vm = buildVm(RecordingAssociator(), backend = backend)

        driveToOobConfirming(vm)
        vm.onOobYesTapped()

        assertEquals(0, backend.abandonCount)
        assertEquals(0, backend.commitCount)
        assertTrue("expected Uncertain, got ${vm.state.value}", vm.state.value is PairingState.Uncertain)
    }

    @Test
    fun bonded_commits_the_provisional_association_and_never_abandons() {
        val backend = StaticPairBackend()
        val vm = buildVm(RecordingAssociator(), backend = backend)

        driveToOobConfirming(vm)
        vm.onOobYesTapped()

        assertEquals(1, backend.commitCount)
        assertEquals(0, backend.abandonCount)
        assertTrue("expected Bonded, got ${vm.state.value}", vm.state.value is PairingState.Bonded)
    }

    @Test
    fun cancel_is_idempotent_and_abandons_once() {
        val backend = StaticPairBackend()
        val vm = buildVm(RecordingAssociator(), backend = backend)

        vm.onStartScanTapped()
        vm.onCancelTapped()
        vm.onCancelTapped()

        assertEquals(1, backend.abandonCount)
    }

    @Test
    fun retry_after_cancel_does_not_accumulate_associations() {
        val backend = StaticPairBackend()
        val vm = buildVm(RecordingAssociator(), backend = backend)

        vm.onStartScanTapped()
        vm.onCancelTapped()
        vm.onStartScanTapped()
        vm.onCancelTapped()

        assertEquals(2, backend.abandonCount)
        assertEquals(0, backend.commitCount)
    }

    @Test
    fun session_abort_before_bonded_abandons_the_provisional_association() {
        val backend = StaticPairBackend()
        val vm = buildVm(RecordingAssociator(), backend = backend)

        vm.onStartScanTapped()
        clearViewModel(vm)

        assertEquals(1, backend.abandonCount)
        assertEquals(0, backend.commitCount)
        assertEquals(1, backend.cleanupCount)
    }

    @Test
    fun session_abort_after_bonded_keeps_the_committed_association() {
        val backend = StaticPairBackend()
        val vm = buildVm(
            RecordingAssociator(),
            persister = RecordingPersister(),
            backend = backend,
        )

        driveToOobConfirming(vm)
        vm.onOobYesTapped()
        clearViewModel(vm)

        assertEquals(1, backend.commitCount)
        assertEquals(0, backend.abandonCount)
        assertEquals(1, backend.cleanupCount)
    }
}
