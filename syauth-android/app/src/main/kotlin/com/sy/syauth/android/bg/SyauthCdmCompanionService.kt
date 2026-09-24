package com.sy.syauth.android.bg

import android.companion.AssociationInfo
import android.companion.CompanionDeviceService
import android.util.Log

/** Logcat tag for the CDM (companion-device) binding path. */
internal const val CDM_SERVICE_LOG_TAG: String = "syauth.bg.cdm"

/**
 * Companion-device service the OS binds when an associated desktop comes into
 * range.
 *
 * This is the only resurrection trigger that works without the user opening the
 * app: the OS calls [onDeviceAppeared] as soon as the associated peer is nearby.
 * The class used to be empty, so nothing happened — the app looked "associated"
 * while no service was running and nothing was connected, and it stayed inert
 * until it was opened by hand (observed on the device, 2026-09-23: after every
 * app update the phone never reconnected on its own).
 *
 * Bond presence is the sole gate: [resurrectIfDead] does nothing when no bond
 * exists, so an unassociated phone costs one log line.
 */
public class SyauthCdmCompanionService : CompanionDeviceService() {
    override fun onDeviceAppeared(associationInfo: AssociationInfo) {
        Log.i(
            CDM_SERVICE_LOG_TAG,
            "device appeared (association=${associationInfo.id}); checking bond + service liveness",
        )
        resurrectIfDead(this)
    }

    override fun onDeviceDisappeared(associationInfo: AssociationInfo) {
        // The desktop decides proximity from its own RSSI samples, so nothing
        // has to happen here; the line is what makes a field report readable.
        Log.i(CDM_SERVICE_LOG_TAG, "device disappeared (association=${associationInfo.id})")
    }
}
