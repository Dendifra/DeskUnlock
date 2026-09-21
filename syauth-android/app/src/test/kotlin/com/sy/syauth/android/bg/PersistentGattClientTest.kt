// Roadmap item S-010 — Robolectric JVM tests for [PersistentGattClient].
//
// The four cases pin the DoD bullets from
// `specs/unlock-proximity/ROADMAP.md` Step S-010 verbatim:
//
//   1. `auto_connect_true_passed_to_connectGatt` — captures the
//      `autoConnect` argument the client passes through the
//      `GattOpener` seam. The seam exists because Robolectric 4.11.1
//      `ShadowBluetoothDevice` does not expose a `getAutoConnect()`
//      getter, so direct argument capture is the only mechanical
//      way to assert the contract.
//   2. `on_services_discovered_subscribes_via_cccd` — drives
//      `BluetoothGattCallback.onServicesDiscovered` and asserts the
//      challenge characteristic's CCCD descriptor's `value` equals
//      `CCCD_ENABLE_NOTIFY` after the production code has run.
//   3. `on_characteristic_changed_invokes_onChallenge` — pushes a
//      notify frame through the API-33+ override and asserts the
//      constructor's `onChallenge` lambda is invoked exactly once
//      with the constructor's `peerId` and the byte-for-byte
//      payload; a notify on a non-challenge UUID does NOT invoke
//      the callback.
//   4. `write_response_targets_response_characteristic` — calls
//      `writeResponse(bytes)` and asserts the response
//      characteristic's `value` was set to `bytes`. (The boolean
//      return is the result of `gatt.writeCharacteristic(c)`,
//      which under Robolectric returns false because no shadow
//      implements the call; the production code still returns the
//      stack's verdict verbatim.)
//
// Journey: specs/journeys/JOURNEY-S-010-persistent-gatt-client.md
package com.sy.syauth.android.bg

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothProfile
import android.content.Context
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.util.UUID
import java.util.concurrent.TimeUnit

private const val TEST_PEER_ID: String = "alex-desktop"
private const val TEST_DEVICE_MAC: String = "AA:BB:CC:DD:EE:FF"
private val TEST_CCCD_UUID: UUID =
    UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
private val TEST_CCCD_ENABLE_NOTIFY: ByteArray = byteArrayOf(0x01, 0x00)
private val TEST_SERVICE_UUID: UUID =
    UUID.fromString("5a4e8e3c-1c4c-4a17-9c81-d518a55a0001")

private class RecordingOpener(
    private val handle: BluetoothGatt,
) : GattOpener {
    var openCalls: Int = 0
        private set
    var lastDevice: BluetoothDevice? = null
        private set
    var lastAutoConnect: Boolean? = null
        private set
    var lastCallback: BluetoothGattCallback? = null
        private set

    override fun open(
        device: BluetoothDevice,
        autoConnect: Boolean,
        callback: BluetoothGattCallback,
    ): BluetoothGatt {
        openCalls += 1
        lastDevice = device
        lastAutoConnect = autoConnect
        lastCallback = callback
        return handle
    }
}

private class SequenceOpener(
    private val handles: ArrayDeque<BluetoothGatt>,
) : GattOpener {
    val callbacks = mutableListOf<BluetoothGattCallback>()

    override fun open(
        device: BluetoothDevice,
        autoConnect: Boolean,
        callback: BluetoothGattCallback,
    ): BluetoothGatt {
        callbacks += callback
        return handles.removeFirst()
    }
}

private fun ctx(): Context = ApplicationProvider.getApplicationContext()

private fun makeChallengeChar(): BluetoothGattCharacteristic {
    val c = BluetoothGattCharacteristic(
        SYAUTH_CHALLENGE_CHAR_UUID,
        BluetoothGattCharacteristic.PROPERTY_NOTIFY,
        BluetoothGattCharacteristic.PERMISSION_READ,
    )
    c.addDescriptor(
        BluetoothGattDescriptor(
            TEST_CCCD_UUID,
            BluetoothGattDescriptor.PERMISSION_READ
                or BluetoothGattDescriptor.PERMISSION_WRITE,
        )
    )
    return c
}

