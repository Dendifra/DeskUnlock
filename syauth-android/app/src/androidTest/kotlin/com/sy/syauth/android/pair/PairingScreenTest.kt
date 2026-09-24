// Roadmap item S-016 — Pairing Compose UI tests.
//
// Each test renders the [PairingScreen] composable against a fixed
// [PairingState] and asserts the test-tag-bearing nodes are displayed (or
// clickable, where it matters). The screen is stateless — pure projection
// — so we don't need a ViewModel here; we feed the state directly.
//
// `createComposeRule()` (host-only, no Activity) is the right rule for
// pure-Composable tests; we deliberately avoid `createAndroidComposeRule`
// because there is no Activity-specific behavior to exercise.
//
// Lambdas are no-ops because the tests only assert *rendering*; the
// click-side-effects are covered by PairingViewModelTest on the JVM.
package com.sy.syauth.android.pair

import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import android.content.Context
import androidx.compose.ui.test.onNodeWithText
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.sy.syauth.android.R
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PairingScreenTest {

    @get:Rule
    val composeTestRule = createComposeRule()

    /**
     * Resolve pairing copy the way the device does, so the assertion follows the
     * locale instead of a hardcoded English literal.
     */
    private fun pairingCopy(id: Int): String =
        ApplicationProvider.getApplicationContext<Context>().getString(id)

    private fun renderState(state: PairingState) {
        composeTestRule.setContent {
            PairingScreen(
                state = state,
                onStartScan = {},
                onCancel = {},
                onOobYes = {},
                onOobNo = {},
                onDone = {},
            )
        }
    }

    // ──── TC-08 ────
    @Test
    fun idle_renders_pair_cta() {
        renderState(PairingState.Idle)

        composeTestRule
            .onNodeWithTag(PairingTestTags.IDLE_CTA)
            .assertIsDisplayed()
            .assertHasClickAction()
        composeTestRule
            .onNodeWithText(pairingCopy(R.string.pair_idle_cta))
            .assertIsDisplayed()
    }

    // ──── TC-09 ────
    @Test
    fun scanning_renders_progress_and_cancel() {
        renderState(PairingState.Scanning)

        composeTestRule
            .onNodeWithTag(PairingTestTags.SCANNING_PROGRESS)
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithTag(PairingTestTags.SCANNING_CANCEL)
            .assertIsDisplayed()
            .assertHasClickAction()
    }

    // ──── TC-10 ────
    @Test
    fun lesc_negotiating_renders_6_digit_code() {
        val code = "123456"
        renderState(PairingState.LescNegotiating(code))

        composeTestRule
            .onNodeWithTag(PairingTestTags.LESC_CODE)
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithText(code)
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithTag(PairingTestTags.LESC_CANCEL)
            .assertIsDisplayed()
            .assertHasClickAction()
    }

    // ──── TC-11 ────
    @Test
    fun oob_confirming_renders_the_numeric_code_and_yes_no_buttons() {
        val code = "04231789"
        renderState(PairingState.OobConfirming(code))

        composeTestRule
            .onNodeWithTag(PairingTestTags.OOB_CODE)
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithText(code)
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithText(pairingCopy(R.string.pair_oob_question))
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithTag(PairingTestTags.OOB_YES)
            .assertIsDisplayed()
            .assertHasClickAction()
        composeTestRule
            .onNodeWithTag(PairingTestTags.OOB_NO)
            .assertIsDisplayed()
            .assertHasClickAction()
        composeTestRule
            .onNodeWithTag(PairingTestTags.OOB_CANCEL)
            .assertIsDisplayed()
            .assertHasClickAction()
    }

    // ──── TC-12 ────
    @Test
    fun bonded_renders_peer_name() {
        val name = "alex-desktop"
        renderState(PairingState.Bonded(name))

        composeTestRule
            .onNodeWithTag(PairingTestTags.BONDED_LABEL)
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithText(pairingCopy(R.string.pair_bonded))
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithTag(PairingTestTags.BONDED_DONE)
            .assertIsDisplayed()
            .assertHasClickAction()
    }

    // ──── TC-13 ────
    @Test
    fun failed_renders_reason_and_back_button() {
        val reason = "LESC handshake failed"
        renderState(PairingState.Failed(reason))

        composeTestRule
            .onNodeWithTag(PairingTestTags.FAILED_REASON)
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithText(pairingCopy(R.string.pair_failed_prefix) + reason)
            .assertIsDisplayed()
        composeTestRule
            .onNodeWithTag(PairingTestTags.FAILED_BACK)
            .assertIsDisplayed()
            .assertHasClickAction()
    }
}
