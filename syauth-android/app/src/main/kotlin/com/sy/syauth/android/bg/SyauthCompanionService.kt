// Roadmap item S-011 — long-running foreground `Service`.
//
// Before S-011 this class extended `CompanionDeviceService` so the OS
// bound it via the CDM proximity-observation callback. That model
// failed in field tests because CDM's `onDeviceAppeared` fires only on
// transitions; if the bonded peer was "already present" at boot, the
// service never started and `pam_syauth` saw `response-timeout` on
// every unlock. S-011 inverts the relationship: the service is a
// plain long-running `android.app.Service` that `MainActivity`
// explicitly starts via `startForegroundService` whenever a bond
// record exists. The service holds one `PersistentGattClient` per
// bonded peer (autoConnect=true) so the OS handles reconnection
// silently across range transitions, and surfaces a low-priority
// notification on the `syauth-presence` channel so the OS keeps the
// process alive across doze.
//
// S-013 collapsed the Android-side topology to a single path: the
// service holds one `PersistentGattClient` per bonded peer and the
// legacy CDM-style direct-controller extension point is gone.
package com.sy.syauth.android.bg

import android.bluetooth.BluetoothAdapter
import android.app.Notification
import android.app.ActivityOptions
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import com.sy.syauth.android.bond.BOND_RECORD_FILE_NAME
import android.os.IBinder
import android.media.RingtoneManager
import android.util.Log
import androidx.core.app.NotificationCompat
import com.sy.syauth.android.bond.BondRecord
import com.sy.syauth.android.bond.loadPersistedBond
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Log tag used by every span the service emits. Pinned constant
 * because the field-inspection workflow (AGENTS.md /bt skill, Phase 5)
 * greps logcat by tag.
 */
internal const val SYAUTH_BG_LOG_TAG: String = "syauth.bg"

/**
 * Notification channel id the foreground service uses. Pinned so
 * `adb shell dumpsys notification | grep syauth-presence` is one
 * grep away in field debugging.
 */
public const val NOTIFICATION_CHANNEL_ID: String = "syauth-presence"

/**
 * Human-readable channel name surfaced in `Settings → Apps → syauth
 * → Notifications`. The "active" suffix tells the operator the
 * notification is the "service is alive" chip, not an actionable
 * prompt.
 */
public const val NOTIFICATION_CHANNEL_NAME: String = "DeskUnlock active"

/**
 * Channel description shown under the name in the system UI. Tells
 * the operator that muting this channel is safe — the unlock prompts
 * use a separate, high-importance channel.
 */
public const val NOTIFICATION_CHANNEL_DESCRIPTION: String =
    "Background bridge that keeps the BLE link to your desktop alive."

/**
 * Stable notification id under which the foreground notification is
 * posted. Pinned int so `NotificationManager.cancel(NOTIFICATION_ID)`
 * works from any future caller without re-deriving it from a hash.
 */
public const val NOTIFICATION_ID: Int = 1001

/**
 * Foreground-service type. Pinned to `CONNECTED_DEVICE` because the
 * service exists exclusively to hold a BLE link to the bonded
 * desktop; declaring it accurately satisfies Android 14+'s
 * type-enforcement check (which throws
 * `SecurityException` otherwise at `startForeground` time).
 */
internal const val FOREGROUND_SERVICE_TYPE: Int =
    ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE

/**
 * Notification title. The operator can mute the channel from the
 * notification's long-press menu once they have seen the chip.
 */
internal const val NOTIFICATION_TITLE: String = "DeskUnlock active"

/**
 * Notification body. Short, plain text — no actionable affordances.
 */
internal const val NOTIFICATION_BODY: String =
    "Keeping the BLE link to your desktop alive."

/**
 * Provider that yields the in-service notification icon. Production
 * uses a stable system drawable; tests use the same. Held as a
 * compile-time constant so the resource lookup happens once at
 * build time.
 */
internal val NOTIFICATION_ICON: Int = android.R.drawable.ic_lock_lock

/**
 * Provider that resolves a bonded peer's hostname (for the
 * notification title). Tests inject a fixed mapping; production
 * wires a query against the bond store once that surface exists in
 * UniFFI (tracked as a follow-up; for S-018 the hostname falls
 * back to the association's `displayName`).
 */
public fun interface HostnameResolver {
    public fun hostnameFor(peerId: String): String
}

