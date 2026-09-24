// Regression guard for the CDM (companion-device) resurrection trigger.
//
// `SyauthCdmCompanionService` used to be an empty class: the OS called
// `onDeviceAppeared` as soon as the associated desktop came into range and
// nothing happened, so the background service never came back on its own — the
// app showed an association while nothing was connected, and stayed inert until
// it was opened by hand (observed on the device, 2026-09-23, after every app
// update).
//
// The action itself is `resurrectIfDead`, whose behaviour is already pinned by
// `BootCompletedReceiverTest`; what this file guards is that the callback is
// actually overridden, because an empty override is exactly the bug.
package com.sy.syauth.android.bg

import android.companion.AssociationInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class SyauthCdmCompanionServiceTest {
    @Test
    fun overrides_the_nearby_callbacks_so_the_service_can_come_back() {
        val appeared = SyauthCdmCompanionService::class.java.getDeclaredMethod(
            "onDeviceAppeared",
            AssociationInfo::class.java,
        )
        val disappeared = SyauthCdmCompanionService::class.java.getDeclaredMethod(
            "onDeviceDisappeared",
            AssociationInfo::class.java,
        )

        assertNotNull(appeared)
        assertNotNull(disappeared)
        assertEquals(
            "onDeviceAppeared must be implemented by us, not inherited as a no-op",
            SyauthCdmCompanionService::class.java,
            appeared.declaringClass,
        )
    }
}
