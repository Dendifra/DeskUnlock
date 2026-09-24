// Roadmap item S-017 — Approve screen Compose UI.
//
// Renders the user-facing surface for every unlock. Layout follows
// the prrr-android MainScreen pattern so the sibling apps share
// visual identity:
//
//   - Canonical DeskUnlock logo + app name header.
//   - Fingerprint icon and approval card with the hostname.
//   - Safe hostname, truthful request description, countdown, and actions.
//   - Countdown line — "Approve within Xs" (rendered only while the
//     state is `Counting`).
//   - `Approve` — full-width primary button on the theme background
//     (white-on-black, matching prrr's Connect button).
//   - `Deny`    — full-width outlined button below, error-tint text.
//   - Terminal status line — "Unlock approved." or "Denied: <reason>"
//     while the state is `Approved` or `Denied`.
//
// The screen is a pure function of `ApproveUiState`. All side-effects
// (start the countdown, dispatch Approve, dispatch Deny) flow through
// the `ApproveViewModel` injected as a parameter.
package com.sy.syauth.android.approve

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import kotlinx.coroutines.delay
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle

/**
 * Test tags attached to the major nodes so the androidTest harness can
 * find them without depending on the rendered strings (which are
 * locale- and config-dependent in future iterations).
 */
public object ApproveScreenTestTags {
    public const val LOGO: String = "syauth.approve.logo"
    public const val HEADER: String = "syauth.approve.header"
    public const val BIOMETRIC_ICON: String = "syauth.approve.biometric_icon"
    public const val HOSTNAME: String = "syauth.approve.hostname"
    public const val APPROVE_BUTTON: String = "syauth.approve.approve_button"
    public const val DENY_BUTTON: String = "syauth.approve.deny_button"
    public const val COUNTDOWN: String = "syauth.approve.countdown"
    public const val TERMINAL: String = "syauth.approve.terminal"
}

/** Maximum hostname length rendered. Excess is ellipsized. */
private const val HOSTNAME_MAX_CHARS: Int = 64

/** Screen padding. */
private val SCREEN_PADDING_DP = 24.dp

/** Vertical spacing between major sections. */
private val SECTION_SPACING_DP = 16.dp

/** Brand logo size at the top of the screen. */
private val BRAND_LOGO_SIZE_DP = 104.dp

/** Biometric icon size inside the approval card. */
private val BIOMETRIC_ICON_SIZE_DP = 72.dp

/** Vertical spacing between the Approve and Deny buttons. */
private val BUTTON_SPACING_DP = 12.dp

/**
 * Action-button height. Matches prrr-android's
 * MainScreen Connect button (56.dp) so the two siblings share the
 * primary-CTA proportions.
 */
private val BUTTON_HEIGHT_DP = 56.dp

/** Horizontal inset around the full-width action buttons. */
private val BUTTON_HORIZONTAL_PADDING_DP = 24.dp

/** Title surfaced in the TopAppBar. Centralised so tests can assert on it. */
public const val APPROVE_SCREEN_TITLE: String = "DeskUnlock"

/**
 * Dwell time on the `Approved` terminal state before the activity
 * dismisses itself. Long enough that the user sees "Unlock approved"
 * as confirmation, short enough that it does not feel like a stall.
 */
public const val APPROVED_DISMISS_DELAY_MILLIS: Long = 1_500L

/**
 * Dwell time on the `Denied` terminal state. Longer than the Approved
 * dwell so the user has time to read the reason ("timed out", "user
 * denied", etc.) before the activity dismisses.
 */
public const val DENIED_DISMISS_DELAY_MILLIS: Long = 2_500L

/**
 * Render the Approve screen for [viewModel]. The composable is a pure
 * function of `viewModel.uiState`; every action flows back through
 * `viewModel.onApproveClicked` / `viewModel.onDenyClicked`. When the
 * state reaches a terminal value (Approved / Denied), [onDismiss] is
 * invoked after a short confirmation dwell so the host activity can
 * `finish()` itself and return the user to the previous foreground.
 */
