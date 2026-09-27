// DEV-001 (re-march): production wrappers around the Android
// Bluetooth platform surface used by [RealPairBackend].
//
// Every wrapper here implements one of the [RealPairBackend]
// constructor's seam interfaces with a real Android dependency. The
// Robolectric tests do NOT touch this file — they inject hand-rolled
// fakes directly into [RealPairBackend].
@file:Suppress("MissingPermission", "NewApi")

package com.sy.syauth.android.pair.impl

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.content.BroadcastReceiver
import android.content.Context
import android.content.IntentFilter
import android.util.Log
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import uniffi.syauth_mobile.sessionUuidForBond

/** Log tag for the raw GATT exchange outcomes (pairing diagnostics). */
private const val PAIR_GATT_LOG_TAG: String = "syauth.pair.gatt"

/**
 * Fixed UUID of the desktop's transient pair service. Byte-identical
 * to the Rust constant `SYAUTH_PAIR_SERVICE_UUID` in
 * `crates/syauth-transport/src/bluez.rs`.
 */
public val SYAUTH_PAIR_SERVICE_UUID: UUID = UUID.fromString("5a4e8e3c-1c4c-4a17-9c81-d518a55a0101")

/**
 * Characteristic holding the desktop's 32-byte Ed25519 host pubkey.
 * Byte-identical to the Rust constant
 * `SYAUTH_PAIR_HOST_PUBKEY_CHAR_UUID`.
 */
public val SYAUTH_PAIR_HOST_PUBKEY_CHAR_UUID: UUID = UUID.fromString("5a4e8e3c-1c4c-4a17-9c81-d518a55a0102")

/**
 * Characteristic the phone writes its 32-byte Ed25519 pubkey to.
 * Byte-identical to the Rust constant
 * `SYAUTH_PAIR_PHONE_PUBKEY_CHAR_UUID`.
 */
public val SYAUTH_PAIR_PHONE_PUBKEY_CHAR_UUID: UUID = UUID.fromString("5a4e8e3c-1c4c-4a17-9c81-d518a55a0103")

/** Optional, authenticated display metadata. This is NOT commit capability. */
public val SYAUTH_PAIR_HOST_NAME_V1_CHAR_UUID: UUID = UUID.fromString("5a4e8e3c-1c4c-4a17-9c81-d518a55a0104")
public val SYAUTH_PAIR_V2_CONTROL_CHAR_UUID: UUID = UUID.fromString("5a4e8e3c-1c4c-4a17-9c81-d518a55a0105")
public val SYAUTH_PAIR_V2_STATUS_CHAR_UUID: UUID = UUID.fromString("5a4e8e3c-1c4c-4a17-9c81-d518a55a0106")

/** Reject unknown versions, malformed UTF-8 and control-character injection. */
internal fun decodePeerDisplayName(bytes: ByteArray): String {
    require(bytes.size in 2..129 && bytes[0] == 1.toByte()) { "Unsupported peer name metadata" }
    val name = Charsets.UTF_8.newDecoder()
        .decode(java.nio.ByteBuffer.wrap(bytes, 1, bytes.size - 1)).toString()
    require(name.isNotBlank() && name.none { it.isISOControl() }) { "Invalid peer name metadata" }
    return name
}

/** Default GATT-exchange wait window for service discovery + char read/write. */
public const val PAIR_GATT_EXCHANGE_TIMEOUT_SECS: Long = 30L

/**
 * Production [ReceiverRegistrar] wrapping
 * `Context.registerReceiver` / `Context.unregisterReceiver`.
 */
public class ContextReceiverRegistrar(private val context: Context) : ReceiverRegistrar {
    override fun register(receiver: BroadcastReceiver, filter: IntentFilter) {
        context.registerReceiver(receiver, filter)
    }
    override fun unregister(receiver: BroadcastReceiver) {
        context.unregisterReceiver(receiver)
    }
}

/**
 * Production [PairClock] wrapping the system wall-clock.
 */
public class SystemPairClock : PairClock {
    override fun nowEpochSeconds(): Long = System.currentTimeMillis() / MILLIS_PER_SECOND

    private companion object {
        const val MILLIS_PER_SECOND: Long = 1_000L
    }
}

/**
 * Production [PairSessionUuidLookup] delegating to UniFFI's
 * `sessionUuidForBond` — byte-identical to the Rust
 * `syauth_transport::session_uuid_for`.
 */
public class UniffiPairSessionUuidLookup : PairSessionUuidLookup {
    override fun lookup(bondKey: ByteArray, minute: Long): ByteArray =
        sessionUuidForBond(bondKey, minute)
}

