package com.sy.syauth.android.pair

import android.bluetooth.BluetoothAdapter
import android.bluetooth.le.ScanResult
import android.companion.CompanionDeviceManager
import android.content.Intent
import com.sy.syauth.android.pair.impl.extractPickedPeerFromIntent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class AndroidCdmPairCompanionScannerTest {
    @Test
    fun extracts_bluetooth_device_without_type_exception() {
        val device = BluetoothAdapter.getDefaultAdapter().getRemoteDevice("AA:BB:CC:DD:EE:FF")
        val intent = Intent().putExtra(CompanionDeviceManager.EXTRA_DEVICE, device)
        assertEquals("AA:BB:CC:DD:EE:FF", extractPickedPeerFromIntent(intent)?.address)
    }

    @Test
    fun extracts_scan_result_device_without_casting_scan_result_as_device() {
        val device = BluetoothAdapter.getDefaultAdapter().getRemoteDevice("11:22:33:44:55:66")
        val result = ScanResult(device, null, -40, 1L)
        val intent = Intent().putExtra(CompanionDeviceManager.EXTRA_DEVICE, result)
        assertEquals("11:22:33:44:55:66", extractPickedPeerFromIntent(intent)?.address)
    }

    @Test
    fun absent_or_invalid_payload_is_rejected() {
        assertNull(extractPickedPeerFromIntent(Intent()))
    }
}
