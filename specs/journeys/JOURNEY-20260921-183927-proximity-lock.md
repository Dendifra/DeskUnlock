# JOURNEY-20260921-183927: Proximity Lock e baseline locale

## Roadmap Link
- Source roadmap: `specs/unlock-proximity/ROADMAP.md`
- Feature: RSSI proximity state machine, proximity lock and DeskUnlock diagnostics

## 1. Journey

When a DeskUnlock user has one paired phone and an active desktop session, I want the computer to learn the nearby RSSI baseline and lock after a sustained departure, so I can leave the workstation without exposing the session.

## 2. CJM

The user should get safe proximity locking without tuning dBm values. The normal GUI exposes only the meaningful proximity state and three sensitivity profiles; advanced diagnostics are available when troubleshooting. Manual locks remain distinguishable from proximity locks, so an explicit lock-screen interaction is never replaced by an automatic return authentication.

### Phase 1: Configure proximity protection

**User Intent:** Enable proximity locking and choose a sensitivity profile.

**Actions:** The user opens DeskUnlock, enables Proximity Lock, and optionally selects Vicino, Bilanciato or Ampio.

**Pain / Risk:** The service may be unavailable; a profile change may overwrite the learned baseline; a normal screen may expose raw location telemetry.

**Success Signal:** The switch and profile are persisted, the default is Bilanciato, and normal status shows only a coarse proximity state.

### Phase 2: Learn and monitor presence

**User Intent:** Use the paired phone normally without manual calibration.

**Actions:** The service consumes fresh filtered RSSI samples, learns a multi-sample local baseline, applies hysteresis, and uses the existing heartbeat as a conservative fallback.

**Pain / Risk:** One weak RSSI sample could cause a lock; stale RSSI could be mistaken for departure; pairing a different phone could reuse the old baseline.

**Success Signal:** The state moves through NEAR, MID, FAR and ABSENT only under the configured persistence rules; stale RSSI alone does not lock; pair, change, dissociation and reset lifecycle events invalidate the baseline without persisting phone identity.

### Phase 3: Lock and return

**User Intent:** Leave and return to the workstation safely.

**Actions:** A sustained FAR state or a genuinely lost heartbeat requests one lock with reason PROXIMITY. On a stable return, with a valid heartbeat and challenge-ready marker, one return authentication request is sent.

**Pain / Risk:** A manual lock could be mistaken for a proximity lock; return authentication could retry after denial or timeout; a service restart could invent a lock from stale state.

**Success Signal:** The runtime reason is PROXIMITY or MANUAL_OR_OTHER, the return request is one-shot, and restart or cancellation preserves the existing security behavior.

### Friction and Opportunity

| Friction | Phase | Opportunity |
|----------|-------|-------------|
| dBm values are difficult to interpret | Configure | Use coarse state labels and named profiles |
| Bluetooth telemetry can be stale | Monitor | Require fresh RSSI and retain the last safe state while heartbeat is valid |
| Automatic and manual locks look alike | Lock and return | Persist an explicit runtime lock reason |
| Diagnostics can leak proximity data | All | Keep raw RSSI in runtime diagnostics only and hide it from normal UI |

### North Star Summary

A paired user enables one switch and receives safe proximity locking with no calibration work. The system learns locally, resists RSSI noise, falls back only when the presence heartbeat is genuinely absent, and performs exactly one authenticated return attempt only for a lock it caused.

## 3. UX Implementation and Assessment

### Time to First Value
- [x] The default profile works without manual thresholds.
- [x] Multiple samples establish a local baseline before departure decisions.

### Onboarding Clarity
- [x] Proximity Lock is a named GUI section.
- [x] The three profiles use user-facing Italian labels.

### Production-Ready Defaults
- [x] Bilanciato is the default profile.
- [x] A stale RSSI sample does not create a FAR transition.

### Golden Path Quality
- [x] Stable FAR requests one lock.
- [x] Stable return sends one authentication request.

### Decision Load
- [x] The normal UI exposes one switch and one profile choice.
- [x] Raw dBm values are excluded from the normal view.

### Progressive Complexity
- [x] Advanced diagnostics are opt-in.
- [x] Normal state remains coarse and readable.