/**
 * Roadmap item S-015 — resolves a bonded peer's per-bond Keystore
 * alias for the Ed25519 private key minted at pair time by
 * `AndroidKeystoreKeyGenerator` (DEV-002). The activity needs the
 * alias on Approve so the production `AndroidBiometricGate` can
 * open the `PrivateKey` from the AndroidKeyStore and wrap it in a
 * `BiometricPrompt.CryptoObject` for the per-use sign. Tests inject
 * a fixed mapping; production wires this in
 * `MainActivity.installCompanionSeams`.
 */
public fun interface KeystoreAliasResolver {
    public fun keystoreAliasFor(peerId: String): String
}

/**
 * Provider that resolves a peer's bond key (the BLAKE3 keyed-hash
 * key used by UniFFI's `verifyChallengeFrame`). Production wires
 * this to the bond store; tests inject a fixed map.
 *
 * Returns `null` when the peer is unknown — the service then drops
 * the frame silently.
 */
public fun interface BondKeyProvider {
    public fun bondKeyFor(peerId: String): ByteArray?
}

/**
 * Adapter so the service can call into the Rust UniFFI surface
 * without statically importing `uniffi.syauth_mobile` (which
 * blows up the JVM unit-test classpath with `UnsatisfiedLinkError`
 * unless the AAR is present).
 *
 * Production wires `UniffiChallengeVerifier` (below); tests inject
 * a fake.
 */
public fun interface ChallengeVerifier {
    /**
     * Verify [frameBytes] under [bondKey]. Returns the verified
     * challenge payload bytes on success, or `null` on any verify
     * failure / malformed frame.
     */
    public fun verify(bondKey: ByteArray, frameBytes: ByteArray): ByteArray?
}

/**
 * Production [ChallengeVerifier] backed by UniFFI's
 * `verifyChallengeFrame(bondKey, frameBytes)`.
 */
public class UniffiChallengeVerifier : ChallengeVerifier {
    override fun verify(bondKey: ByteArray, frameBytes: ByteArray): ByteArray? =
        try {
            uniffi.syauth_mobile.verifyChallengeFrame(bondKey, frameBytes)
        } catch (t: Throwable) {
            null
        }
}

/**
 * Derives the desktop-side peer id (32 hex characters) for this phone's
 * Ed25519 public key, mirroring `syauth_core::peer_id_from_pubkey`. The
 * phone needs that identity to name itself in the `Revoke` frame — the
 * desktop's bond store keys bonds by it, not by the Bluetooth MAC the
 * `BondRecord.peerId` field carries (observed 2026-09-23: the app built
 * the frame from the MAC, `revokeFrame` rejected it as malformed, and
 * the desktop never learned the association was over).
 *
 * Production wires UniFFI's `peerIdFromPubkey`; tests inject a fixed
 * mapping so JVM tests never load the native AAR.
 */
public fun interface PeerIdComputer {
    public fun peerIdFor(pubkey: ByteArray): String?
}

/** UniFFI-backed production [PeerIdComputer]; never throws across the FFI. */
public fun defaultPeerIdComputer(): PeerIdComputer = PeerIdComputer { pubkey ->
    runCatching { uniffi.syauth_mobile.peerIdFromPubkey(pubkey) }.getOrNull()
}

/**
 * Opaque handle for a managed GATT client the service owns. The
 * service constructs one instance per bonded peer at `onCreate` and
 * calls `stop()` on every instance at `onDestroy`.
 *
 * Production binds this to [PersistentGattClient] via a closure in
 * the [GattClientFactory] returned by `MainActivity`'s installer.
 * Tests bind a recording fake.
 */
public interface ManagedClient {
    /** Open the underlying GATT link. Idempotent. */
    public fun start()

    /** Tear down the underlying GATT link. Idempotent. */
    public fun stop()

    /**
     * Send one raw control frame to the desktop, best effort.
     *
     * Returns `true` when the frame was handed to the radio. The default is a
     * no-op so test doubles do not have to implement a path they never use.
     */
    public fun send(frameBytes: ByteArray): Boolean = false
}

/** Wire version of the 18-byte transaction message (`[version][id:16][op]`). */
public const val TRANSACTION_WIRE_VERSION: Byte = 2

/** Day-2 revocation op: "the association is over". Mirrors `Operation::Revoke`. */
public const val OP_REVOKE: Byte = 14

/** Length of the fixed transaction message: version + 16-byte id + op. */
public const val TRANSACTION_MESSAGE_LENGTH: Int = 18

/**
 * `true` when the frame is the day-2 `Revoke` op: the desktop is telling this
 * phone that the association is over, so the app must drop its own bond
 * instead of the operator having to dissociate twice (2026-09-23).
 */
