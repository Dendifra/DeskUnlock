// DEV-001 (CDM pivot): Robolectric tests pinning the new
// [RealPairBackend] runtime contract after the migration from
// `BluetoothLeScanner.startScan` to
// `CompanionDeviceManager.associate`:
//
//   1. `startScan()` invokes the injected [PairCompanionScanner]
//      with two slot UUIDs (current minute, previous minute) derived
//      from `sessionUuidForBond(zero, n)`. The OS picker is
//      simulated by the fake scanner driving the captured `onPicked`
//      callback synchronously.
//   2. The pairing-request receiver registered at construction time
//      accepts variant 2 (PASSKEY_CONFIRMATION) and rejects variant 3
//      (JUST_WORKS).
//   3. The bond-state receiver registered at construction time
//      resolves the LescResult deferred when an
//      `ACTION_BOND_STATE_CHANGED` intent reports `BOND_BONDED`.
//
// Limitation: the production [AndroidCdmPairCompanionScanner] uses
// an `ActivityResultLauncher<IntentSenderRequest>` that Robolectric
// cannot fully model (the launcher dispatches into the real Android
// `ActivityResultRegistry`, which is bound to a real Activity
// lifecycle). The fakes here exercise the seam contract only; the
// end-to-end IntentSender path is validated on-device by the
// orchestrator's e2e probe.
//
// Journey: specs/journeys/JOURNEY-DEV-001-real-lesc.md
package com.sy.syauth.android.pair

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.util.Base64
import androidx.test.core.app.ApplicationProvider
import com.sy.syauth.android.pair.api.LescResult
import com.sy.syauth.android.pair.api.PickPeerResult
import com.sy.syauth.android.pair.impl.BondStateBroadcastReceiver
import com.sy.syauth.android.pair.impl.EXTRA_PAIRING_KEY_NAME
import com.sy.syauth.android.pair.impl.EXTRA_PAIRING_VARIANT_NAME
import com.sy.syauth.android.pair.impl.KEYSTORE_ALIAS_PREFIX
import com.sy.syauth.android.pair.impl.KEYSTORE_MINT_FAILED_PREFIX
import com.sy.syauth.android.pair.impl.KEYSTORE_UNAVAILABLE_REASON
import com.sy.syauth.android.pair.impl.KeystoreEd25519KeyMaterial
import com.sy.syauth.android.pair.impl.KeystoreKeyGenerator
import com.sy.syauth.android.pair.impl.KeystoreKeygenError
import com.sy.syauth.android.pair.impl.PAIR_BOND_KEY_LEN
import com.sy.syauth.android.pair.impl.PAIR_PUBKEY_LEN
import com.sy.syauth.android.pair.impl.PAIRING_VARIANT_PASSKEY_CONFIRMATION
import com.sy.syauth.android.pair.impl.PairBondKeyDeriver
import com.sy.syauth.android.pair.impl.PairClock
import com.sy.syauth.android.pair.impl.PairCompanionScanner
import com.sy.syauth.android.pair.impl.PairGattExchange
import com.sy.syauth.android.pair.impl.PairSessionUuidLookup
import com.sy.syauth.android.pair.impl.PairingBroadcastReceiver
import com.sy.syauth.android.pair.impl.PairingVariantDecision
import com.sy.syauth.android.pair.impl.RealPairBackend
import com.sy.syauth.android.pair.impl.ReceiverRegistrar
import com.sy.syauth.android.pair.impl.decideAndroidPairingVariant
import com.sy.syauth.android.pair.impl.pairModeUuidsFor
import com.sy.syauth.android.pair.impl.transactionStateBelongsToAttempt
import com.sy.syauth.android.pair.api.PeerHandle
import java.util.UUID
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val FIXTURE_NOW_SECONDS: Long = 1_800_000_000L
private const val FIXTURE_MINUTE_CURRENT: Long = FIXTURE_NOW_SECONDS / 60L
private const val FIXTURE_MINUTE_PREVIOUS: Long = FIXTURE_MINUTE_CURRENT - 1L
private const val JUST_WORKS_VARIANT: Int = 3
private const val FIXTURE_PASSKEY: Int = 123_456
private const val FIXTURE_PEER_ADDR: String = "AA:BB:CC:DD:EE:01"
private const val FIXTURE_PEER_NAME: String = "test-desktop"
private const val FIXTURE_CANCEL_REASON: String = "user cancelled the companion-device picker"