### Error Quality
- [x] Diagnostics reports sample age, heartbeat age and lock reason.
- [x] Failed return requests do not retry silently.

### Failure Safety
- [x] Manual locks do not trigger proximity return authentication.
- [x] Runtime state is atomic and private.

### Runtime Transparency
- [x] The status command exposes state, profile and baseline readiness.
- [x] The advanced command exposes the reason for a lock.

### Debuggability
- [x] Raw and filtered RSSI are available only through diagnostics.
- [x] The regression harness exercises deterministic timestamps and markers.

### Cross-Surface Consistency
- [x] The GUI, status command and engine use the same NEAR/MID/FAR/ABSENT vocabulary.
- [x] Existing DMS and Android cancellation paths remain covered.

### Workflow Consistency
- [x] The existing user service owns the proximity watcher.
- [x] The existing runtime directory is used for volatile state.

### Change Safety
- [x] Profile and baseline writes use private atomic files.
- [x] Pair, change and dissociation call proximity reset without persisting phone identity.

### Experimentation Safety
- [x] `syauth-proximity reset` removes only learned proximity state.
- [x] Diagnostics does not alter runtime state.

### Interaction Latency
- [x] The watcher evaluates once per second.
- [x] Hysteresis makes transitions explicit rather than reacting to one sample.

### Developer Feedback Speed
- [x] The shell regression harness runs without Bluetooth hardware.
- [x] Failure cases report the named assertion.

### Team Scale
- [x] Configuration keys are stable and limited to proximity concerns.
- [x] Runtime keys are validated by regression tests.

### System Scale
- [x] The engine uses the existing single active peer contract.
- [x] Profile thresholds are centralized in one function.

### Right Behavior by Default
- [x] The default profile is Bilanciato.
- [x] No lock is inferred from a stale runtime FAR state after restart.

### Anti-Bypass Design
- [x] Lock reason is set only after a successful lock request.
- [x] One-shot return state is persisted before the request is dispatched.

## 4. Tests

### TC-01: multi-sample baseline

**Given** three fresh near samples.
**When** the engine processes them.
**Then** it persists one local baseline and no raw RSSI in configuration.

### TC-02: RSSI hysteresis

**Given** a learned baseline.
**When** one weak sample or a short MID excursion arrives.
**Then** the state does not immediately become FAR or return to NEAR.

### TC-03: sustained FAR lock

**Given** fresh heartbeat and RSSI below the profile FAR threshold.
**When** FAR persists for the profile duration.
**Then** the session receives one lock request and the reason is PROXIMITY.

### TC-04: stale RSSI safety

**Given** a valid heartbeat and no fresh RSSI sample.
**When** the watcher evaluates the session.
**Then** it does not invent a FAR transition or lock.

### TC-05: heartbeat fallback

**Given** a previously observed phone and an expired heartbeat window.
**When** the watcher evaluates the session.
**Then** the state becomes ABSENT and one fallback lock is requested.

### TC-06: manual lock separation

**Given** a locked session without a proximity lock request.
**When** the watcher observes the lock.
**Then** the reason is MANUAL_OR_OTHER and no automatic return request is sent.

### TC-07: one-shot return

**Given** a PROXIMITY lock, a stable NEAR return, a valid heartbeat and challenge-ready marker.
**When** the state becomes NEAR.
**Then** exactly one `phoneReturned` request is dispatched.

### TC-08: denied or timed-out return

**Given** a PROXIMITY lock and a failed return request.
**When** further near samples arrive.
**Then** no automatic retry is dispatched.

### TC-09: restart safety

**Given** volatile FAR state without a completed lock request.
**When** the proximity service initializes.
**Then** it does not invent a lock from that state.

### TC-10: privacy and diagnostics

**Given** a configured profile and runtime RSSI sample.
**When** normal status and advanced diagnostics are invoked.
**Then** normal status omits dBm values and diagnostics exposes them without writing them to persistent configuration.

## Traceability
- Roadmap item: user-requested Phase 2/3 proximity deliverable; related roadmap: `specs/unlock-proximity/ROADMAP.md`
- Implementation files: `desktop/bin/syauth-proximity`, `desktop/bin/syauth-settings`, `docs/installation.md`
- Test files: `tests/proximity_engine.sh`
