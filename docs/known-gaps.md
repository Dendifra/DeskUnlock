# syauth — Known SPEC Deviations (public audit trail)

This file records security- and correctness-relevant deviations from
`specs/syauth/SPEC.md` without publishing operator-specific identifiers,
device serials, local usernames, filesystem paths, private chat quotations, or
other machine-specific evidence.

A public deviation row must contain the SPEC clause, shipped behaviour, source
locations, approval status, closure condition, and privacy-safe verification
summary. Raw hardware logs and verbatim operator conversations do **not** belong
in the public repository.

## Open deviations

### `DEV-007` — bond key on disk instead of the kernel keyring

**SPEC clause:** §3.2 D6 requires Linux bond-key storage through the kernel
keyring with a `libsecret` fallback, with keys kept out of ordinary plaintext
files.

**Shipped behaviour:** the daemon currently stores the 32-byte bond key under
`/var/lib/syauth/keys/<peer_id>.bin`, mode `0600`, inside a directory protected
with mode `0700`. `syauth_core::KeyStore` already contains the keyring/libsecret
abstraction but the production unlock path does not yet use it.

**Source locations:**
- `crates/syauth-core/src/pair_recovery.rs`
- `crates/syauth-core/src/secrets.rs`

**Approval:** explicitly approved as a documented temporary deviation. The
public audit trail intentionally records the approval outcome, not a verbatim
private conversation.

**Status:** open.

**Closure condition:** production pair/recovery/unlock paths load and store the
bond key through `syauth_core::KeyStore`; ordinary per-peer key files are no
longer the active secret store; restart/recovery tests prove the selected secure
backend survives the supported lifecycle.

---

### `DEV-006` — `pam_syauth` in the login greeter uses `sufficient`

**SPEC clause:** §3.2 D7 prefers `auth required` for the authentication module
while preserving a documented password fallback.

**Shipped behaviour:** `scripts/enable-greeter-unlock.sh` installs
`auth sufficient pam_syauth.so timeout=8000` before the stock
`auth include system-login` entry for the Plasma login greeter. A successful
phone approval can therefore short-circuit the stack; failure continues to the
password path.

**Source locations:**
- `scripts/enable-greeter-unlock.sh`
- `crates/syauth-cli/src/install_pam.rs`

**Approval:** explicitly approved to preserve the existing manual-login
fallback while enabling phone approval at the greeter. No local account name or
verbatim private conversation is retained here.

**Status:** open.

**Closure condition:** a hermetic or documented hardware test proves all three
behaviours: phone approval grants login, absent/denied phone falls through to the
password path within the timeout, and the behaviour remains correct after
revoke + re-pair. The operator then either keeps the deviation with that evidence
or reverts to the SPEC control flag.

---

## Closed deviations

### `DEV-003` — BLE role direction

**SPEC clause:** §3.2 D8 requires the desktop to advertise a rotating
session-bound UUID while the phone scans and connects.

**Resolution:** pair and unlock channels now use the same direction: desktop
advertises; Android discovers/selects the desktop and opens the GATT client.
Legacy phone-advertising/server paths were removed.

**Primary evidence:**
- `crates/syauth-cli/src/pair_backend.rs`
- `crates/syauth-transport/src/bluez_advertise.rs`
- `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/impl/RealPairBackend.kt`
- `syauth-android/app/src/main/AndroidManifest.xml`
- `specs/journeys/JOURNEY-DEV-003-invert-advertising.md`

**Status:** closed 2026-05-17. Verification covered pair-direction consistency,
BLE permission cleanup, scope-discipline checks, Rust tests and Android build /
unit-test gates. Hardware evidence is referenced in privacy-safe form only.

---

### `DEV-002` — Android signing key moved to Keystore-backed signing

**SPEC clause:** §3.2 D6 requires the Android signing key to remain in Android
Keystore, prefer StrongBox when supported, and require fresh user
authentication before signing.

**Resolution:** the production signing path uses the Keystore-backed
`FrameSigner` callback; the Ed25519 private key is not serialized into the bond
record and does not cross the JVM↔Rust boundary as raw key bytes. StrongBox
fallback and key-generation error paths are typed and tested.

**Primary evidence:**
- `crates/syauth-mobile/src/mobile.udl`
- `crates/syauth-mobile/src/implementation.rs`
- `syauth-android/app/src/main/kotlin/com/sy/syauth/android/approve/KeystoreFrameSigner.kt`
- `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/impl/KeystoreKeyGenerator.kt`
- `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/impl/RealPairBackend.kt`
- `specs/journeys/JOURNEY-DEV-002-keystore-strongbox.md`

**Status:** closed 2026-05-17. Rust, Android build, Android unit tests and
schema/grep checks verified that private signing material is not written into
the public bond schema.

---

### `DEV-001` — real LESC pairing replaces the provision-file shortcut

**SPEC clause:** §3.2 D5 requires LE Secure Connections numeric comparison plus
an application-level out-of-band confirmation; the v0.1.0 scope requires a real
pairing flow rather than a provision-file shortcut.

**Resolution:** the desktop advertises pair mode and runs the BlueZ confirmation
agent; Android uses `CompanionDeviceManager`, opens a GATT connection to the
selected desktop, verifies the pairing variant, waits for the real bond state,
performs the post-bond public-key exchange and derives the shared bond key. The
stub/provision path is no longer the production implementation.

**Primary evidence:**
- `crates/syauth-cli/src/pair_backend.rs`
- `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/impl/RealPairBackend.kt`
- `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/impl/AndroidCdmPairCompanionScanner.kt`
- `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/impl/PairingBroadcastReceiver.kt`
- `specs/journeys/JOURNEY-DEV-001-real-lesc.md`

**Status:** closed 2026-05-17. The closure was exercised against real hardware,
but device serials, local hostnames, peer identifiers, Keystore aliases and raw
operator logs are intentionally omitted from this public summary.

---

### `DEV-004` — authenticated encryption required on unlock GATT characteristics

**SPEC / threat-model basis:** the BLE unlock channel must not expose challenge
or response bytes to an unauthenticated/non-bonded peer.

**Resolution:** the desktop GATT application uses
`encrypt_authenticated_read` for the challenge characteristic and
`encrypt_authenticated_write` for the response characteristic. BlueZ therefore
rejects unauthenticated access before bytes reach the application layer.

**Primary evidence:**
- `crates/syauth-transport/src/bluez_advertise.rs`
- `crates/syauth-transport/tests/dev004_link_encryption.rs`
- `specs/journeys/JOURNEY-DEV-004-link-encryption.md`

**Status:** closed 2026-05-17 for the structural requirement. Radio-backed test
cases remain explicitly gated by the project's real-radio test switch.

---

## How to add a row

1. Re-read `AGENTS.md` → **Scope Discipline (Non-Negotiable)**.
2. Obtain explicit approval before shipping a SPEC deviation.
3. Record the approval outcome or a non-sensitive issue/PR reference. **Do not
   paste verbatim private conversations, local usernames, device serials,
   hostnames, MAC addresses, peer IDs, Keystore aliases, or machine-specific
   filesystem paths into the public repository.**
4. Assign the next monotonic `DEV-NNN` id.
5. Fill: SPEC clause, shipped behaviour, source locations, approval status,
   status, closure condition, and privacy-safe evidence summary.
6. Add `// SPEC-DEVIATION: DEV-NNN — <reason> — see docs/known-gaps.md` at the
   affected source locations.
7. Run `make scope-discipline`, `bash scripts/privacy-check.sh`, and the deep
   public-repository audit before merge/release.
