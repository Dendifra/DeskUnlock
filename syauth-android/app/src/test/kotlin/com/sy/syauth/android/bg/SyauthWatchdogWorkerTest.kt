// Roadmap item S-012 — Robolectric JVM test for the
// `SyauthWatchdogWorker`. Pins the DoD bullet verbatim:
//
//   - `resurrects_killed_service` — given `isRunning = false` and a
//     bond on disk, running the worker queues a `startForegroundService`
//     against `SyauthCompanionService` and returns `Result.success()`.
//
// Journey: specs/journeys/JOURNEY-S-012-boot-receiver-watchdog.md
package com.sy.syauth.android.bg

import android.app.Application
import androidx.test.core.app.ApplicationProvider
import androidx.work.ListenableWorker
import androidx.work.testing.TestListenableWorkerBuilder
import com.sy.syauth.android.bond.BondRecord
import com.sy.syauth.android.bond.BondStore
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val FIXTURE_BOND_KEY_LEN: Int = 32
private const val FIXTURE_HOST: String = "alex-desktop"
private const val FIXTURE_PEER: String = "DD:EE:FF:00:11:22"
private const val FIXTURE_KEYSTORE_ALIAS: String = "syauth.watchdog.alias"

private fun fixtureBond(): BondRecord = BondRecord(
    peerId = FIXTURE_PEER,
    hostName = FIXTURE_HOST,
    bondKey = ByteArray(FIXTURE_BOND_KEY_LEN) { it.toByte() },
    keystoreAlias = FIXTURE_KEYSTORE_ALIAS,
    phonePubkey = ByteArray(FIXTURE_BOND_KEY_LEN) { it.toByte() },
)

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class SyauthWatchdogWorkerTest {

    private val app: Application
        get() = ApplicationProvider.getApplicationContext()

    @After
    fun cleanup() {
        BondStore(app.filesDir).storePath.delete()
        SyauthCompanionService.isRunning.set(false)
    }

    @Test
    fun resurrects_killed_service() {
        SyauthCompanionService.isRunning.set(false)
        BondStore(app.filesDir).save(fixtureBond())

        val worker = TestListenableWorkerBuilder
            .from(app, SyauthWatchdogWorker::class.java)
            .build()
        val result = worker.startWork().get()

        assertEquals(ListenableWorker.Result.success(), result)
        val started = shadowOf(app).nextStartedService
        assertNotNull("expected SyauthCompanionService start, got null", started)
        assertEquals(
            SyauthCompanionService::class.java.name,
            started!!.component?.className,
        )
    }

    /**
     * Day-2 guard (observed 2026-09-23): with the service already running the
     * worker used to do nothing, so a bond set that changed behind the
     * service's back (a dissociation, a re-pair) left a stale GATT client
     * alive — the app UI said "No computer paired" while the service kept
     * heartbeating for the old bond, the desktop stayed `Connected: no` and
     * presence samples dried up. Every tick must now ask the service to
     * reconcile, and the request must carry the reload action.
     */
    @Test
    fun asks_the_running_service_to_reconcile_its_clients() {
        SyauthCompanionService.isRunning.set(true)
        BondStore(app.filesDir).save(fixtureBond())

        val worker = TestListenableWorkerBuilder
            .from(app, SyauthWatchdogWorker::class.java)
            .build()
        val result = worker.startWork().get()

        assertEquals(ListenableWorker.Result.success(), result)
        val request = shadowOf(app).nextStartedService
        assertNotNull("expected a reload request against SyauthCompanionService", request)
        assertEquals(SyauthCompanionService::class.java.name, request!!.component?.className)
        assertEquals(
            "the request must carry the reload action",
            SyauthCompanionService.ACTION_RELOAD_BONDS,
            request.action,
        )
    }

    /**
     * With no service running there is nothing to reconcile — the resurrect
     * path owns that case and must not be pre-empted by a reload request.
     */
    @Test
    fun does_not_reload_when_the_service_is_not_running() {
        SyauthCompanionService.isRunning.set(false)
        BondStore(app.filesDir).save(fixtureBond())

        TestListenableWorkerBuilder
            .from(app, SyauthWatchdogWorker::class.java)
            .build()
            .startWork()
            .get()

        val resurrected = shadowOf(app).nextStartedService
        assertNotNull("resurrection must still happen", resurrected)
        assertEquals("resurrection must not carry the reload action", null, resurrected!!.action)
        assertEquals("no further start request when nothing is running", null, shadowOf(app).nextStartedService)
    }
}
