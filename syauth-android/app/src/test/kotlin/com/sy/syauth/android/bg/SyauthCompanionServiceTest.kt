// Roadmap item S-011 — Robolectric JVM tests for the foreground
// `SyauthCompanionService`. Pins the DoD bullets from
// `specs/unlock-proximity/ROADMAP.md` Step S-011 verbatim:
//
//   1. `starts_foreground_with_connected_device_type` — boots the
//      service via `Robolectric.buildService(...).create()` and
//      asserts the captured `lastForegroundType` field equals
//      `ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE`, the
//      shadow's `lastForegroundNotification` is non-null, and the
//      notification's channel id equals
//      `NOTIFICATION_CHANNEL_ID`. Robolectric 4.11.1's
//      `ShadowService` does not expose `getForegroundServiceType()`
//      directly, so we read the package-internal recording field the
//      service writes inside `startForeground`.
//   2. `injects_one_gatt_client_per_bond` — pre-seeds three bond
//      records via a `BondListProvider` seam and asserts the
//      recording `GattClientFactory` saw three `create(record)`
//      invocations whose peer ids match the fixtures.
//   3. `stops_clients_on_destroy` — drives `.destroy()` and asserts
//      every recording client saw exactly one `stop()` call.
//
// Journey: specs/journeys/JOURNEY-S-011-service-foreground-lifecycle.md
package com.sy.syauth.android.bg

import android.content.Intent
import android.content.pm.ServiceInfo
import com.sy.syauth.android.bond.BOND_RECORD_FILE_NAME
import com.sy.syauth.android.bond.BondRecord
import com.sy.syauth.android.bond.BondStore
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val FIXTURE_BOND_KEY_LEN: Int = 32
private const val FIXTURE_HOST: String = "alex-desktop"
private const val FIXTURE_PEER_A: String = "AA:AA:AA:AA:AA:AA"
private const val FIXTURE_PEER_B: String = "BB:BB:BB:BB:BB:BB"
private const val FIXTURE_PEER_C: String = "CC:CC:CC:CC:CC:CC"
private const val FIXTURE_KEYSTORE_ALIAS: String = "syauth.test.alias"

private fun bondFor(peerId: String, seed: Int = 0): BondRecord = BondRecord(
    peerId = peerId,
    hostName = FIXTURE_HOST,
    bondKey = ByteArray(FIXTURE_BOND_KEY_LEN) { (it + seed).toByte() },
    keystoreAlias = FIXTURE_KEYSTORE_ALIAS,
    phonePubkey = ByteArray(FIXTURE_BOND_KEY_LEN) { (it + seed).toByte() },
)

private class RecordingManagedClient : ManagedClient {
    var stopCalls: Int = 0
        private set
    var startCalls: Int = 0
        private set
    val sent: MutableList<ByteArray> = mutableListOf()
    override fun start() {
        startCalls += 1
    }

    override fun send(frameBytes: ByteArray): Boolean {
        sent += frameBytes
        return true
    }
    override fun stop() {
        stopCalls += 1
    }
}