/**
 * Deterministic lookup: every 16-byte UUID is `(key xor minute xor i)`
 * for the byte index `i`. Pure function; tests pin specific values.
 */
private fun lookupStub(key: ByteArray, minute: Long): ByteArray {
    val out = ByteArray(16)
    for (i in 0 until 16) {
        out[i] = (key[i % key.size].toInt() xor minute.toInt() xor i).toByte()
    }
    return out
}

/** Records calls + UUID sets so the test can assert without driving real CDM. */
private class RecordingCompanionScanner : PairCompanionScanner {
    var associateCalls: Int = 0
        private set
    var lastUuids: List<UUID> = emptyList()
        private set
    val lastOnPicked: AtomicReference<((String, String?) -> Unit)?> =
        AtomicReference(null)
    val lastOnFailed: AtomicReference<((String) -> Unit)?> = AtomicReference(null)
    var commitCalls: Int = 0
        private set
    var abandonCalls: Int = 0
        private set

    override fun commitSessionAssociation() {
        commitCalls += 1
    }

    override fun abandonSessionAssociation() {
        abandonCalls += 1
    }

    override fun associate(
        serviceUuids: List<UUID>,
        onPicked: (deviceAddress: String, deviceName: String?) -> Unit,
        onFailed: (reason: String) -> Unit,
    ) {
        associateCalls += 1
        lastUuids = serviceUuids
        lastOnPicked.set(onPicked)
        lastOnFailed.set(onFailed)
    }
}

/** Records registered receivers so the tests can drive `onReceive` directly. */
private class RecordingReceiverRegistrar : ReceiverRegistrar {
    val registered: MutableList<BroadcastReceiver> = mutableListOf()
    override fun register(receiver: BroadcastReceiver, filter: IntentFilter) {
        registered.add(receiver)
    }
    override fun unregister(receiver: BroadcastReceiver) {
        registered.remove(receiver)
    }
}

/** Returns a deterministic `bond_key` for any pair of pubkeys. */
private class XorPairBondKeyDeriver : PairBondKeyDeriver {
    override fun derive(hostPubkey: ByteArray, phonePubkey: ByteArray): ByteArray {
        val out = ByteArray(PAIR_BOND_KEY_LEN)
        for (i in 0 until PAIR_BOND_KEY_LEN) {
            out[i] = (hostPubkey[i % hostPubkey.size].toInt() xor phonePubkey[i % phonePubkey.size].toInt()).toByte()
        }
        return out
    }
}

/** Returns a canned 32-byte host pubkey; used by the DEV-002 runtime error-path tests. */
private class CannedHostPubkeyExchange(private val displayName: String? = null) : PairGattExchange {
    override fun peerDisplayName(): String? = displayName
    override fun readTransactionMessage(): ByteArray = byteArrayOf(2) + ByteArray(16) + byteArrayOf(1)

    /** A real peer accepts the V2 control write; this canned one always does. */
    override fun writeTransactionMessage(message: ByteArray): Boolean = true

    override fun exchangePubkeys(address: String, phonePubkey: ByteArray): ByteArray =
        ByteArray(PAIR_PUBKEY_LEN) { (it + 0x40).toByte() }
}

// ---------------------------------------------------------------------------
// V2 transaction fixtures. The Android-side state machine can be poisoned by a
// snapshot left over from a *previous* attempt; these pin the exact failure
// seen on hardware ("local confirmation transition rejected") and the retry
// lifecycle that must survive it.
// ---------------------------------------------------------------------------

private const val V2_VERSION: Byte = 2
private const val V2_OP_CAPABILITY: Byte = 1
private const val V2_OP_CONFIRM: Byte = 2
private const val V2_OP_REJECT: Byte = 3
private const val V2_OP_PREPARED: Byte = 6
private const val V2_OP_COMMIT: Byte = 7
private const val V2_OP_COMMITTED: Byte = 9
private const val V2_STATE_LEN: Int = 27
private const val V2_ROLE_PARTICIPANT: Byte = 1
private const val V2_PHASE_OOB_PENDING: Byte = 1
private const val TRANSACTION_PREF_NAME: String = "pairing-v2-recovery"

/** `[version:1][transaction-id:16][operation:1]`, matching `Message::encode`. */
private fun v2Message(id: ByteArray, operation: Byte): ByteArray =
    byteArrayOf(V2_VERSION) + id + byteArrayOf(operation)