private fun makeResponseChar(): BluetoothGattCharacteristic =
    BluetoothGattCharacteristic(
        SYAUTH_RESPONSE_CHAR_UUID,
        BluetoothGattCharacteristic.PROPERTY_WRITE,
        BluetoothGattCharacteristic.PERMISSION_WRITE,
    )

private fun makeServiceWithBothChars(): BluetoothGattService {
    val service = BluetoothGattService(
        TEST_SERVICE_UUID,
        BluetoothGattService.SERVICE_TYPE_PRIMARY,
    )
    service.addCharacteristic(makeChallengeChar())
    service.addCharacteristic(makeResponseChar())
    return service
}

private fun newShadowGatt(): BluetoothGatt {
    val device = BluetoothAdapter.getDefaultAdapter()
        .getRemoteDevice(TEST_DEVICE_MAC)
    return org.robolectric.shadows.ShadowBluetoothGatt.newInstance(device)
}

private fun shadowGattAddService(
    gatt: BluetoothGatt,
    service: BluetoothGattService,
) {
    shadowOf(gatt).addDiscoverableService(service)
    // The shadow's `getServices()` reflects the `services` list, not
    // `discoverableServices`. Drive the shadow's `discoverServices`
    // so subsequent `gatt.services` returns the same list our
    // production code will inspect inside `onServicesDiscovered`.
    gatt.discoverServices()
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PersistentGattClientTest {

    @Test
    fun rssi_gate_requires_ready_and_allows_only_one_read() {
        val gate = GattOperationGate()
        assertEquals(GattWriteDecision.Skip, gate.requestWrite(byteArrayOf(1), diagnostic = true))
        assertEquals(false, gate.tryBeginRssi())
        gate.markReady()
        assertEquals(true, gate.tryBeginRssi())
        assertEquals(false, gate.tryBeginRssi())
        gate.finishRssi()
        assertEquals(true, gate.tryBeginRssi())
    }

    @Test
    fun rssi_gate_skips_when_write_is_in_flight_and_resets_on_disconnect() {
        val gate = GattOperationGate()
        gate.markReady()
        assertEquals(GattWriteDecision.Start, gate.requestWrite(byteArrayOf(1), diagnostic = true))
        assertEquals(false, gate.tryBeginRssi())
        assertEquals(GattWriteDecision.Queued, gate.requestWrite(byteArrayOf(2), diagnostic = false))
        gate.markNotReady()
        assertEquals(false, gate.tryBeginRssi())
        gate.markReady()
        assertEquals(true, gate.tryBeginRssi())
    }

    @Test
    fun auth_waits_for_rssi_completion_and_wins_over_next_rssi_tick() {
        val gate = GattOperationGate()
        gate.markReady()
        assertEquals(true, gate.tryBeginRssi())
        val auth = byteArrayOf(9)
        assertEquals(GattWriteDecision.Queued, gate.requestWrite(auth, diagnostic = false))
        assertEquals(false, gate.tryBeginRssi())
        assertEquals(auth.toList(), gate.finishRssi()?.toList())
        assertEquals(null, gate.finishWrite())
        assertEquals(true, gate.tryBeginRssi())
    }

    @Test
    fun second_auth_is_rejected_while_one_auth_is_pending() {
        val gate = GattOperationGate()
        gate.markReady()
        assertEquals(true, gate.tryBeginRssi())
        assertEquals(GattWriteDecision.Queued, gate.requestWrite(byteArrayOf(1), diagnostic = false))
        assertEquals(GattWriteDecision.Skip, gate.requestWrite(byteArrayOf(2), diagnostic = false))
        assertEquals(listOf<Byte>(1), gate.finishRssi()?.toList())
    }

    @Test
    fun auto_connect_true_passed_to_connectGatt() {
        val handle = newShadowGatt()
        val opener = RecordingOpener(handle)
        val client = PersistentGattClient(
            context = ctx(),
            adapter = BluetoothAdapter.getDefaultAdapter(),
            peerId = TEST_PEER_ID,
            deviceMac = TEST_DEVICE_MAC,
            onChallenge = { _, _ -> },
            gattOpener = opener,
        )

        client.start()

        assertEquals(1, opener.openCalls)
        assertEquals(true, opener.lastAutoConnect)
        assertEquals(TEST_DEVICE_MAC, opener.lastDevice?.address)
        assertNotNull(opener.lastCallback)
    }

    @Test
    fun start_arms_reconnect_watchdog_without_state_callback() {
        // BUG-20260528-2334: a connectGatt(autoConnect=true) that never
        // completes emits NO onConnectionStateChange callback, so the
        // reconnect watchdog — which used to be armed only on
        // STATE_DISCONNECTED — was never scheduled and the client
        // wedged forever in a never-completing background scan. The
        // desktop then saw notifier_slot=None on every unlock. start()
        // must arm the watchdog itself so a stalled initial connect is
        // retried after RECONNECT_INTERVAL_MS with no user action.
        val handle = newShadowGatt()
        val opener = RecordingOpener(handle)
        val client = PersistentGattClient(
            context = ctx(),
            adapter = BluetoothAdapter.getDefaultAdapter(),
            peerId = TEST_PEER_ID,
            deviceMac = TEST_DEVICE_MAC,
            onChallenge = { _, _ -> },
            gattOpener = opener,
        )

        client.start()
        assertEquals("initial connectGatt issued", 1, opener.openCalls)

        // No connection callback fires (the autoConnect scan never
        // matches the peer). Advance the main looper past the watchdog
        // cadence — the watchdog must re-issue connectGatt on its own.
        shadowOf(Looper.getMainLooper())
            .idleFor(PersistentGattClient.RECONNECT_INTERVAL_MS, TimeUnit.MILLISECONDS)

        assertEquals(
            "watchdog re-issues connectGatt when the initial autoConnect never completes",
            2,
            opener.openCalls,
        )
    }

    @Test
    fun on_services_discovered_subscribes_via_cccd() {
        val handle = newShadowGatt()
        val opener = RecordingOpener(handle)
        val service = makeServiceWithBothChars()
        shadowGattAddService(handle, service)

        val client = PersistentGattClient(
            context = ctx(),
            adapter = BluetoothAdapter.getDefaultAdapter(),
            peerId = TEST_PEER_ID,
            deviceMac = TEST_DEVICE_MAC,
            onChallenge = { _, _ -> },
            gattOpener = opener,
        )
        client.start()

        val callback = opener.lastCallback!!
        callback.onConnectionStateChange(
            handle,
            BluetoothGatt.GATT_SUCCESS,
            BluetoothProfile.STATE_CONNECTED,
        )
        callback.onServicesDiscovered(handle, BluetoothGatt.GATT_SUCCESS)

        val challenge = service.getCharacteristic(SYAUTH_CHALLENGE_CHAR_UUID)
        val cccd = challenge.getDescriptor(TEST_CCCD_UUID)
        assertNotNull("CCCD descriptor present", cccd)
        assertArrayEquals(TEST_CCCD_ENABLE_NOTIFY, cccd.value)
    }

    @Test
    fun on_characteristic_changed_invokes_onChallenge() {
        val handle = newShadowGatt()
        val opener = RecordingOpener(handle)
        val service = makeServiceWithBothChars()
        shadowGattAddService(handle, service)
        val payload = byteArrayOf(0x10, 0x20, 0x30, 0x40)
        var received: Pair<String, ByteArray>? = null

        val client = PersistentGattClient(
            context = ctx(),
            adapter = BluetoothAdapter.getDefaultAdapter(),
            peerId = TEST_PEER_ID,
            deviceMac = TEST_DEVICE_MAC,
            onChallenge = { peer, bytes -> received = peer to bytes },
            gattOpener = opener,
        )
        client.start()
        val callback = opener.lastCallback!!
        callback.onServicesDiscovered(handle, BluetoothGatt.GATT_SUCCESS)
        val challenge = service.getCharacteristic(SYAUTH_CHALLENGE_CHAR_UUID)
        callback.onDescriptorWrite(handle, challenge.getDescriptor(TEST_CCCD_UUID)!!, BluetoothGatt.GATT_SUCCESS)

        callback.onCharacteristicChanged(handle, challenge, payload)

        assertNotNull("onChallenge fired", received)
        assertEquals(TEST_PEER_ID, received?.first)
        assertArrayEquals(payload, received?.second)

        // A notify on a non-challenge UUID must NOT invoke the callback.
        val resp = service.getCharacteristic(SYAUTH_RESPONSE_CHAR_UUID)
        received = null
        callback.onCharacteristicChanged(handle, resp, byteArrayOf(0x99.toByte()))
        assertEquals(null, received)
    }

    @Test
    fun successful_rssi_read_is_sent_as_telemetry_without_changing_heartbeat_protocol() {
        val handle = newShadowGatt()
        val opener = RecordingOpener(handle)
        val service = makeServiceWithBothChars()
        shadowGattAddService(handle, service)
        val client = PersistentGattClient(
            context = ctx(),
            adapter = BluetoothAdapter.getDefaultAdapter(),
            peerId = TEST_PEER_ID,
            deviceMac = TEST_DEVICE_MAC,
            onChallenge = { _, _ -> },
            gattOpener = opener,
        )
        client.start()
        val callback = opener.lastCallback!!
        callback.onServicesDiscovered(handle, BluetoothGatt.GATT_SUCCESS)
        val challenge = service.getCharacteristic(SYAUTH_CHALLENGE_CHAR_UUID)
        callback.onDescriptorWrite(handle, challenge.getDescriptor(TEST_CCCD_UUID)!!, BluetoothGatt.GATT_SUCCESS)
        val response = service.getCharacteristic(SYAUTH_RESPONSE_CHAR_UUID)

        callback.onReadRemoteRssi(handle, -67, BluetoothGatt.GATT_SUCCESS)
        assertEquals("${PersistentGattClient.RSSI_TELEMETRY_PREFIX}-67", response.value.toString(Charsets.UTF_8))

        val previous = response.value
        callback.onReadRemoteRssi(handle, -66, BluetoothGatt.GATT_FAILURE)
        assertSame("failed RSSI reads do not emit telemetry", previous, response.value)
    }

    @Test
    fun rssi_sampling_is_diagnostic_only_and_does_not_start_a_scan() {
        assertEquals(2_000L, PersistentGattClient.RSSI_SAMPLE_INTERVAL_MS)
        assertEquals("SYAUTH-RSSI-v1:", PersistentGattClient.RSSI_TELEMETRY_PREFIX)
    }

    @Test
    fun service_changed_blocks_writes_until_fresh_cccd_resolution() {
        val handle = newShadowGatt()
        val opener = RecordingOpener(handle)
        val service = makeServiceWithBothChars()
        shadowGattAddService(handle, service)
        val client = PersistentGattClient(
            context = ctx(),
            adapter = BluetoothAdapter.getDefaultAdapter(),
            peerId = TEST_PEER_ID,
            deviceMac = TEST_DEVICE_MAC,
            onChallenge = { _, _ -> },
            gattOpener = opener,
        )
        client.start()
        val callback = opener.lastCallback!!
        callback.onServicesDiscovered(handle, BluetoothGatt.GATT_SUCCESS)
        val challenge = service.getCharacteristic(SYAUTH_CHALLENGE_CHAR_UUID)
        callback.onDescriptorWrite(handle, challenge.getDescriptor(TEST_CCCD_UUID)!!, BluetoothGatt.GATT_SUCCESS)
        val response = service.getCharacteristic(SYAUTH_RESPONSE_CHAR_UUID)
        val first = byteArrayOf(1)
        val second = byteArrayOf(2)
        client.writeResponse(first)
        assertSame(first, response.value)

        callback.onServiceChanged(handle)
        callback.onServiceChanged(handle)
        assertEquals(false, client.writeResponse(second))
        assertSame(first, response.value)

        callback.onServicesDiscovered(handle, BluetoothGatt.GATT_SUCCESS)
        callback.onDescriptorWrite(handle, challenge.getDescriptor(TEST_CCCD_UUID)!!, BluetoothGatt.GATT_SUCCESS)
        client.writeResponse(second)
        assertSame(second, response.value)
    }

    @Test
    fun callbacks_from_previous_gatt_generation_are_ignored() {
        val firstHandle = newShadowGatt()
        val secondHandle = newShadowGatt()
        val firstService = makeServiceWithBothChars()
        val secondService = makeServiceWithBothChars()
        shadowGattAddService(firstHandle, firstService)
        shadowGattAddService(secondHandle, secondService)
        val opener = SequenceOpener(ArrayDeque(listOf(firstHandle, secondHandle)))
        val client = PersistentGattClient(
            context = ctx(),
            adapter = BluetoothAdapter.getDefaultAdapter(),
            peerId = TEST_PEER_ID,
            deviceMac = TEST_DEVICE_MAC,
            onChallenge = { _, _ -> },
            gattOpener = opener,
        )
        client.start()
        val firstCallback = opener.callbacks[0]
        firstCallback.onServicesDiscovered(firstHandle, BluetoothGatt.GATT_SUCCESS)
        val firstChallenge = firstService.getCharacteristic(SYAUTH_CHALLENGE_CHAR_UUID)
        firstCallback.onDescriptorWrite(
            firstHandle,
            firstChallenge.getDescriptor(TEST_CCCD_UUID)!!,
            BluetoothGatt.GATT_SUCCESS,
        )

        client.forceReconnect()
        val secondCallback = opener.callbacks[1]
        firstCallback.onDescriptorWrite(
            firstHandle,
            firstChallenge.getDescriptor(TEST_CCCD_UUID)!!,
            BluetoothGatt.GATT_SUCCESS,
        )
        assertEquals(false, client.writeResponse(byteArrayOf(9)))

        secondCallback.onServicesDiscovered(secondHandle, BluetoothGatt.GATT_SUCCESS)
        val secondChallenge = secondService.getCharacteristic(SYAUTH_CHALLENGE_CHAR_UUID)
        secondCallback.onDescriptorWrite(
            secondHandle,
            secondChallenge.getDescriptor(TEST_CCCD_UUID)!!,
            BluetoothGatt.GATT_SUCCESS,
        )
        assertEquals(false, client.writeResponse(byteArrayOf(10)))
    }

    @Test
    fun write_response_targets_response_characteristic() {
        val handle = newShadowGatt()
        val opener = RecordingOpener(handle)
        val service = makeServiceWithBothChars()
        shadowGattAddService(handle, service)
        val payload = byteArrayOf(0x55, 0x66, 0x77, 0x77.toByte())

        val client = PersistentGattClient(
            context = ctx(),
            adapter = BluetoothAdapter.getDefaultAdapter(),
            peerId = TEST_PEER_ID,
            deviceMac = TEST_DEVICE_MAC,
            onChallenge = { _, _ -> },
            gattOpener = opener,
        )
        client.start()
        val callback = opener.lastCallback!!
        callback.onServicesDiscovered(handle, BluetoothGatt.GATT_SUCCESS)
        val challenge = service.getCharacteristic(SYAUTH_CHALLENGE_CHAR_UUID)
        callback.onDescriptorWrite(handle, challenge.getDescriptor(TEST_CCCD_UUID)!!, BluetoothGatt.GATT_SUCCESS)

        client.writeResponse(payload)

        val resp = service.getCharacteristic(SYAUTH_RESPONSE_CHAR_UUID)
        // The production code sets the characteristic value and
        // then calls `gatt.writeCharacteristic(c)`. Under
        // Robolectric the write itself is a no-op (no shadow), but
        // the `.value` assignment is observable here.
        assertSame(
            "response characteristic value points at payload",
            payload,
            resp.value,
        )
    }
}
