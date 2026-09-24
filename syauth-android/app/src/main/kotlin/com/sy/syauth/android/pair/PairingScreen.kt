// Roadmap item S-016 — Pairing Compose screen.
//
// The screen is a pure projection of [PairingState] to Compose nodes. It
// never reads side-effect state, never owns timers, never calls into
// `BluetoothDevice.removeBond()` (DoD #3 — the screen MUST go through
// the [BluetoothBondRemover] interface via the ViewModel).
//
// Every interactive node carries a `testTag` so the Compose UI test in
// `PairingScreenTest.kt` can target it without depending on rendered
// text. The test tags are the public contract of the screen for tests.
//
// Why a `state: PairingState` parameter instead of a `viewModel: …`:
//   The Compose UI test renders this screen against fixed states; the
//   ViewModel is exercised by the Robolectric unit tests. Passing the
//   state in lets us test each branch independently without spinning
//   up the entire ViewModel + fakes for a render check.
package com.sy.syauth.android.pair

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.unit.dp
import com.sy.syauth.android.pair.api.PEER_REJECTED_REASON

/**
 * Test tags. Public so [PairingScreenTest] can use the same constants.
 */
object PairingTestTags {
    const val IDLE_CTA: String = "pair.idle.cta"
    const val SCANNING_PROGRESS: String = "pair.scanning.progress"
    const val SCANNING_CANCEL: String = "pair.scanning.cancel"
    const val LESC_CODE: String = "pair.lesc.code"
    const val LESC_CANCEL: String = "pair.lesc.cancel"
    const val OOB_CODE: String = "pair.oob.code"
    const val OOB_YES: String = "pair.oob.yes"
    const val OOB_NO: String = "pair.oob.no"
    const val OOB_CANCEL: String = "pair.oob.cancel"
    const val FINALIZING_PROGRESS: String = "pair.finalizing.progress"
    const val UNCERTAIN_REASON: String = "pair.uncertain.reason"
    const val BONDED_LABEL: String = "pair.bonded.label"
    const val BONDED_DONE: String = "pair.bonded.done"
    const val FAILED_REASON: String = "pair.failed.reason"
    const val FAILED_BACK: String = "pair.failed.back"
}

/** Static UI strings. Centralised so tests assert on the same constants. */
object PairingStrings {
    const val IDLE_CTA: String = "Find computer"
    const val IDLE_TITLE: String = "Pair a computer"
    const val IDLE_DESCRIPTION: String = "Link your phone to your computer securely over Bluetooth."
    const val SEARCHING: String = "Searching for nearby DeskUnlock computers…"
    const val FOUND_PREFIX: String = "Computer found: "
    const val BLUETOOTH_HELP: String = "Check that the Bluetooth code matches on both devices."
    const val OOB_HELP: String = "Check that this number matches the one shown on the computer."
    const val FINALIZING: String = "Finishing pairing…"
    const val CANCEL: String = "Cancel"
    const val OOB_QUESTION: String = "Do the numbers match?"
    const val OOB_YES: String = "Sì"
    const val OOB_NO: String = "No"
    const val BONDED: String = "Computer paired"
    const val DONE: String = "Done"
    const val FAILED_PREFIX: String = "Pairing failed: "

    /**
     * Shown instead of [FAILED_PREFIX] + reason when the computer refused the
     * transaction: the raw reason carries no action, this one names the arming
     * step on the computer (SPEC §6 T-004).
     */
    const val PEER_REJECTED_HELP: String =
        "The computer refused the pairing. On the computer, open DeskUnlock " +
            "and press \"Associa telefono\", then try again."
    const val BACK: String = "Back"
}

/**
 * Pairing screen.
 *
 * @param state authoritative state from [PairingViewModel.state].
 * @param onStartScan invoked when the Idle CTA is tapped.
 * @param onCancel invoked from Scanning / LescNegotiating.
 * @param onOobYes invoked from OobConfirming.
 * @param onOobNo invoked from OobConfirming.
 * @param onDone invoked from Bonded / Failed — pops the route back to home.
 */
@Composable
fun PairingScreen(
    state: PairingState,
    onStartScan: () -> Unit,
    onCancel: () -> Unit,
    onOobYes: () -> Unit,
    onOobNo: () -> Unit,
    onDone: () -> Unit,
    onRetry: (() -> Unit)? = null,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        when (state) {
            is PairingState.Idle -> IdleContent(onStartScan = onStartScan)
            is PairingState.Scanning -> ScanningContent(onCancel = onCancel)
            is PairingState.LescNegotiating -> LescContent(
                code = state.code,
                onCancel = onCancel,
            )
            is PairingState.OobConfirming -> OobContent(
                code = state.code,
                onYes = onOobYes,
                onNo = onOobNo,
                onCancel = onCancel,
            )
            is PairingState.Finalizing -> FinalizingContent()
            is PairingState.Uncertain -> UncertainContent(reason = state.reason, onRetry = onRetry ?: onDone)
            is PairingState.Bonded -> BondedContent(onDone = onDone)
            is PairingState.Failed -> FailedContent(reason = state.reason, onBack = onDone)
        }
    }
}