/**
 * A persisted snapshot in exactly the state a stranded attempt leaves behind:
 * the phone had already taken its local CONFIRM for [transactionId], so the
 * Rust validator (`Transaction::local` → `confirm`) rejects a second one.
 * Layout matches `Transaction::encode`:
 * `[version:1][role:1][phase:1][id:16][caps:2][confirmed:2][prepared:2][committed:2]`.
 */
private fun stalePostConfirmSnapshot(transactionId: ByteArray): ByteArray =
    byteArrayOf(V2_VERSION, V2_ROLE_PARTICIPANT, V2_PHASE_OOB_PENDING) +
        transactionId +
        byteArrayOf(1, 1, 1, 0, 0, 0, 0, 0)

/**
 * Scripted V2 GATT session. The queue is consumed in order (the coordinator's
 * capability first, then each acknowledgement the phone waits for); every
 * message the phone publishes is recorded.
 */
private class ScriptedPairGattExchange(id: ByteArray, vararg reads: Byte) : PairGattExchange {
    val writes: MutableList<ByteArray> = mutableListOf()
    private val queued: ArrayDeque<ByteArray> = ArrayDeque(reads.map { v2Message(id, it) })

    override fun peerDisplayName(): String = "cached-desktop"

    override fun readTransactionMessage(): ByteArray? = queued.removeFirstOrNull()

    override fun writeTransactionMessage(message: ByteArray): Boolean {
        writes.add(message.copyOf())
        return true
    }

    override fun exchangePubkeys(address: String, phonePubkey: ByteArray): ByteArray =
        ByteArray(PAIR_PUBKEY_LEN) { (it + 0x40).toByte() }
}

/** Returns a pre-built [KeystoreEd25519KeyMaterial] regardless of alias input. */
private class FixedKeystoreKeyGenerator(
    private val material: KeystoreEd25519KeyMaterial = KeystoreEd25519KeyMaterial(
        alias = "syauth.ed25519.fixed",
        pubkey = ByteArray(PAIR_PUBKEY_LEN) { (it + 1).toByte() },
        strongBoxBacked = false,
    ),
) : KeystoreKeyGenerator {
    override fun generate(alias: String): KeystoreEd25519KeyMaterial = material
}