private class RecordingGattClientFactory : GattClientFactory {
    val created: MutableList<Pair<String, RecordingManagedClient>> = mutableListOf()
    override fun create(bond: BondRecord): ManagedClient {
        val client = RecordingManagedClient()
        created += bond.peerId to client
        return client
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class SyauthCompanionServiceTest {

    @After
    fun cleanup() {
        SyauthCompanionService.resetSeams()
        ChallengeApprovalActivity.resetSeams()
        // `isRunning` is a JVM-static: a test that creates the service leaves it
        // true and every later `resurrectIfDead` call short-circuits, which is
        // how `boot_with_bond_starts_service` started failing. Reset it here so
        // the file cannot leak state into its neighbours.
        SyauthCompanionService.isRunning.set(false)
    }

    @Test
    fun starts_foreground_with_connected_device_type() {
        SyauthCompanionService.bondListProvider = BondListProvider { emptyList() }
        SyauthCompanionService.gattClientFactory = RecordingGattClientFactory()

        val controller = Robolectric.buildService(SyauthCompanionService::class.java).create()
        val service = controller.get()

        assertNotNull("service created", service)
        assertEquals(
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            service.lastForegroundType,
        )
        val notification = shadowOf(service).lastForegroundNotification
        assertNotNull("foreground notification posted", notification)
        assertEquals(NOTIFICATION_CHANNEL_ID, notification.channelId)
    }

    @Test
    fun injects_one_gatt_client_per_bond() {
        val bonds = listOf(
            bondFor(FIXTURE_PEER_A),
            bondFor(FIXTURE_PEER_B),
            bondFor(FIXTURE_PEER_C),
        )
        val factory = RecordingGattClientFactory()
        SyauthCompanionService.bondListProvider = BondListProvider { bonds }
        SyauthCompanionService.gattClientFactory = factory

        Robolectric.buildService(SyauthCompanionService::class.java).create()

        assertEquals(3, factory.created.size)
        assertEquals(FIXTURE_PEER_A, factory.created[0].first)
        assertEquals(FIXTURE_PEER_B, factory.created[1].first)
        assertEquals(FIXTURE_PEER_C, factory.created[2].first)
        for ((_, client) in factory.created) {
            assertTrue("client started", client.startCalls >= 1)
        }
    }

    /**
     * BUG-20260522-0130: when Android kills the app process under memory
     * pressure and `START_STICKY` brings `SyauthCompanionService` back
     * without going through `MainActivity`, the JVM-static
     * [SyauthCompanionService.gattClientFactory] is `null`. The previous
     * `injectClientsForBonds()` early-returned in that case, leaving
     * the resurrected service alive (foreground notification visible,
     * `isRunning=true`) but with `clients = []` — every challenge
     * from the desktop instant-failed `transport-error` until the user
     * manually opened the app UI. Regression guard: `onCreate` must
     * install a default factory itself so a process-restarted service
     * is self-sufficient.
     */
    @Test
    fun on_create_installs_default_gatt_client_factory_when_none_preset() {
        // Empty bond list keeps the test JVM-pure — we're asserting the
        // factory got installed, not exercising connectGatt. The
        // factory's `create` lambda is never invoked here.
        SyauthCompanionService.bondListProvider = BondListProvider { emptyList() }
        // gattClientFactory intentionally left null — simulates the
        // post-process-restart cold-start.

        Robolectric.buildService(SyauthCompanionService::class.java).create()

        assertNotNull(
            "onCreate must install a default GattClientFactory so a service " +
                "resurrected via START_STICKY can connect without MainActivity",
            SyauthCompanionService.gattClientFactory,
        )
    }

    /**
     * BUG-20260522-0138 (extension): even with `gattClientFactory`
     * defaulted, the approval path still failed on a process-restarted
     * service because four other JVM-static seams owned by
     * `MainActivity.installCompanionSeams` were null. The user observed
     * "tap Approve → app closes without biometric" — the activity bailed
     * with `alias=''` (`keystoreAliasResolver=null`) before reaching
     * BiometricPrompt. Regression guard: every load-bearing companion
     * seam must be non-null after `onCreate`.
     */
    @Test
    fun on_create_installs_default_companion_seams_when_none_preset() {
        SyauthCompanionService.bondListProvider = BondListProvider { emptyList() }
        // All seams intentionally left null — simulates the
        // post-process-restart cold-start where MainActivity has not
        // had a chance to call installCompanionSeams.

        Robolectric.buildService(SyauthCompanionService::class.java).create()

        assertNotNull(
            "onCreate must default bondKeyProvider",
            SyauthCompanionService.bondKeyProvider,
        )
        assertNotNull(
            "onCreate must default hostnameResolver",
            SyauthCompanionService.hostnameResolver,
        )
        assertNotNull(
            "onCreate must default keystoreAliasResolver (load-bearing for Approve)",
            SyauthCompanionService.keystoreAliasResolver,
        )
        assertNotNull(
            "onCreate must default challengeVerifier",
            SyauthCompanionService.challengeVerifier,
        )
        assertNotNull(
            "onCreate must default ChallengeApprovalActivity.responseSink " +
                "(load-bearing for Approve to deliver signature to host)",
            ChallengeApprovalActivity.responseSink,
        )
        assertNotNull(
            "onCreate must default ChallengeApprovalActivity.cancelSink",
            ChallengeApprovalActivity.cancelSink,
        )
    }

    @Test
    fun stops_clients_on_destroy() {
        val bonds = listOf(
            bondFor(FIXTURE_PEER_A),
            bondFor(FIXTURE_PEER_B),
            bondFor(FIXTURE_PEER_C),
        )
        val factory = RecordingGattClientFactory()
        SyauthCompanionService.bondListProvider = BondListProvider { bonds }
        SyauthCompanionService.gattClientFactory = factory

        val controller = Robolectric.buildService(SyauthCompanionService::class.java).create()
        controller.destroy()

        assertEquals(3, factory.created.size)
        for ((_, client) in factory.created) {
            assertEquals(1, client.stopCalls)
        }
    }

    /**
     * Day-2 regression guard (observed 2026-09-23): after the operator
     * dissociates and pairs again, the bond on disk changes while
     * `SyauthCompanionService` keeps running. It used to hold the client it
     * built for the *previous* bond, so the desktop saw `Connected: no`,
     * presence samples stopped and both the proximity lock and the phone
     * unlock went dead until the app was restarted by hand.
     *
     * `ACTION_RELOAD_BONDS` must therefore reconcile: stop + drop clients
     * whose bond is gone, start clients for new bonds, and leave the
     * surviving connection untouched (a reload must not churn what works).
     */
    @Test
    fun reload_drops_gone_bonds_and_starts_only_new_ones() {
        var bonds = listOf(bondFor(FIXTURE_PEER_A), bondFor(FIXTURE_PEER_B))
        val factory = RecordingGattClientFactory()
        SyauthCompanionService.bondListProvider = BondListProvider { bonds }
        SyauthCompanionService.gattClientFactory = factory

        val controller = Robolectric.buildService(SyauthCompanionService::class.java).create()
        val service = controller.get()
        assertEquals(2, factory.created.size)

        // The operator re-pairs: peer A is gone, peer C appears, B is kept.
        bonds = listOf(bondFor(FIXTURE_PEER_B), bondFor(FIXTURE_PEER_C))
        service.onStartCommand(Intent(SyauthCompanionService.ACTION_RELOAD_BONDS), 0, 0)

        val goneClient = factory.created.first { it.first == FIXTURE_PEER_A }.second
        val keptClient = factory.created.first { it.first == FIXTURE_PEER_B }.second
        val newClient = factory.created.firstOrNull { it.first == FIXTURE_PEER_C }?.second

        assertEquals("client for the vanished bond must be stopped", 1, goneClient.stopCalls)
        assertEquals(
            "the surviving bond must not be re-created",
            1,
            factory.created.count { it.first == FIXTURE_PEER_B },
        )
        assertEquals("the surviving client must not be restarted", 1, keptClient.startCalls)
        assertNotNull("the new bond must get a client", newClient)
        assertEquals("the new client must be started", 1, newClient?.startCalls)
    }

    /**
     * The real dissociate + re-associate flow keeps the same Bluetooth MAC:
     * only `bondKey` / `phonePubkey` change. Reconciling by MAC alone kept the
     * stale client, which stayed wedged against the desktop's torn-down GATT
     * service, so presence samples stopped and the proximity lock went dead
     * until the app was restarted by hand (observed 2026-09-24). The service
     * must rebuild the client when the bond changes under the same MAC.
     */
    @Test
    fun reload_rebuilds_client_when_the_same_mac_gets_a_new_bond() {
        var bonds = listOf(bondFor(FIXTURE_PEER_A, seed = 0))
        val factory = RecordingGattClientFactory()
        SyauthCompanionService.bondListProvider = BondListProvider { bonds }
        SyauthCompanionService.gattClientFactory = factory

        val controller = Robolectric.buildService(SyauthCompanionService::class.java).create()
        val service = controller.get()
        assertEquals(1, factory.created.size)
        val firstClient = factory.created.single().second

        // Dissociate + re-associate the same phone: same MAC, new bond key.
        bonds = listOf(bondFor(FIXTURE_PEER_A, seed = 1))
        service.onStartCommand(Intent(SyauthCompanionService.ACTION_RELOAD_BONDS), 0, 0)

        assertEquals("the stale client must be stopped", 1, firstClient.stopCalls)
        assertEquals(
            "the re-paired bond must get a fresh client",
            2,
            factory.created.count { it.first == FIXTURE_PEER_A },
        )
        val rebuilt = factory.created.last().second
        assertEquals("the rebuilt client must be started", 1, rebuilt.startCalls)
    }

    /**
     * Day-2 revocation (observed 2026-09-23: "if I dissociate in the app the PC
     * stays active"). The frame must carry the pubkey-derived 32-hex id — the
     * identity the desktop's bond store actually uses — not the Bluetooth MAC
     * in `BondRecord.peerId`, which only names the GATT link. Building the
     * frame from the MAC made `revokeFrame` reject it as malformed and the
     * desktop never learned the association was over.
     */
    @Test
    fun a_revoke_request_uses_the_pubkey_derived_id_not_the_bond_peer_field() {
        val bond = bondFor(FIXTURE_PEER_A)
        val appContext = RuntimeEnvironment.getApplication()
        BondStore(appContext.filesDir).save(bond)
        val derivedId = "0f1e2d3c4b5a69788796a5b4c3d2e1f0"
        SyauthCompanionService.peerIdComputer = PeerIdComputer { derivedId }

        val factory = RecordingGattClientFactory()
        SyauthCompanionService.bondListProvider = BondListProvider { listOf(bond) }
        SyauthCompanionService.gattClientFactory = factory

        val controller = Robolectric.buildService(SyauthCompanionService::class.java).create()
        val service = controller.get()
        service.onStartCommand(
            Intent(SyauthCompanionService.ACTION_REVOKE_BOND)
                .putExtra(SyauthCompanionService.EXTRA_PEER_ID, bond.peerId),
            0,
            0,
        )

        val client = factory.created.single().second
        val frame = client.sent.singleOrNull()
        assertNotNull("expected exactly one revoke frame, got ${client.sent.size}", frame)
        assertEquals(18, frame!!.size)
        assertEquals(OP_REVOKE, frame[17])
        assertArrayEquals(revokeFrame(derivedId), frame)
        controller.destroy()
    }

    /**
     * The revoke must survive the caller's own local cleanup.
     * `onBondRevokeTapped` hands the service a fire-and-forget
     * `startService` intent and then synchronously deletes the bond file, so by
     * the time the service reads it the file is always gone (observed
     * 2026-09-24: every in-app revoke logged "revoke requested without a
     * persisted bond record" and the desktop kept serving the bond). The record
     * the service already holds for the live client must be enough.
     */
    @Test
    fun a_revoke_still_sends_when_the_bond_file_was_already_deleted() {
        val bond = bondFor(FIXTURE_PEER_A)
        val appContext = RuntimeEnvironment.getApplication()
        val store = BondStore(appContext.filesDir)
        store.save(bond)
        val derivedId = "0f1e2d3c4b5a69788796a5b4c3d2e1f0"
        SyauthCompanionService.peerIdComputer = PeerIdComputer { derivedId }

        val factory = RecordingGattClientFactory()
        SyauthCompanionService.bondListProvider = BondListProvider { listOf(bond) }
        SyauthCompanionService.gattClientFactory = factory

        val controller = Robolectric.buildService(SyauthCompanionService::class.java).create()
        val service = controller.get()

        // The UI's own cleanup, landing before the fire-and-forget intent does.
        assertTrue("fixture bond file must exist", store.storePath.delete())

        service.onStartCommand(
            Intent(SyauthCompanionService.ACTION_REVOKE_BOND)
                .putExtra(SyauthCompanionService.EXTRA_PEER_ID, bond.peerId),
            0,
            0,
        )

        val client = factory.created.single().second
        val frame = client.sent.singleOrNull()
        assertNotNull("the revoke must not depend on the deleted file, got ${client.sent.size}", frame)
        assertEquals(OP_REVOKE, frame!![17])
        assertArrayEquals(revokeFrame(derivedId), frame)
        controller.destroy()
    }

    /**
     * The frame builder is the only place that turns a `peer_id` into wire
     * bytes: a malformed record must never put a bogus frame on the radio.
     */
    @Test
    fun a_malformed_peer_id_never_produces_a_frame() {
        assertEquals(null, revokeFrame(""))
        assertEquals(null, revokeFrame("not-hex"))
        assertEquals(null, revokeFrame("0123456789abcdef0123456789abcde"))
        assertNotNull(revokeFrame("0123456789abcdef0123456789abcdef"))
    }

    /**
     * Direction desktop → phone (2026-09-23: "sul cel ho ancora apk che dice
     * associato" after dissociating on the PC). The desktop sends the same
     * `Revoke` op the phone sends; the app must recognise it and drop its own
     * bond, so one dissociation is enough for both sides.
     */
    @Test
    fun a_desktop_revoke_frame_is_recognised_and_a_challenge_is_not() {
        val revoke = revokeFrame("0123456789abcdef0123456789abcdef")
        assertNotNull(revoke)
        assertTrue("the desktop's revoke op must be recognised", isRevokeFrame(revoke!!))

        // A challenge frame (the phone-pubkey write carries a different op) must
        // never be mistaken for a revocation.
        val challenge = byteArrayOf(TRANSACTION_WIRE_VERSION) +
            ByteArray(16) +
            byteArrayOf(1)
        assertFalse("a capability/challenge frame is not a revoke", isRevokeFrame(challenge))
        assertFalse("a truncated frame is not a revoke", isRevokeFrame(byteArrayOf(OP_REVOKE)))
    }

    /**
     * A revoke is bound to the bond identity: the 16 id bytes must name THIS
     * phone the way the desktop's bond store does (the pubkey-derived id),
     * otherwise the frame is stale, replayed or for another peer and must be
     * dropped without touching the local bond.
     */
    @Test
    fun a_desktop_revoke_drops_the_local_bond_only_when_the_id_matches() {
        val appContext = RuntimeEnvironment.getApplication()
        val bond = bondFor(FIXTURE_PEER_A)
        BondStore(appContext.filesDir).save(bond)
        val derivedId = "0f1e2d3c4b5a69788796a5b4c3d2e1f0"
        SyauthCompanionService.peerIdComputer = PeerIdComputer { derivedId }
        SyauthCompanionService.bondListProvider = BondListProvider { listOf(bond) }
        SyauthCompanionService.gattClientFactory = RecordingGattClientFactory()
        val controller = Robolectric.buildService(SyauthCompanionService::class.java).create()

        // A revoke frame naming THIS phone's derived id drops the bond file.
        SyauthCompanionService.handleIncomingFrame(
            appContext,
            bond.peerId,
            revokeFrame(derivedId)!!,
        )
        assertFalse(
            "the bond file must be gone after a matching desktop revoke",
            java.io.File(appContext.filesDir, BOND_RECORD_FILE_NAME).exists(),
        )

        // A frame naming some other peer never touches this phone's bond.
        BondStore(appContext.filesDir).save(bond)
        SyauthCompanionService.handleIncomingFrame(
            appContext,
            bond.peerId,
            revokeFrame("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")!!,
        )
        assertTrue(
            "a foreign revoke frame must not drop the local bond",
            java.io.File(appContext.filesDir, BOND_RECORD_FILE_NAME).exists(),
        )
        controller.destroy()
    }
}
