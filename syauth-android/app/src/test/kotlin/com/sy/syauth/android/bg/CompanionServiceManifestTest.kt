package com.sy.syauth.android.bg

import android.app.Service
import android.companion.CompanionDeviceService
import android.content.Intent
import android.content.pm.PackageManager
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class CompanionServiceManifestTest {
    @Test
    fun separates_foreground_and_cdm_services() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val packageInfo = context.packageManager.getPackageInfo(
            context.packageName,
            PackageManager.GET_SERVICES,
        )
        val services = packageInfo.services.orEmpty().associateBy { it.name }
        val foreground = services["${context.packageName}.bg.SyauthCompanionService"]
        val cdm = services["${context.packageName}.bg.SyauthCdmCompanionService"]

        assertNotNull(foreground)
        assertNotNull(cdm)
        assertTrue(Service::class.java.isAssignableFrom(SyauthCompanionService::class.java))
        assertFalse(
            CompanionDeviceService::class.java.isAssignableFrom(
                SyauthCompanionService::class.java,
            ),
        )
        assertTrue(
            CompanionDeviceService::class.java.isAssignableFrom(
                SyauthCdmCompanionService::class.java,
            ),
        )
        assertFalse(foreground!!.exported)
        assertNull(foreground.permission)
        assertTrue(cdm!!.exported)
        assertEquals(
            "android.permission.BIND_COMPANION_DEVICE_SERVICE",
            cdm.permission,
        )

        val cdmServices = context.packageManager.queryIntentServices(
            Intent("android.companion.CompanionDeviceService")
                .setPackage(context.packageName),
            PackageManager.GET_INTENT_FILTERS,
        )
        assertEquals(1, cdmServices.size)
        assertEquals(cdm.name, cdmServices.single().serviceInfo.name)
    }

    /**
     * Regression guard (observed 2026-09-23): after an app update the
     * background service never came back — the app showed an association while
     * nothing was connected, until it was opened by hand. The receiver that
     * handles the update was declared `exported="false"`, so the system's
     * `MY_PACKAGE_REPLACED` broadcast was never delivered to it.
     *
     * Both resurrection receivers must be exported: their broadcasts come from
     * the system, not from another app.
     */
    @Test
    fun resurrection_receivers_are_exported_and_filter_the_system_broadcasts() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val packageInfo = context.packageManager.getPackageInfo(
            context.packageName,
            PackageManager.GET_RECEIVERS or PackageManager.GET_INTENT_FILTERS,
        )
        val receivers = packageInfo.receivers.orEmpty().associateBy { it.name }

        val boot = receivers["${context.packageName}.bg.BootCompletedReceiver"]
        val replaced = receivers["${context.packageName}.bg.PackageReplacedReceiver"]
        assertNotNull(boot)
        assertNotNull(replaced)
        assertTrue("BootCompletedReceiver must receive a system broadcast", boot!!.exported)
        assertTrue("PackageReplacedReceiver must receive a system broadcast", replaced!!.exported)
    }
}