@Composable
public fun ApproveScreen(
    viewModel: ApproveViewModel,
    onDismiss: () -> Unit = {},
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()

    // Start the countdown exactly once when the screen enters
    // composition. `LaunchedEffect(Unit)` keys to the screen lifecycle.
    LaunchedEffect(Unit) {
        viewModel.start()
    }

    // Auto-dismiss on a terminal state. The delay gives the user time
    // to read the confirmation; once it elapses, [onDismiss] is
    // called exactly once per terminal transition. Keying on
    // `state::class` (not the whole instance) means a value change
    // inside the same terminal variant — none today — would not
    // restart the timer.
    LaunchedEffect(state::class) {
        when (state) {
            is ApproveUiState.Approved -> {
                delay(APPROVED_DISMISS_DELAY_MILLIS)
                onDismiss()
            }
            is ApproveUiState.Denied -> {
                delay(DENIED_DISMISS_DELAY_MILLIS)
                onDismiss()
            }
            else -> Unit
        }
    }

    val safeHostname = sanitizeHostname(viewModel.hostname)
    val approveEnabled = state is ApproveUiState.Counting

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
    ) { paddingValues ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
                .padding(horizontal = SCREEN_PADDING_DP),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Top,
        ) {
            Spacer(modifier = Modifier.height(SECTION_SPACING_DP))
            Image(
                painter = painterResource(com.sy.syauth.android.R.drawable.deskunlock_logo),
                contentDescription = androidx.compose.ui.res.stringResource(com.sy.syauth.android.R.string.approve_a11y_logo),
                modifier = Modifier
                    .size(BRAND_LOGO_SIZE_DP)
                    .semantics { testTag = ApproveScreenTestTags.LOGO },
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = APPROVE_SCREEN_TITLE,
                style = MaterialTheme.typography.headlineSmall,
                color = MaterialTheme.colorScheme.onBackground,
                modifier = Modifier.semantics { testTag = ApproveScreenTestTags.HEADER },
            )
            Spacer(modifier = Modifier.height(SECTION_SPACING_DP))
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surface,
                ),
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(SCREEN_PADDING_DP),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Icon(
                        imageVector = Icons.Filled.Fingerprint,
                        contentDescription = androidx.compose.ui.res.stringResource(com.sy.syauth.android.R.string.approve_a11y_biometric),
                        modifier = Modifier
                            .size(BIOMETRIC_ICON_SIZE_DP)
                            .semantics { testTag = ApproveScreenTestTags.BIOMETRIC_ICON },
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Spacer(modifier = Modifier.height(SECTION_SPACING_DP))
                    Text(
                        text = androidx.compose.ui.res.stringResource(com.sy.syauth.android.R.string.approve_title),
                        style = MaterialTheme.typography.headlineSmall,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = safeHostname,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        color = MaterialTheme.colorScheme.onSurface,
                        modifier = Modifier.semantics { testTag = ApproveScreenTestTags.HOSTNAME },
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = androidx.compose.ui.res.stringResource(com.sy.syauth.android.R.string.approve_subtitle),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(modifier = Modifier.height(SECTION_SPACING_DP))
                    CountdownRow(state)
                }
            }
            Spacer(modifier = Modifier.weight(1f))
            ButtonStack(
                approveEnabled = approveEnabled,
                onApprove = viewModel::onApproveClicked,
                onDeny = viewModel::onDenyClicked,
            )
            Spacer(modifier = Modifier.height(SECTION_SPACING_DP))
            TerminalMessage(state)
            Spacer(modifier = Modifier.height(SECTION_SPACING_DP))
        }
    }
}

@Composable
private fun CountdownRow(state: ApproveUiState) {
    val text = when (state) {
        is ApproveUiState.Counting -> androidx.compose.ui.res.stringResource(com.sy.syauth.android.R.string.approve_counting, state.remainingSeconds)
        ApproveUiState.AwaitingBiometric -> androidx.compose.ui.res.stringResource(com.sy.syauth.android.R.string.approve_waiting)
        ApproveUiState.Signing -> androidx.compose.ui.res.stringResource(com.sy.syauth.android.R.string.approve_signing)
        else -> ""
    }
    Text(
        text = text,
        style = MaterialTheme.typography.bodyLarge,
        modifier = Modifier.semantics { testTag = ApproveScreenTestTags.COUNTDOWN },
    )
}

@Composable
private fun ButtonStack(
    approveEnabled: Boolean,
    onApprove: () -> Unit,
    onDeny: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = BUTTON_HORIZONTAL_PADDING_DP),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Button(
            onClick = onApprove,
            enabled = approveEnabled,
            modifier = Modifier
                .fillMaxWidth()
                .height(BUTTON_HEIGHT_DP)
                .semantics { testTag = ApproveScreenTestTags.APPROVE_BUTTON },
            colors = ButtonDefaults.buttonColors(
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ),
        ) {
            Text(
                text = androidx.compose.ui.res.stringResource(com.sy.syauth.android.R.string.approve_button),
                style = MaterialTheme.typography.titleMedium,
            )
        }
        Spacer(modifier = Modifier.height(BUTTON_SPACING_DP))
        OutlinedButton(
            onClick = onDeny,
            enabled = approveEnabled,
            modifier = Modifier
                .fillMaxWidth()
                .height(BUTTON_HEIGHT_DP)
                .semantics { testTag = ApproveScreenTestTags.DENY_BUTTON },
            colors = ButtonDefaults.outlinedButtonColors(
                contentColor = MaterialTheme.colorScheme.error,
            ),
        ) {
            Text(
                text = "Deny",
                style = MaterialTheme.typography.titleMedium,
            )
        }
    }
}

@Composable
private fun TerminalMessage(state: ApproveUiState) {
    val text = when (state) {
        is ApproveUiState.Approved -> "Unlock approved"
        is ApproveUiState.Denied -> "Richiesta rifiutata: ${denialReasonLabel(state.reason)}"
        else -> ""
    }
    Text(
        text = text,
        style = MaterialTheme.typography.bodyMedium,
        modifier = Modifier.semantics { testTag = ApproveScreenTestTags.TERMINAL },
    )
}

private fun denialReasonLabel(reason: DenialReason): String = when (reason) {
    DenialReason.UserDenied -> "user denied"
    DenialReason.TimedOut -> "timed out"
    DenialReason.BiometricFailed -> "biometric failed"
    DenialReason.BiometricUnavailable -> "biometric unavailable"
    is DenialReason.SignError -> "sign error (${reason.reason})"
}

/**
 * Cap the hostname at [HOSTNAME_MAX_CHARS] characters and strip
 * newlines so an attacker-controlled name cannot inject a multi-line
 * phishing prompt. Defends T-014 (biometric coercion / phishing).
 */
internal fun sanitizeHostname(raw: String): String {
    val singleLine = raw.replace('\n', ' ').replace('\r', ' ')
    return if (singleLine.length > HOSTNAME_MAX_CHARS) {
        singleLine.substring(0, HOSTNAME_MAX_CHARS) + "…"
    } else {
        singleLine
    }
}