public fun isRevokeFrame(frameBytes: ByteArray): Boolean =
    frameBytes.size == TRANSACTION_MESSAGE_LENGTH &&
        frameBytes[0] == TRANSACTION_WIRE_VERSION &&
        frameBytes[17] == OP_REVOKE

/**
 * Build the frame that tells the desktop this phone dropped the association.
 *
 * The 16 bytes after the version carry the phone's `peer_id` (32 hex
 * characters) — a revocation is not part of any pairing transaction, so that
 * field is where the desktop reads *which* bond to revoke. Returns `null` when
 * the id is not a 32-character hex string, so a malformed record can never put
 * a bogus frame on the wire.
 */
public fun revokeFrame(peerId: String): ByteArray? {
    if (peerId.length != 32) return null
    val id = ByteArray(16)
    for (index in 0 until 16) {
        val value = peerId.substring(index * 2, index * 2 + 2).toIntOrNull(16) ?: return null
        id[index] = value.toByte()
    }
    return byteArrayOf(TRANSACTION_WIRE_VERSION) + id + byteArrayOf(OP_REVOKE)
}

/**
 * Provider that constructs a [ManagedClient] for one bonded peer.
 * Production: wraps `PersistentGattClient`. Tests: returns a
 * recording fake.
 */
public fun interface GattClientFactory {
    public fun create(bond: BondRecord): ManagedClient
}

/**
 * Production [ManagedClient] that delegates to a [PersistentGattClient].
 * Kept as a thin adapter so the foreground service can hold a generic
 * `ManagedClient` reference (which keeps the test seam clean) without
 * the production call site having to fabricate one inline.
 */
public class PersistentManagedClient(
    private val client: PersistentGattClient,
) : ManagedClient {
    override fun start() {
        client.start()
    }

    override fun stop() {
        client.stop()
    }

    override fun send(frameBytes: ByteArray): Boolean = client.writeResponse(frameBytes)
}

/**
 * Provider that yields the list of currently-bonded peers. Production
 * delegates to `loadPersistedBond(filesDir)` and wraps the single
 * record in a one-element list when present. Tests pre-seed an
 * arbitrary list. Returning an empty list means "no bonds; do not
 * inject any clients".
 */
public fun interface BondListProvider {
    public fun bonds(): List<BondRecord>
}

public class SyauthCompanionService : Service() {

    /**
     * Per-peer GATT client. Keyed by `BondRecord.peerId` so multi-bond
     * deployments scale without refactor.
     */
    private val clients: ConcurrentHashMap<String, ManagedClient> =
        ConcurrentHashMap()

    /**
     * The bond each live client was built from. A re-pair keeps the same
     * Bluetooth MAC (`BondRecord.peerId`) but mints a new bond key and phone
     * pubkey, so the MAC alone cannot tell a stale client from a live one.
     * Without this map the reconciliation sees the same key and keeps the
     * old client, which stays wedged against the desktop's torn-down GATT
     * service and never reconnects — proximity lock and phone unlock went
     * dead after a dissociate + re-associate (observed 2026-09-24).
     */
    private val clientBonds: ConcurrentHashMap<String, BondRecord> =
        ConcurrentHashMap()

    /**
     * The foreground type passed to the most recent `startForeground`
     * call. Robolectric 4.11.1's `ShadowService` does not expose
     * `getForegroundServiceType()`, so the test reads this field
     * directly. Package-internal — only the test friend reads it.
     */
    internal var lastForegroundType: Int = 0
        private set