/** Throws the configured [KeystoreKeygenError] on every `generate` call. */
private class ThrowingKeystoreKeyGenerator(
    private val error: KeystoreKeygenError,
) : KeystoreKeyGenerator {
    override fun generate(alias: String): KeystoreEd25519KeyMaterial {
        throw error
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class RealPairBackendRuntimeTest {

    private fun makeBackend(
        companionScanner: PairCompanionScanner?,
        gattExchange: PairGattExchange?,
        pairingRegistrar: RecordingReceiverRegistrar = RecordingReceiverRegistrar(),
        bondStateRegistrar: RecordingReceiverRegistrar = RecordingReceiverRegistrar(),
        bondKeyDeriver: PairBondKeyDeriver = XorPairBondKeyDeriver(),
        keystoreKeyGenerator: KeystoreKeyGenerator? = null,
        adapter: BluetoothAdapter? = null,
    ): RealPairBackend = RealPairBackend(
        context = ApplicationProvider.getApplicationContext(),
        adapter = adapter,
        companionScanner = companionScanner,
        gattExchange = gattExchange,
        pairingReceiverRegistrar = pairingRegistrar,
        bondStateReceiverRegistrar = bondStateRegistrar,
        clock = PairClock { FIXTURE_NOW_SECONDS },
        sessionUuidLookup = PairSessionUuidLookup(::lookupStub),
        bondKeyDeriver = bondKeyDeriver,
        keystoreKeyGenerator = keystoreKeyGenerator,
    )

    // -------------------------------------------------------------
    // (1) Scanner filter UUIDs include current + previous minute.
    // -------------------------------------------------------------

    @Test
    fun pair_mode_uuids_for_returns_previous_current_and_future_minute_slots() {
        val uuids = pairModeUuidsFor(FIXTURE_NOW_SECONDS, PairSessionUuidLookup(::lookupStub))
        val expectedSize = 2 + com.sy.syauth.android.pair.impl.PAIR_FILTER_FUTURE_SLOTS
        assertEquals("must include previous + current + future slot UUIDs", expectedSize, uuids.size)
        val previous = lookupStub(ByteArray(PAIR_BOND_KEY_LEN), FIXTURE_MINUTE_PREVIOUS)
        val current = lookupStub(ByteArray(PAIR_BOND_KEY_LEN), FIXTURE_MINUTE_CURRENT)
        assertTrue("filter[0] must be previous minute", toBytes(uuids[0]).contentEquals(previous))
        assertTrue("filter[1] must be current minute", toBytes(uuids[1]).contentEquals(current))
        for (offset in 1..com.sy.syauth.android.pair.impl.PAIR_FILTER_FUTURE_SLOTS) {
            val expected = lookupStub(ByteArray(PAIR_BOND_KEY_LEN), FIXTURE_MINUTE_CURRENT + offset.toLong())
            assertTrue(
                "filter[${1 + offset}] must be current+$offset minute",
                toBytes(uuids[1 + offset]).contentEquals(expected),
            )
        }
    }

    @Test
    fun start_scan_associates_with_previous_current_and_future_minute_slot_uuids() {
        val scanner = RecordingCompanionScanner()
        val backend = makeBackend(companionScanner = scanner, gattExchange = null)
        backend.startScan()
        assertEquals("associate must fire exactly once", 1, scanner.associateCalls)
        val expectedSize = 2 + com.sy.syauth.android.pair.impl.PAIR_FILTER_FUTURE_SLOTS
        assertEquals("must request all slot UUIDs", expectedSize, scanner.lastUuids.size)
        val expectedPrevious = uuidFromBytes(lookupStub(ByteArray(PAIR_BOND_KEY_LEN), FIXTURE_MINUTE_PREVIOUS))
        val expectedCurrent = uuidFromBytes(lookupStub(ByteArray(PAIR_BOND_KEY_LEN), FIXTURE_MINUTE_CURRENT))
        assertEquals(expectedPrevious, scanner.lastUuids[0])
        assertEquals(expectedCurrent, scanner.lastUuids[1])
        for (offset in 1..com.sy.syauth.android.pair.impl.PAIR_FILTER_FUTURE_SLOTS) {
            val expectedFuture =
                uuidFromBytes(lookupStub(ByteArray(PAIR_BOND_KEY_LEN), FIXTURE_MINUTE_CURRENT + offset.toLong()))
            assertEquals(expectedFuture, scanner.lastUuids[1 + offset])
        }
        backend.cleanup()
    }

    @Test
    fun cdm_picker_resolves_peer_via_on_peer_picked_callback() {
        val scanner = RecordingCompanionScanner()
        val backend = makeBackend(companionScanner = scanner, gattExchange = null)
        val picked: AtomicReference<PeerHandle?> = AtomicReference(null)
        backend.setOnPeerPickedCallback { peer -> picked.set(peer) }
        backend.startScan()
        scanner.lastOnPicked.get()?.invoke(FIXTURE_PEER_ADDR, FIXTURE_PEER_NAME)
        val resolved = picked.get()
        assertNotNull("onPeerPicked must fire after CDM pick", resolved)
        assertEquals(FIXTURE_PEER_ADDR, resolved?.id)
        assertEquals(FIXTURE_PEER_NAME, resolved?.name)
        backend.cleanup()
    }

    @Test
    fun cdm_picker_cancel_resolves_via_on_scan_failed_callback() {
        val scanner = RecordingCompanionScanner()
        val backend = makeBackend(companionScanner = scanner, gattExchange = null)
        val reasonRef: AtomicReference<String?> = AtomicReference(null)
        backend.setOnScanFailedCallback { reason -> reasonRef.set(reason) }
        backend.startScan()
        scanner.lastOnFailed.get()?.invoke(FIXTURE_CANCEL_REASON)
        assertEquals(FIXTURE_CANCEL_REASON, reasonRef.get())
        backend.cleanup()
    }

    @Test
    fun backend_delegates_provisional_association_commit_and_abandon_to_the_scanner() {
        val scanner = RecordingCompanionScanner()
        val backend = makeBackend(companionScanner = scanner, gattExchange = null)

        backend.abandonProvisionalAssociation()
        backend.commitProvisionalAssociation()

        assertEquals("abandon must reach the CDM scanner", 1, scanner.abandonCalls)
        assertEquals("commit must reach the CDM scanner", 1, scanner.commitCalls)
        backend.cleanup()
    }

    @Test
    fun init_registers_pairing_request_and_bond_state_receivers() {
        val pairingReg = RecordingReceiverRegistrar()
        val bondReg = RecordingReceiverRegistrar()
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = null,
            pairingRegistrar = pairingReg,
            bondStateRegistrar = bondReg,
        )
        assertEquals("exactly one pairing-request receiver registered", 1, pairingReg.registered.size)
        assertEquals("exactly one bond-state receiver registered", 1, bondReg.registered.size)
        backend.cleanup()
        assertEquals("cleanup unregisters the pairing-request receiver", 0, pairingReg.registered.size)
        assertEquals("cleanup unregisters the bond-state receiver", 0, bondReg.registered.size)
    }

    // -------------------------------------------------------------
    // (2) Broadcast receiver gates on the pairing variant.
    // -------------------------------------------------------------

    @Test
    fun pairing_receiver_invokes_onaccept_only_for_passkey_confirmation_variant() {
        var accepted: Int = -1
        var rejected: Int = -1
        val receiver = PairingBroadcastReceiver(
            onAccept = { accepted = it },
            onReject = { rejected = it },
        )
        val intent = Intent(BluetoothDevice.ACTION_PAIRING_REQUEST).apply {
            putExtra(EXTRA_PAIRING_VARIANT_NAME, PAIRING_VARIANT_PASSKEY_CONFIRMATION)
            putExtra(EXTRA_PAIRING_KEY_NAME, FIXTURE_PASSKEY)
        }
        receiver.onReceive(ApplicationProvider.getApplicationContext(), intent)
        assertEquals(FIXTURE_PASSKEY, accepted)
        assertEquals(-1, rejected)
    }

    @Test
    fun pairing_receiver_invokes_onreject_for_just_works_variant() {
        var accepted: Int = -1
        var rejected: Int = -1
        val receiver = PairingBroadcastReceiver(
            onAccept = { accepted = it },
            onReject = { rejected = it },
        )
        val intent = Intent(BluetoothDevice.ACTION_PAIRING_REQUEST).apply {
            putExtra(EXTRA_PAIRING_VARIANT_NAME, JUST_WORKS_VARIANT)
            putExtra(EXTRA_PAIRING_KEY_NAME, FIXTURE_PASSKEY)
        }
        receiver.onReceive(ApplicationProvider.getApplicationContext(), intent)
        assertEquals(-1, accepted)
        assertEquals(JUST_WORKS_VARIANT, rejected)
    }

    @Test
    fun decide_android_pairing_variant_returns_typed_accept_for_lesc() {
        val decision = decideAndroidPairingVariant(PAIRING_VARIANT_PASSKEY_CONFIRMATION, FIXTURE_PASSKEY)
        assertTrue(
            "LESC variant must accept, got $decision",
            decision is PairingVariantDecision.AcceptPasskeyConfirmation,
        )
    }

    @Test
    fun decide_android_pairing_variant_returns_typed_reject_for_just_works() {
        val decision = decideAndroidPairingVariant(JUST_WORKS_VARIANT, FIXTURE_PASSKEY)
        assertTrue(
            "Just Works variant must reject, got $decision",
            decision is PairingVariantDecision.Reject,
        )
    }

    // -------------------------------------------------------------
    // (3) Bond-state receiver resolves the LescResult deferred.
    // -------------------------------------------------------------

    @Test
    fun bond_state_receiver_invokes_onbonded_on_bond_bonded_intent() {
        var bondedAddr: String? = null
        var failureReason: String? = null
        val receiver = BondStateBroadcastReceiver(
            onBonded = { bondedAddr = it },
            onFailed = { failureReason = it },
        )
        // Robolectric uses a real BluetoothDevice from the platform's
        // BluetoothManager; we use the BluetoothAdapter shadow to
        // construct one. The receiver pulls EXTRA_DEVICE from the intent,
        // so we synthesize a Parcelable that maps to the FIXTURE_PEER_ADDR.
        val adapter = android.bluetooth.BluetoothAdapter.getDefaultAdapter()
        val device = adapter?.getRemoteDevice(FIXTURE_PEER_ADDR)
        assertNotNull("Robolectric must surface a BluetoothAdapter", adapter)
        assertNotNull("Robolectric must surface a remote device", device)
        val intent = Intent(BluetoothDevice.ACTION_BOND_STATE_CHANGED).apply {
            putExtra(BluetoothDevice.EXTRA_BOND_STATE, BluetoothDevice.BOND_BONDED)
            putExtra(BluetoothDevice.EXTRA_PREVIOUS_BOND_STATE, BluetoothDevice.BOND_BONDING)
            putExtra(BluetoothDevice.EXTRA_DEVICE, device)
        }
        receiver.onReceive(ApplicationProvider.getApplicationContext(), intent)
        assertEquals(FIXTURE_PEER_ADDR, bondedAddr)
        assertNull("no failure path must fire", failureReason)
    }

    @Test
    fun bond_state_receiver_invokes_onfailed_on_bonding_to_none_transition() {
        var bondedAddr: String? = null
        var failureReason: String? = null
        val receiver = BondStateBroadcastReceiver(
            onBonded = { bondedAddr = it },
            onFailed = { failureReason = it },
        )
        val adapter = android.bluetooth.BluetoothAdapter.getDefaultAdapter()
        val device = adapter?.getRemoteDevice(FIXTURE_PEER_ADDR)
        val intent = Intent(BluetoothDevice.ACTION_BOND_STATE_CHANGED).apply {
            putExtra(BluetoothDevice.EXTRA_BOND_STATE, BluetoothDevice.BOND_NONE)
            putExtra(BluetoothDevice.EXTRA_PREVIOUS_BOND_STATE, BluetoothDevice.BOND_BONDING)
            putExtra(BluetoothDevice.EXTRA_DEVICE, device)
        }
        receiver.onReceive(ApplicationProvider.getApplicationContext(), intent)
        assertNull("no bonded path must fire", bondedAddr)
        assertNotNull("BOND_BONDING -> BOND_NONE must fire onFailed", failureReason)
    }

    // -------------------------------------------------------------
    // DEV-002 (re-march) — `runPostBondExchange` error surfaces.
    // -------------------------------------------------------------

    @Test
    fun runPostBondExchange_without_gatt_seam_completes_failed_with_typed_reason() {
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = null,
            keystoreKeyGenerator = FixedKeystoreKeyGenerator(),
        )
        backend.runPostBondExchange(FIXTURE_PEER_ADDR)
        val result = backend.awaitLescResult()
        assertTrue("expected Failed, got $result", result is LescResult.Failed)
        assertEquals(
            "no GATT exchange wired",
            (result as LescResult.Failed).reason,
        )
        backend.cleanup()
    }

    @Test
    fun runPostBondExchange_without_keystore_generator_refuses_to_ship_zero_pubkey() {
        // SPEC §3.2 D6: no keystore generator wired => no Keystore-resident
        // Ed25519 keypair => must NOT proceed and ship a zero-pubkey. The
        // pre-Tiramisu / test path now surfaces a typed `LescResult.Failed`.
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = CannedHostPubkeyExchange(),
            keystoreKeyGenerator = null,
        )
        backend.runPostBondExchange(FIXTURE_PEER_ADDR)
        val result = backend.awaitLescResult()
        assertTrue("expected Failed, got $result", result is LescResult.Failed)
        assertEquals(KEYSTORE_UNAVAILABLE_REASON, (result as LescResult.Failed).reason)
        backend.cleanup()
    }

    @Test
    fun runPostBondExchange_propagates_keystore_keygen_error_as_failed() {
        // The previous diagnostic-swallow path returned `null` on any
        // throw and silently shipped a zero-pubkey. The new contract:
        // `KeystoreKeygenError` propagates to `runPostBondExchange`,
        // which catches it and resolves the deferred with a typed
        // `LescResult.Failed` carrying the mint-failure prefix.
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = CannedHostPubkeyExchange(),
            keystoreKeyGenerator = ThrowingKeystoreKeyGenerator(
                error = KeystoreKeygenError.UnsupportedApi(sdkInt = 32),
            ),
        )
        backend.runPostBondExchange(FIXTURE_PEER_ADDR)
        val result = backend.awaitLescResult()
        assertTrue("expected Failed, got $result", result is LescResult.Failed)
        val failed = result as LescResult.Failed
        assertTrue(
            "reason should start with mint-failed prefix, got: ${failed.reason}",
            failed.reason.startsWith(KEYSTORE_MINT_FAILED_PREFIX),
        )
        backend.cleanup()
    }

    @Test
    fun runPostBondExchange_uses_hostname_from_the_same_peer_not_its_address() {
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = CannedHostPubkeyExchange("office-workstation"),
            keystoreKeyGenerator = FixedKeystoreKeyGenerator(),
        )
        try {
            backend.runPostBondExchange(FIXTURE_PEER_ADDR)
            val result = backend.awaitLescResult() as LescResult.Bonded
            assertEquals("office-workstation", result.peerName)
        } finally {
            backend.cleanup()
        }
    }

    @Test
    fun runPostBondExchange_success_propagates_keystore_alias_and_pubkey_into_bonded() {
        val expectedAlias = "$KEYSTORE_ALIAS_PREFIX${FIXTURE_PEER_ADDR.replace(":", "")}"
        val phonePubkey = ByteArray(PAIR_PUBKEY_LEN) { (it + 1).toByte() }
        val generator = FixedKeystoreKeyGenerator(
            material = KeystoreEd25519KeyMaterial(
                alias = expectedAlias,
                pubkey = phonePubkey,
                strongBoxBacked = false,
            ),
        )
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = CannedHostPubkeyExchange(),
            keystoreKeyGenerator = generator,
        )
        backend.runPostBondExchange(FIXTURE_PEER_ADDR)
        val result = backend.awaitLescResult()
        assertTrue("expected Bonded, got $result", result is LescResult.Bonded)
        val bonded = result as LescResult.Bonded
        assertEquals(expectedAlias, bonded.keystoreAlias)
        assertTrue(
            "phonePubkey must round-trip into the Bonded result",
            bonded.phonePubkey.contentEquals(phonePubkey),
        )
        backend.cleanup()
    }

    @Test
    fun already_bonded_peer_continues_the_deskunlock_handshake_not_a_bluetooth_success() {
        val adapter = BluetoothAdapter.getDefaultAdapter()
        val device = adapter.getRemoteDevice(FIXTURE_PEER_ADDR)
        shadowOf(device).setBondState(BluetoothDevice.BOND_BONDED)
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = CannedHostPubkeyExchange("office-workstation"),
            keystoreKeyGenerator = FixedKeystoreKeyGenerator(),
            adapter = adapter,
        )
        try {
            val started = backend.pickPeer(PeerHandle(FIXTURE_PEER_ADDR, "cachyos-x8664"))
            assertTrue(
                "an already Bluetooth-bonded peer must still enter the DeskUnlock handshake",
                started is PickPeerResult.LescStarted,
            )
            // BOND_BONDED is transport only: the outcome must come from the
            // DeskUnlock handshake, never from the Bluetooth bond.
            val result = backend.awaitLescResult()
            assertTrue(
                "Bluetooth BOND_BONDED must not complete DeskUnlock; got $result",
                result is LescResult.Bonded,
            )
        } finally {
            backend.cleanup()
        }
    }

    // -------------------------------------------------------------
    // The capability is a transport fact, published before any human decision.
    // -------------------------------------------------------------

    @Test
    fun the_capability_is_published_before_the_operators_oob_decision() {
        val adapter = BluetoothAdapter.getDefaultAdapter()
        shadowOf(adapter.getRemoteDevice(FIXTURE_PEER_ADDR)).setBondState(BluetoothDevice.BOND_BONDED)
        val attemptId = ByteArray(16) { 0x55 }
        val exchange = ScriptedPairGattExchange(attemptId, V2_OP_CAPABILITY)
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = exchange,
            keystoreKeyGenerator = FixedKeystoreKeyGenerator(),
            adapter = adapter,
        )
        try {
            backend.pickPeer(PeerHandle(FIXTURE_PEER_ADDR, "cachyos-x8664"))
            assertTrue(backend.awaitLescResult() is LescResult.Bonded)

            // The desktop raises its OOB dialog only after this message lands,
            // so it must be published during the key exchange: the operator then
            // compares both screens instead of waiting for a serialized round.
            assertEquals("exactly one capability is published", 1, exchange.writes.size)
            val published = exchange.writes.first()
            assertEquals(V2_OP_CAPABILITY, published[17])
            assertArrayEquals(attemptId, published.copyOfRange(1, 17))
        } finally {
            backend.cleanup()
        }
    }

    // -------------------------------------------------------------
    // A new attempt must never advance an older attempt's snapshot.
    // -------------------------------------------------------------

    @Test
    fun transaction_state_ownership_rejects_snapshots_of_other_attempts() {
        val attemptId = ByteArray(16) { 0x11 }
        val snapshot = stalePostConfirmSnapshot(attemptId)

        assertTrue(
            "the current attempt's snapshot is resumable",
            transactionStateBelongsToAttempt(snapshot, attemptId),
        )
        assertTrue(
            "a snapshot minted for another attempt is never resumed",
            !transactionStateBelongsToAttempt(snapshot, ByteArray(16) { 0x22 }),
        )
        assertTrue("no snapshot means no ownership", !transactionStateBelongsToAttempt(null, attemptId))
        assertTrue(
            "a blob that is not a 27-byte snapshot has no ownership",
            !transactionStateBelongsToAttempt(ByteArray(V2_STATE_LEN - 1), attemptId),
        )
    }

    @Test
    fun a_foreign_snapshot_is_discarded_before_the_new_attempt_uses_it() {
        val context: Context = ApplicationProvider.getApplicationContext()
        val prefs = context.getSharedPreferences(TRANSACTION_PREF_NAME, Context.MODE_PRIVATE)
        prefs.edit()
            .putString("state", Base64.encodeToString(stalePostConfirmSnapshot(ByteArray(16) { 0x11 }), Base64.NO_WRAP))
            .commit()

        val adapter = BluetoothAdapter.getDefaultAdapter()
        shadowOf(adapter.getRemoteDevice(FIXTURE_PEER_ADDR)).setBondState(BluetoothDevice.BOND_BONDED)
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = ScriptedPairGattExchange(ByteArray(16) { 0x22 }, V2_OP_CAPABILITY),
            keystoreKeyGenerator = FixedKeystoreKeyGenerator(),
            adapter = adapter,
        )
        try {
            backend.pickPeer(PeerHandle(FIXTURE_PEER_ADDR, "cachyos-x8664"))
            assertTrue(backend.awaitLescResult() is LescResult.Bonded)

            // The desktop's capability for THIS attempt carries a new id.
            backend.coordinateTransaction { true }

            assertNull(
                "a snapshot minted for another attempt must be dropped, never advanced",
                prefs.getString("state", null),
            )
        } finally {
            backend.cleanup()
        }
    }

    @Test
    fun a_snapshot_of_the_current_attempt_survives_for_recovery() {
        val context: Context = ApplicationProvider.getApplicationContext()
        val prefs = context.getSharedPreferences(TRANSACTION_PREF_NAME, Context.MODE_PRIVATE)
        val attemptId = ByteArray(16) { 0x44 }
        val snapshot = Base64.encodeToString(stalePostConfirmSnapshot(attemptId), Base64.NO_WRAP)
        prefs.edit().putString("state", snapshot).commit()

        val adapter = BluetoothAdapter.getDefaultAdapter()
        shadowOf(adapter.getRemoteDevice(FIXTURE_PEER_ADDR)).setBondState(BluetoothDevice.BOND_BONDED)
        val backend = makeBackend(
            companionScanner = null,
            gattExchange = ScriptedPairGattExchange(attemptId, V2_OP_CAPABILITY),
            keystoreKeyGenerator = FixedKeystoreKeyGenerator(),
            adapter = adapter,
        )
        try {
            backend.pickPeer(PeerHandle(FIXTURE_PEER_ADDR, "cachyos-x8664"))
            assertTrue(backend.awaitLescResult() is LescResult.Bonded)

            backend.coordinateTransaction { true }

            assertEquals(
                "a snapshot of the current attempt is the recovery contract: never dropped",
                snapshot,
                prefs.getString("state", null),
            )
        } finally {
            backend.cleanup()
        }
    }

    // -------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------

    private fun toBytes(uuid: java.util.UUID): ByteArray {
        val out = ByteArray(16)
        val msb = uuid.mostSignificantBits
        val lsb = uuid.leastSignificantBits
        for (i in 0 until 8) {
            out[7 - i] = ((msb shr (i * 8)) and 0xFF).toByte()
        }
        for (i in 0 until 8) {
            out[15 - i] = ((lsb shr (i * 8)) and 0xFF).toByte()
        }
        return out
    }

    private fun uuidFromBytes(bytes: ByteArray): UUID {
        var msb = 0L
        var lsb = 0L
        for (i in 0 until 8) {
            msb = (msb shl 8) or (bytes[i].toLong() and 0xFFL)
        }
        for (i in 8 until 16) {
            lsb = (lsb shl 8) or (bytes[i].toLong() and 0xFFL)
        }
        return UUID(msb, lsb)
    }
}