@Composable
private fun IdleContent(onStartScan: () -> Unit) {
    Text(text = "DeskUnlock", style = MaterialTheme.typography.headlineMedium)
    Spacer(modifier = Modifier.height(8.dp))
    Text(text = PairingStrings.IDLE_TITLE, style = MaterialTheme.typography.titleLarge)
    Spacer(modifier = Modifier.height(8.dp))
    Text(text = PairingStrings.IDLE_DESCRIPTION)
    Spacer(modifier = Modifier.height(24.dp))
    Button(
        onClick = onStartScan,
        modifier = Modifier
            .fillMaxWidth()
            .semantics { testTag = PairingTestTags.IDLE_CTA },
    ) {
        Text(text = PairingStrings.IDLE_CTA)
    }
}

@Composable
private fun ScanningContent(onCancel: () -> Unit) {
    Text(text = PairingStrings.SEARCHING)
    Spacer(modifier = Modifier.height(12.dp))
    CircularProgressIndicator(
        modifier = Modifier.semantics { testTag = PairingTestTags.SCANNING_PROGRESS },
    )
    Spacer(modifier = Modifier.height(24.dp))
    Button(
        onClick = onCancel,
        modifier = Modifier.semantics { testTag = PairingTestTags.SCANNING_CANCEL },
    ) {
        Text(text = PairingStrings.CANCEL)
    }
}

@Composable
private fun LescContent(code: String, onCancel: () -> Unit) {
    Text(text = "Verify Bluetooth", style = MaterialTheme.typography.titleLarge)
    Text(text = PairingStrings.BLUETOOTH_HELP)
    Spacer(modifier = Modifier.height(12.dp))
    Text(
        text = code,
        style = MaterialTheme.typography.headlineLarge,
        modifier = Modifier.semantics { testTag = PairingTestTags.LESC_CODE },
    )
    Spacer(modifier = Modifier.height(24.dp))
    Button(
        onClick = onCancel,
        modifier = Modifier.semantics { testTag = PairingTestTags.LESC_CANCEL },
    ) {
        Text(text = PairingStrings.CANCEL)
    }
}

@Composable
private fun OobContent(
    code: String,
    onYes: () -> Unit,
    onNo: () -> Unit,
    onCancel: () -> Unit,
) {
    Text(
        text = code,
        style = MaterialTheme.typography.headlineMedium,
        modifier = Modifier.semantics { testTag = PairingTestTags.OOB_CODE },
    )
    Spacer(modifier = Modifier.height(16.dp))
    Text(text = PairingStrings.OOB_HELP)
    Spacer(modifier = Modifier.height(8.dp))
    Text(text = PairingStrings.OOB_QUESTION)
    Spacer(modifier = Modifier.height(16.dp))
    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Button(
            onClick = onCancel,
            modifier = Modifier.semantics { testTag = PairingTestTags.OOB_CANCEL },
        ) {
            Text(text = PairingStrings.CANCEL)
        }
        Button(
            onClick = onYes,
            modifier = Modifier.semantics { testTag = PairingTestTags.OOB_YES },
        ) {
            Text(text = PairingStrings.OOB_YES)
        }
        Button(
            onClick = onNo,
            modifier = Modifier.semantics { testTag = PairingTestTags.OOB_NO },
        ) {
            Text(text = PairingStrings.OOB_NO)
        }
    }
}

@Composable
private fun FinalizingContent() {
    CircularProgressIndicator(modifier = Modifier.semantics { testTag = PairingTestTags.FINALIZING_PROGRESS })
    Spacer(modifier = Modifier.height(16.dp))
    Text(text = PairingStrings.FINALIZING)
}

@Composable
private fun UncertainContent(reason: String, onRetry: () -> Unit) {
    Text(text = "Verifying pairing…", style = MaterialTheme.typography.titleLarge)
    Spacer(modifier = Modifier.height(8.dp))
    Text(text = reason, modifier = Modifier.semantics { testTag = PairingTestTags.UNCERTAIN_REASON })
    Spacer(modifier = Modifier.height(16.dp))
    Button(onClick = onRetry) { Text(text = "Riprova") }
}

@Composable
private fun BondedContent(onDone: () -> Unit) {
    Text(
        text = PairingStrings.BONDED,
        modifier = Modifier.semantics { testTag = PairingTestTags.BONDED_LABEL },
    )
    Spacer(modifier = Modifier.height(24.dp))
    Button(
        onClick = onDone,
        modifier = Modifier.semantics { testTag = PairingTestTags.BONDED_DONE },
    ) {
        Text(text = PairingStrings.DONE)
    }
}

@Composable
private fun FailedContent(reason: String, onBack: () -> Unit) {
    Text(
        text = failure_message(reason),
        modifier = Modifier.semantics { testTag = PairingTestTags.FAILED_REASON },
    )
    Spacer(modifier = Modifier.height(24.dp))
    Button(
        onClick = onBack,
        modifier = Modifier.semantics { testTag = PairingTestTags.FAILED_BACK },
    ) {
        Text(text = PairingStrings.BACK)
    }
}

/**
 * Render one backend failure reason for the operator.
 *
 * A peer rejection is not a crash to report: it is the computer refusing the
 * transaction because nobody armed the pairing there (SPEC §6 T-004). The raw
 * reason says only that the remote confirmation failed, so it is replaced by
 * the next action to take (BUG-20260924: the operator saw nothing to act on).
 */
internal fun failure_message(reason: String): String =
    if (reason == PEER_REJECTED_REASON) {
        PairingStrings.PEER_REJECTED_HELP
    } else {
        PairingStrings.FAILED_PREFIX + reason
    }