    private val bluetoothStateReceiver = object : android.content.BroadcastReceiver() {
        override fun onReceive(context: android.content.Context?, intent: Intent?) {
            if (intent?.action != BluetoothAdapter.ACTION_STATE_CHANGED) return

            val state = intent.getIntExtra(
                BluetoothAdapter.EXTRA_STATE,
                BluetoothAdapter.ERROR,
            )

            Log.i(SYAUTH_BG_LOG_TAG, "bluetooth state changed state=$state")

            if (state == BluetoothAdapter.STATE_ON) {
                val provider = bondListProvider ?: defaultBondListProvider()

                for (bond in provider.bonds()) {
                    val client = PersistentGattClientRegistry.lookup(bond.peerId)
                    if (client != null) {
                        Log.i(
                            SYAUTH_BG_LOG_TAG,
                            "bluetooth STATE_ON: forceReconnect",
                        )
                        runCatching { client.forceReconnect() }
                            .onFailure {
                                Log.w(
                                    SYAUTH_BG_LOG_TAG,
                                    "bluetooth STATE_ON reconnect failed",
                                    it,
                                )
                            }
                    }
                }
            }
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ensureNotificationChannel()
        val notification = buildForegroundNotification()
        startForegroundCompat(notification)
        ensureDefaultGattClientFactory()
        ensureDefaultCompanionSeams()
        injectClientsForBonds()

        val btFilter = android.content.IntentFilter(
            BluetoothAdapter.ACTION_STATE_CHANGED
        )

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(
                bluetoothStateReceiver,
                btFilter,
                android.content.Context.RECEIVER_NOT_EXPORTED,
            )
        } else {
            @Suppress("DEPRECATION")
            registerReceiver(bluetoothStateReceiver, btFilter)
        }

        isRunning.set(true)
        Log.i(SYAUTH_BG_LOG_TAG, "onCreate: foreground up, clients=${clients.size}")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // The service is "sticky" — if the OS kills the process, the
        // system tries to recreate it. `onCreate` will re-run the
        // bond-injection path on every recreate.
        //
        // `ACTION_RELOAD_BONDS` is the day-2 path: after the operator pairs
        // or dissociates in the app UI the bond set changes on disk while
        // this service keeps running. Without a reload the service kept
        // serving the *previous* bond, so the desktop saw `Connected: no`,
        // presence samples stopped arriving and both the proximity lock and
        // the phone unlock went dead until the app was restarted by hand
        // (observed 2026-09-23).
        if (intent?.action == ACTION_RELOAD_BONDS) {
            Log.i(SYAUTH_BG_LOG_TAG, "onStartCommand: reloading clients for the current bonds")
            injectClientsForBonds()
        }
        if (intent?.action == ACTION_REVOKE_BOND) {
            sendRevoke(intent.getStringExtra(EXTRA_PEER_ID))
        }
        return START_STICKY
    }

    /**
     * Send one `Revoke` frame so the desktop drops this bond.
     *
     * Best effort and deliberately narrow: the frame goes out over the live
     * GATT link the service already owns, and nothing else changes here. The
     * app UI owns the local cleanup (bond file, keystore entry, reload) so
     * there is exactly one owner for it.
     */
    private fun sendRevoke(peerId: String?) {
        val clientKey = peerId ?: clients.keys.firstOrNull()
        val client = clientKey?.let { clients[it] }
        if (clientKey == null || client == null) {
            Log.w(SYAUTH_BG_LOG_TAG, "revoke requested with no client to send it on")
            return
        }
        val record = runCatching { loadPersistedBond(filesDir) }.getOrNull()
        if (record == null) {
            Log.w(SYAUTH_BG_LOG_TAG, "revoke requested without a persisted bond record")
            return
        }
        // The frame must carry the id the desktop's bond store actually
        // uses — the pubkey-derived 32-hex id — not the MAC in
        // `BondRecord.peerId` (which names the GATT link only).
        val ownId = (peerIdComputer ?: defaultPeerIdComputer()).peerIdFor(record.phonePubkey)
        if (ownId == null || ownId.length != 32) {
            Log.w(SYAUTH_BG_LOG_TAG, "revoke requested but the own peer id cannot be derived")
            return
        }
        val frame = revokeFrame(ownId)
        if (frame == null) {
            Log.w(SYAUTH_BG_LOG_TAG, "revoke requested with a malformed derived peer id")
            return
        }
        val sent = client.send(frame)
        Log.i(SYAUTH_BG_LOG_TAG, "revoke: frame handed to the radio sent=$sent peer=$clientKey")
    }

    override fun onDestroy() {
        runCatching { unregisterReceiver(bluetoothStateReceiver) }

        for ((_, client) in clients) {
            runCatching { client.stop() }
                .onFailure {
                    Log.w(SYAUTH_BG_LOG_TAG, "onDestroy: client.stop failed", it)
                }
        }
        clients.clear()
        clientBonds.clear()
        isRunning.set(false)
        Log.i(SYAUTH_BG_LOG_TAG, "onDestroy: clients torn down")
        super.onDestroy()
    }