/**
 * Production [PairBondKeyDeriver]. Mirrors
 * `syauth_core::bond_key_from_pubkeys` byte-for-byte:
 *
 *   bond_key = HKDF-SHA256(salt=None,
 *                           ikm = host_pubkey || phone_pubkey,
 *                           info = "syauth-bond-v1")[0..32]
 *
 * Implemented in pure JVM crypto so the runtime does not need to
 * cross the UniFFI boundary for this single derivation.
 */
public class HkdfPairBondKeyDeriver : PairBondKeyDeriver {

    override fun derive(hostPubkey: ByteArray, phonePubkey: ByteArray): ByteArray {
        val ikm = ByteArray(hostPubkey.size + phonePubkey.size)
        System.arraycopy(hostPubkey, 0, ikm, 0, hostPubkey.size)
        System.arraycopy(phonePubkey, 0, ikm, hostPubkey.size, phonePubkey.size)
        return hkdfSha256(ikm = ikm, info = BOND_HKDF_INFO_V1, length = PAIR_BOND_KEY_LEN)
    }

    private fun hkdfSha256(ikm: ByteArray, info: ByteArray, length: Int): ByteArray {
        val mac = javax.crypto.Mac.getInstance(HMAC_ALG)
        val zeroSalt = ByteArray(MessageDigest.getInstance(SHA256_ALG).digestLength)
        mac.init(javax.crypto.spec.SecretKeySpec(zeroSalt, HMAC_ALG))
        val prk = mac.doFinal(ikm)
        // expand: T(1) = HMAC(prk, info || 0x01); concat until length.
        val out = ByteArray(length)
        var written = 0
        var prev = ByteArray(0)
        var counter = 1
        while (written < length) {
            mac.init(javax.crypto.spec.SecretKeySpec(prk, HMAC_ALG))
            mac.update(prev)
            mac.update(info)
            mac.update(counter.toByte())
            prev = mac.doFinal()
            val take = minOf(prev.size, length - written)
            System.arraycopy(prev, 0, out, written, take)
            written += take
            counter += 1
        }
        return out
    }

    private companion object {
        const val HMAC_ALG: String = "HmacSHA256"
        const val SHA256_ALG: String = "SHA-256"
        val BOND_HKDF_INFO_V1: ByteArray = "syauth-bond-v1".toByteArray(Charsets.US_ASCII)
    }
}

/**
 * Production [PairGattExchange] wrapping
 * `BluetoothDevice.connectGatt(...)`. Used by [RealPairBackend] after
 * `BOND_BONDED` lands; opens a fresh GATT client connection to the
 * (now bonded) device, discovers the pair service, writes the phone
 * pubkey to the `phone-pubkey` characteristic, reads the
 * `host-pubkey` characteristic, and returns the 32 bytes.
 *
 * The implementation uses blocking `CountDownLatch`-driven callbacks
 * because the [PairBackend] surface is synchronous. The backend
 * already calls [PairGattExchange.exchangePubkeys] from a coroutine
 * pumped via `awaitLescResult` -> `runBlocking`, so the blocking is
 * intentional and bounded by [PAIR_GATT_EXCHANGE_TIMEOUT_SECS].
 */