    private fun ensureNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            NOTIFICATION_CHANNEL_ID,
            NOTIFICATION_CHANNEL_NAME,
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = NOTIFICATION_CHANNEL_DESCRIPTION
            setShowBadge(false)
        }
        // Re-registering an existing channel updates its user-visible
        // name and description while preserving the stable channel ID
        // and the user's notification preferences.
        manager.createNotificationChannel(channel)
        if (channelCreatedLogged.compareAndSet(false, true)) {
            Log.i(SYAUTH_BG_LOG_TAG, "channel registered id=$NOTIFICATION_CHANNEL_ID")
        }
    }

    private fun buildForegroundNotification(): Notification =
        NotificationCompat.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setContentTitle(NOTIFICATION_TITLE)
            .setContentText(NOTIFICATION_BODY)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setOngoing(true)
            .setSmallIcon(NOTIFICATION_ICON)
            .build()

    private fun startForegroundCompat(notification: Notification) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, FOREGROUND_SERVICE_TYPE)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        lastForegroundType = FOREGROUND_SERVICE_TYPE
    }

    private fun injectClientsForBonds() {
        val factory = gattClientFactory ?: return
        val provider = bondListProvider ?: defaultBondListProvider()
        val desired = provider.bonds().associateBy { it.peerId }

        // Drop a client whose bond is gone, or whose bond changed under the
        // same MAC. A re-pair mints a new bond key, so the old GATT link is
        // dead even though `peerId` is unchanged; keeping it left proximity
        // and phone unlock dead until the app was restarted by hand.
        for (peerId in clients.keys.toList()) {
            val bond = desired[peerId]
            val built = clientBonds[peerId]
            if (bond != null && built == bond) continue
            val client = clients.remove(peerId) ?: continue
            clientBonds.remove(peerId)
            runCatching { client.stop() }
                .onFailure { Log.w(SYAUTH_BG_LOG_TAG, "stale client.stop failed peer=$peerId", it) }
            Log.i(
                SYAUTH_BG_LOG_TAG,
                if (bond == null) {
                    "dropped client for a bond that is gone peer=$peerId"
                } else {
                    "rebuilt client for a re-paired bond peer=$peerId"
                },
            )
        }

        // Start a client only for bonds that do not have one yet: a reload
        // must never churn the connection that already works.
        for ((peerId, bond) in desired) {
            if (clients.containsKey(peerId)) continue
            val client = factory.create(bond)
            clients[peerId] = client
            clientBonds[peerId] = bond
            runCatching { client.start() }
                .onFailure {
                    Log.w(SYAUTH_BG_LOG_TAG, "client.start failed", it)
                }
        }
    }

    /**
     * BUG-20260522-0130: when Android kills the app process under
     * memory pressure and `START_STICKY` resurrects the service, the
     * companion-object [gattClientFactory] is reset to `null` because
     * the entire JVM was torn down. `MainActivity` is the only place
     * the production factory was previously installed, so a service
     * resurrected without the UI being reopened would sit alive with
     * `clients = []` forever — every challenge from the desktop
     * instant-failed `transport-error`. This installer runs inside
     * `onCreate` so a process-restarted service is self-sufficient.
     *
     * Mirrors `MainActivity.installPersistentClientFactory` field-by-
     * field; the activity's installer remains as a no-op when the
     * default is already in place (the `gattClientFactory != null`
     * guard there short-circuits cleanly).
     */
    private fun ensureDefaultGattClientFactory() {
        if (gattClientFactory != null) return
        val adapter = runCatching {
            getSystemService(BluetoothManager::class.java)?.adapter
        }.getOrNull()
        if (adapter == null) {
            Log.w(
                SYAUTH_BG_LOG_TAG,
                "ensureDefaultGattClientFactory: no BluetoothAdapter; clients will stay empty until MainActivity installs a factory",
            )
            return
        }
        val appContext = applicationContext
        SyauthCompanionService.gattClientFactory = GattClientFactory { bond ->
            val client = PersistentGattClient(
                context = appContext,
                adapter = adapter,
                peerId = bond.peerId,
                deviceMac = bond.peerId,
                onChallenge = { peerId, frameBytes ->
                    handleIncomingFrame(appContext, peerId, frameBytes)
                },
            )
            PersistentGattClientRegistry.put(bond.peerId, client)
            PersistentManagedClient(client)
        }
        Log.i(
            SYAUTH_BG_LOG_TAG,
            "ensureDefaultGattClientFactory: installed (MainActivity had not yet)",
        )
    }

    /**
     * BUG-20260522-0138 (extension): the original fix installed a
     * default `gattClientFactory` so the persistent BLE link came up
     * after a `START_STICKY` resurrect. That unblocked transport but
     * the **approval path** then failed with `alias=''` because
     * `keystoreAliasResolver`, `hostnameResolver`, `bondKeyProvider`,
     * `challengeVerifier`, and the activity-level
     * [ChallengeApprovalActivity.responseSink] /
     * [ChallengeApprovalActivity.cancelSink] are also JVM-static
     * seams owned by `MainActivity.installCompanionSeams`. Each gets
     * wiped by the process-restart and the user observes "Approve
     * tap closes the app without biometric → desktop PAM falls back
     * to FIDO2."
     *
     * This helper mirrors `MainActivity.installCompanionSeams`
     * load-bearing assignments using the persisted bond record as the
     * source of truth. It does **not** install
     * [ChallengeApprovalActivity.historyDispatcher] — history
     * notifications are UI-tier and the responseSink default
     * dispatches a `null`-safe history payload when no dispatcher is
     * present (the post-restart user gets unlock back; the history
     * surface re-attaches when they next open the app).
     */
    private fun ensureDefaultCompanionSeams() {
        val appContext = applicationContext
        val recordSupplier: () -> BondRecord? = {
            runCatching { loadPersistedBond(appContext.filesDir) }.getOrNull()
        }
        if (bondKeyProvider == null) {
            bondKeyProvider = BondKeyProvider { peerId ->
                recordSupplier()?.takeIf { it.peerId == peerId }?.bondKey
            }
        }
        if (hostnameResolver == null) {
            hostnameResolver = HostnameResolver { peerId ->
                recordSupplier()?.takeIf { it.peerId == peerId }?.hostName ?: peerId
            }
        }
        if (keystoreAliasResolver == null) {
            keystoreAliasResolver = KeystoreAliasResolver { peerId ->
                recordSupplier()?.takeIf { it.peerId == peerId }?.keystoreAlias.orEmpty()
            }
        }
        if (challengeVerifier == null) {
            challengeVerifier = UniffiChallengeVerifier()
        }
        if (ChallengeApprovalActivity.responseSink == null) {
            ChallengeApprovalActivity.responseSink = ResponseSink { peerId, responseBytes ->
                val client = PersistentGattClientRegistry.lookup(peerId)
                if (client == null) {
                    Log.w(SYAUTH_BG_LOG_TAG, "approve: no persistent client")
                } else {
                    runCatching { client.writeResponse(responseBytes) }
                        .onFailure {
                            Log.w(SYAUTH_BG_LOG_TAG, "approve: writeResponse failed", it)
                        }
                }
            }
        }
        if (ChallengeApprovalActivity.cancelSink == null) {
            ChallengeApprovalActivity.cancelSink = CancelSink { peerId, deniedFrameBytes ->
                val client = PersistentGattClientRegistry.lookup(peerId)
                if (client == null) {
                    Log.w(SYAUTH_BG_LOG_TAG, "cancel: no persistent client")
                } else {
                    runCatching { client.writeResponse(deniedFrameBytes) }
                        .onFailure {
                            Log.w(SYAUTH_BG_LOG_TAG, "cancel: writeResponse failed", it)
                        }
                }
            }
        }
        Log.i(
            SYAUTH_BG_LOG_TAG,
            "ensureDefaultCompanionSeams: installed missing seams (MainActivity had not yet)",
        )
    }

    private fun defaultBondListProvider(): BondListProvider = BondListProvider {
        val record = runCatching { loadPersistedBond(filesDir) }.getOrNull()
        if (record == null) emptyList() else listOf(record)
    }

    public companion object {
        internal const val LOG_TAG: String = SYAUTH_BG_LOG_TAG

        /**
         * Day-2 reload: the bond set changed on disk while this service was
         * already running (a new pairing, or a dissociation).
         *
         * `injectClientsForBonds()` runs in `onCreate`, and the service is
         * `START_STICKY`, so without this action it kept the clients it built
         * for the previous bond: the desktop stayed `Connected: no`, presence
         * samples stopped and proximity lock plus phone unlock both went dead
         * until the app was restarted by hand (observed 2026-09-23).
         */
        public const val ACTION_RELOAD_BONDS: String = "com.sy.syauth.android.action.RELOAD_BONDS"

        /**
         * Day-2 revocation from the app UI: send one `Revoke` frame to the
         * desktop so it stops serving this bond.
         *
         * Without it, dissociating in the app left the desktop advertising a
         * peer the phone had already forgotten (observed 2026-09-23: "if I
         * dissociate in the app the PC stays active").
         */
        public const val ACTION_REVOKE_BOND: String = "com.sy.syauth.android.action.REVOKE_BOND"

        /** Extra carrying the `peer_id` to revoke. */
        public const val EXTRA_PEER_ID: String = "com.sy.syauth.android.extra.PEER_ID"

        /**
         * Broadcast sent after a desktop-initiated revoke dropped the local
         * bond, so the UI can stop showing "associated" without a manual
         * refresh.
         */
        public const val ACTION_BOND_DROPPED: String = "com.sy.syauth.android.action.BOND_DROPPED"

        /**
         * Latches the "channel created" log so it appears at most once
         * per process lifetime — first creation logs, every subsequent
         * `ensureNotificationChannel` call no-ops silently.
         */
        private val channelCreatedLogged: AtomicBoolean = AtomicBoolean(false)

        /**
         * Process-local lifecycle flag the S-012 resurrection helper
         * consults. `true` while `onCreate` has run and `onDestroy`
         * has not; `false` otherwise (cold-start default, post-destroy).
         * The flag survives only inside the app process — a separate
         * `WORK_PROCESS` worker would read `false` here even when the
         * service is alive elsewhere, which is acceptable because
         * `startForegroundService` is idempotent at the OS layer.
         */
        public val isRunning: AtomicBoolean = AtomicBoolean(false)

        /** Bond-key provider seam; see [BondKeyProvider]. */
        @Volatile
        public var bondKeyProvider: BondKeyProvider? = null

        /** Peer-id derivation seam; see [PeerIdComputer]. */
        @Volatile
        public var peerIdComputer: PeerIdComputer? = null

        /** Hostname resolver seam; see [HostnameResolver]. */
        @Volatile
        public var hostnameResolver: HostnameResolver? = null

        /**
         * Keystore-alias resolver seam (S-015); see
         * [KeystoreAliasResolver]. Production wires this from
         * `MainActivity.installCompanionSeams` to the bond record's
         * `keystoreAlias`. Tests inject a fixed map. When `null` the
         * alias extra is empty and the activity falls through to a
         * denied frame on Approve (the OS Keystore would reject the
         * `getKey(null, ...)` anyway).
         */
        @Volatile
        public var keystoreAliasResolver: KeystoreAliasResolver? = null

        /** Challenge verifier seam; see [ChallengeVerifier]. */
        @Volatile
        public var challengeVerifier: ChallengeVerifier? = null

        /**
         * Persistent-client factory. Production sets this from
         * `MainActivity.onCreate`; tests inject a recording fake.
         */
        @Volatile
        public var gattClientFactory: GattClientFactory? = null

        /**
         * Bond-list provider seam. Production leaves it `null`, in
         * which case the service falls back to
         * `loadPersistedBond(filesDir)`; tests inject a fixed list.
         */
        @Volatile
        public var bondListProvider: BondListProvider? = null

        /**
         * Reset all seams to `null`. Used by Robolectric tests to keep
         * state clean between cases.
         */
        public fun resetSeams() {
            bondKeyProvider = null
            hostnameResolver = null
            keystoreAliasResolver = null
            challengeVerifier = null
            gattClientFactory = null
            bondListProvider = null
            peerIdComputer = null
        }

        /**
         * Single entry point for every frame the desktop notifies on the
         * challenge characteristic. Both factories (the service's default
         * one and MainActivity's) must route here so a desktop-side revoke
         * is handled identically no matter who installed the factory
         * (observed 2026-09-23: only the default factory recognised the
         * frame; opening the app swapped in a factory that didn't).
         */
        public fun handleIncomingFrame(context: Context, peerId: String, frameBytes: ByteArray) {
            if (!isRevokeFrame(frameBytes)) {
                // Strip the trailing 16-byte MAC tag so the signature is
                // computed over the frame body only (version || nonce ||
                // payload), matching the daemon's verify_frame contract.
                val challengeBody = if (frameBytes.size > 16) {
                    frameBytes.copyOfRange(0, frameBytes.size - 16)
                } else {
                    frameBytes
                }
                launchApprovalActivity(context, peerId, challengeBody)
                return
            }
            // A revoke must be bound to the current bond: the 16 id bytes
            // after the version must name THIS phone the way the desktop's
            // bond store does (the pubkey-derived id). Anything else is a
            // stale, replayed or wrong-peer frame and is dropped.
            val record = runCatching { loadPersistedBond(context.filesDir) }.getOrNull()
            val ownId = record?.let { (peerIdComputer ?: defaultPeerIdComputer()).peerIdFor(it.phonePubkey) }
            val frameId = hexOf(frameBytes, 1, 17)
            if (record != null && ownId != null && ownId.equals(frameId, ignoreCase = true)) {
                dropLocalBond(context, record)
            } else {
                Log.w(SYAUTH_BG_LOG_TAG, "revoke frame rejected: the peer id does not match this phone's bond")
            }
        }

        /**
         * Delete the local bond because the desktop said the association is
         * over, then reconcile clients and tell the UI.
         */
        public fun dropLocalBond(context: Context, record: BondRecord) {
            Log.i(SYAUTH_BG_LOG_TAG, "desktop revoked the association: dropping the local bond peer=${record.peerId}")
            runCatching { java.io.File(context.filesDir, BOND_RECORD_FILE_NAME).delete() }
                .onFailure { Log.w(SYAUTH_BG_LOG_TAG, "revoke: bond file delete failed", it) }
            runCatching {
                val ks = java.security.KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
                if (ks.containsAlias(record.keystoreAlias)) ks.deleteEntry(record.keystoreAlias)
            }.onFailure { Log.w(SYAUTH_BG_LOG_TAG, "revoke: keystore entry delete failed", it) }
            runCatching {
                context.startService(
                    Intent(context, SyauthCompanionService::class.java)
                        .setAction(ACTION_RELOAD_BONDS),
                )
            }.onFailure { Log.w(SYAUTH_BG_LOG_TAG, "revoke: reload dispatch failed", it) }
            runCatching {
                context.sendBroadcast(
                    Intent(ACTION_BOND_DROPPED).putExtra(EXTRA_PEER_ID, record.peerId),
                )
            }.onFailure { Log.w(SYAUTH_BG_LOG_TAG, "revoke: dropped-broadcast failed", it) }
        }

        /**
         * Roadmap item S-014 — launch [ChallengeApprovalActivity] for a
         * fresh challenge frame the [PersistentGattClient.onChallenge]
         * callback delivered.
         *
         * The hostname comes from the installed [hostnameResolver]
         * (which `MainActivity.installCompanionSeams` wires to the
         * bond record's `hostName`). If no resolver is installed the
         * peer id is used as the displayed hostname so the prompt
         * still renders something the user can recognise — see SPEC
         * §9 Q2 for the prompt copy contract.
         *
         * The intent is dispatched via [PendingIntent.getActivity]
         * with `FLAG_IMMUTABLE` (Android 12+ floor) so the OS treats
         * the launch as foreground-equivalent and the activity's
         * `showWhenLocked` / `turnScreenOn` manifest attributes wake
         * the screen over the keyguard.
         */
        public fun launchApprovalActivity(
            context: Context,
            peerId: String,
            challengeBytes: ByteArray,
        ) {
            val hostname = hostnameResolver?.hostnameFor(peerId) ?: peerId
            val keystoreAlias =
                keystoreAliasResolver?.keystoreAliasFor(peerId).orEmpty()

            val intent = buildApprovalIntent(
                context = context,
                peerId = peerId,
                hostname = hostname,
                challengeBytes = challengeBytes,
                keystoreAlias = keystoreAlias,
            )

            val pending = PendingIntent.getActivity(
                context,
                APPROVAL_PENDING_REQUEST_CODE,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or
                    PendingIntent.FLAG_IMMUTABLE,
            )

            runCatching {
                val soundUri =
                    RingtoneManager.getDefaultUri(
                        RingtoneManager.TYPE_NOTIFICATION,
                    )

                RingtoneManager
                    .getRingtone(context, soundUri)
                    ?.play()
            }.onFailure {
                Log.w(
                    SYAUTH_BG_LOG_TAG,
                    "approval sound failed peer=$peerId",
                    it,
                )
            }

            runCatching {
                if (Build.VERSION.SDK_INT >= 34) {
                    val options = ActivityOptions.makeBasic().apply {
                        setPendingIntentBackgroundActivityStartMode(
                            if (Build.VERSION.SDK_INT >= 36) {
                                3 // MODE_BACKGROUND_ACTIVITY_START_ALLOW_ALWAYS
                            } else {
                                ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED
                            },
                        )
                    }

                    pending.send(
                        context,
                        0,
                        null,
                        null,
                        null,
                        null,
                        options.toBundle(),
                    )
                } else {
                    pending.send()
                }

                Log.i(
                    SYAUTH_BG_LOG_TAG,
                    "approval activity dispatched",
                )
            }.onFailure {
                Log.e(
                    SYAUTH_BG_LOG_TAG,
                    "approval activity dispatch failed",
                    it,
                )
            }
        }
    }
}

/**
 * 16 bytes at [from..to) as a lowercase hex string. Used to compare the
 * id inside a revoke frame against the phone's own derived id.
 */
private fun hexOf(bytes: ByteArray, from: Int, to: Int): String =
    bytes.copyOfRange(from, to).joinToString("") { "%02x".format(it.toInt() and 0xFF) }

/**
 * Roadmap item S-014 — request code passed to
 * `PendingIntent.getActivity`. Pinned constant so a future caller
 * does not collide on the same request code by accident.
 */
internal const val APPROVAL_PENDING_REQUEST_CODE: Int = 0x5A14