public class AndroidPairGattExchange(
    private val context: Context,
    private val adapter: BluetoothAdapter,
) : PairGattExchange {
    @Volatile private var activeGatt: BluetoothGatt? = null
    @Volatile private var controlCharacteristic: BluetoothGattCharacteristic? = null
    @Volatile private var statusCharacteristic: BluetoothGattCharacteristic? = null
    private val statusValue = AtomicReference<ByteArray?>(null)
    @Volatile private var statusReadLatch: CountDownLatch? = null
    @Volatile private var controlWriteLatch: CountDownLatch? = null
    @Volatile private var controlWriteStatus: Int = BluetoothGatt.GATT_FAILURE

    override fun writeTransactionMessage(message: ByteArray): Boolean {
        val gatt = activeGatt ?: return false
        val characteristic = controlCharacteristic ?: return false
        if (message.size != 18 && message.size != 27) return false
        val latch = CountDownLatch(1)
        controlWriteLatch = latch
        characteristic.value = message.copyOf()
        val started = runCatching { gatt.writeCharacteristic(characteristic) }.getOrDefault(false)
        if (!started || !latch.await(PAIR_GATT_EXCHANGE_TIMEOUT_SECS, TimeUnit.SECONDS)) {
            controlWriteLatch = null
            Log.w(PAIR_GATT_LOG_TAG, "v2 write op=${message[17]} refused started=$started")
            return false
        }
        controlWriteLatch = null
        Log.i(PAIR_GATT_LOG_TAG, "v2 write op=${message[17]} status=$controlWriteStatus")
        return controlWriteStatus == BluetoothGatt.GATT_SUCCESS
    }

    override fun readTransactionMessage(): ByteArray? {
        val gatt = activeGatt ?: return null
        val characteristic = statusCharacteristic ?: return null
        statusValue.set(null)
        val latch = CountDownLatch(1)
        statusReadLatch = latch
        if (!runCatching { gatt.readCharacteristic(characteristic) }.getOrDefault(false)) return null
        if (!latch.await(PAIR_GATT_EXCHANGE_TIMEOUT_SECS, TimeUnit.SECONDS)) return null
        return statusValue.get()?.copyOf()
    }

    override fun reconnectStatus(address: String): Boolean {
        closeSession()
        val done = CountDownLatch(1)
        val success = AtomicReference(false)
        val device = runCatching { adapter.getRemoteDevice(address) }.getOrNull() ?: return false
        val callback = object : BluetoothGattCallback() {
            override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                if (newState == BluetoothGatt.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
                    runCatching { gatt.discoverServices() }.onFailure { done.countDown() }
                } else if (newState == BluetoothGatt.STATE_DISCONNECTED) {
                    done.countDown()
                }
            }
            override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                val service = if (status == BluetoothGatt.GATT_SUCCESS) gatt.getService(SYAUTH_PAIR_SERVICE_UUID) else null
                val control = service?.getCharacteristic(SYAUTH_PAIR_V2_CONTROL_CHAR_UUID)
                val state = service?.getCharacteristic(SYAUTH_PAIR_V2_STATUS_CHAR_UUID)
                if (control != null && state != null) {
                    activeGatt = gatt
                    controlCharacteristic = control
                    statusCharacteristic = state
                    success.set(true)
                }
                done.countDown()
            }
        }
        val gatt = runCatching { device.connectGatt(context, false, callback) }.getOrNull() ?: return false
        if (!done.await(PAIR_GATT_EXCHANGE_TIMEOUT_SECS, TimeUnit.SECONDS) || !success.get()) {
            runCatching { gatt.disconnect() }
            runCatching { gatt.close() }
            return false
        }
        return true
    }

    override fun closeSession() {
        val gatt = activeGatt ?: return
        runCatching { gatt.disconnect() }
        runCatching { gatt.close() }
        activeGatt = null
        controlCharacteristic = null
        statusCharacteristic = null
    }

    @Volatile private var hostDisplayName: String? = null

    override fun peerDisplayName(): String? = hostDisplayName

    /**
     * Reflective `BluetoothGatt.refresh()` — clears the OS per-device GATT
     * cache. `@hide` on AOSP but stable; mirrors the unlock client's helper.
     * Never throws: a failed refresh just means the next discovery may use
     * cached data.
     */
    private fun refreshGattCache(gatt: BluetoothGatt): Boolean = runCatching {
        gatt.javaClass.getMethod("refresh").invoke(gatt) as? Boolean ?: false
    }.getOrDefault(false)

    override fun exchangePubkeys(address: String, phonePubkey: ByteArray): ByteArray {
        hostDisplayName = null
        val nameDone = CountDownLatch(1)
        val device = adapter.getRemoteDevice(address)
        val servicesDiscovered = CountDownLatch(1)
        val writeDone = CountDownLatch(1)
        val readDone = CountDownLatch(1)
        val hostPubkey: AtomicReference<ByteArray?> = AtomicReference(null)
        val failure: AtomicReference<String?> = AtomicReference(null)

        val callback = object : BluetoothGattCallback() {
            override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                if (newState == BluetoothGatt.STATE_CONNECTED) {
                    // Clear the stale per-device GATT cache first: the desktop
                    // re-registers its pair application on every start, and a
                    // cached service tree makes `discoverServices()` hand back
                    // the dead registration (a fresh handshake is what the
                    // unlock client already does).
                    refreshGattCache(gatt)
                    runCatching { gatt.discoverServices() }
                } else if (newState == BluetoothGatt.STATE_DISCONNECTED) {
                    if (hostPubkey.get() == null && failure.get() == null) {
                        failure.set("GATT disconnected before host-pubkey read")
                    }
                    servicesDiscovered.countDown()
                    writeDone.countDown()
                    readDone.countDown()
                    nameDone.countDown()
                }
            }

            override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                servicesDiscovered.countDown()
            }

            override fun onCharacteristicWrite(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                status: Int,
            ) {
                if (characteristic.uuid == SYAUTH_PAIR_PHONE_PUBKEY_CHAR_UUID) {
                    if (status != BluetoothGatt.GATT_SUCCESS) {
                        failure.set("phone-pubkey write failed status=$status")
                    }
                    writeDone.countDown()
                } else if (characteristic.uuid == SYAUTH_PAIR_V2_CONTROL_CHAR_UUID) {
                    controlWriteStatus = status
                    controlWriteLatch?.countDown()
                }
            }

            override fun onCharacteristicRead(
                gatt: BluetoothGatt,
                characteristic: BluetoothGattCharacteristic,
                status: Int,
            ) {
                if (characteristic.uuid == SYAUTH_PAIR_V2_STATUS_CHAR_UUID) {
                    if (status == BluetoothGatt.GATT_SUCCESS) {
                        statusValue.set(characteristic.value?.copyOf())
                    }
                    statusReadLatch?.countDown()
                } else if (characteristic.uuid == SYAUTH_PAIR_HOST_NAME_V1_CHAR_UUID) {
                    if (status == BluetoothGatt.GATT_SUCCESS) {
                        try {
                            hostDisplayName = decodePeerDisplayName(characteristic.value ?: byteArrayOf())
                        } catch (_: Exception) {
                            failure.set("Invalid peer name metadata")
                        }
                    } else {
                        failure.set("Peer name read failed")
                    }
                    nameDone.countDown()
                } else if (characteristic.uuid == SYAUTH_PAIR_HOST_PUBKEY_CHAR_UUID) {
                    if (status == BluetoothGatt.GATT_SUCCESS) {
                        hostPubkey.set(characteristic.value?.copyOf())
                    } else {
                        failure.set("host-pubkey read failed status=$status")
                    }
                    readDone.countDown()
                }
            }
        }

        val gatt = device.connectGatt(context, false, callback)
        try {
            if (!servicesDiscovered.await(PAIR_GATT_EXCHANGE_TIMEOUT_SECS, TimeUnit.SECONDS)) {
                throw RuntimeException("service-discovery timeout")
            }
            failure.get()?.let { throw RuntimeException(it) }
            val service = gatt.getService(SYAUTH_PAIR_SERVICE_UUID)
                ?: throw RuntimeException("pair service not present on bonded device")
            val phoneChar = service.getCharacteristic(SYAUTH_PAIR_PHONE_PUBKEY_CHAR_UUID)
                ?: throw RuntimeException("phone-pubkey characteristic missing")
            controlCharacteristic = service.getCharacteristic(SYAUTH_PAIR_V2_CONTROL_CHAR_UUID)
                ?: throw RuntimeException("pair v2 control characteristic missing")
            statusCharacteristic = service.getCharacteristic(SYAUTH_PAIR_V2_STATUS_CHAR_UUID)
                ?: throw RuntimeException("pair v2 status characteristic missing")
            val hostChar = service.getCharacteristic(SYAUTH_PAIR_HOST_PUBKEY_CHAR_UUID)
                ?: throw RuntimeException("host-pubkey characteristic missing")
            // Read private display metadata before the public-key write
            // releases the desktop's scan wait. Old peers may omit this
            // optional field; that says nothing about commit capability.
            service.getCharacteristic(SYAUTH_PAIR_HOST_NAME_V1_CHAR_UUID)?.let { nameChar ->
                if (!gatt.readCharacteristic(nameChar) ||
                    !nameDone.await(PAIR_GATT_EXCHANGE_TIMEOUT_SECS, TimeUnit.SECONDS)) {
                    throw RuntimeException("Peer name read timeout")
                }
                failure.get()?.let { throw RuntimeException(it) }
            }
            phoneChar.value = phonePubkey
            if (!gatt.writeCharacteristic(phoneChar)) {
                throw RuntimeException("phone-pubkey writeCharacteristic refused")
            }
            if (!writeDone.await(PAIR_GATT_EXCHANGE_TIMEOUT_SECS, TimeUnit.SECONDS)) {
                throw RuntimeException("phone-pubkey write timeout")
            }
            failure.get()?.let { throw RuntimeException(it) }
            if (!gatt.readCharacteristic(hostChar)) {
                throw RuntimeException("host-pubkey readCharacteristic refused")
            }
            if (!readDone.await(PAIR_GATT_EXCHANGE_TIMEOUT_SECS, TimeUnit.SECONDS)) {
                throw RuntimeException("host-pubkey read timeout")
            }
            failure.get()?.let { throw RuntimeException(it) }
            activeGatt = gatt
            return hostPubkey.get() ?: throw RuntimeException("host-pubkey read returned null")
        } finally {
            if (activeGatt !== gatt) {
                runCatching { gatt.disconnect() }
                runCatching { gatt.close() }
            }
        }
    }
}
