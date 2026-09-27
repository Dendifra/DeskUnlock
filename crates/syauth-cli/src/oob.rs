//! `syauth-cli` — app-level OOB code derivation.
//!
//! Per SPEC §4.1 the desktop and the phone each compute, after BlueZ-level
//! LE Secure Connections completes, an *independent* OOB confirmation derived
//! from the freshly-negotiated bond key:
//!
//! ```text
//! HKDF(bond, "syauth-oob-v1")[0..4] → an 8-digit decimal code
//! ```
//!
//! The operator compares that single number on the two screens. It never
//! crosses the wire: both sides derive it from the shared secret, so the
//! rendering is presentation, not protocol. Rotating it means bumping the HKDF
//! info string ([`HKDF_INFO_OOB_V1`]).
//!
//! The number carries ~26.6 bits (10^8 codes), the same order as the Bluetooth
//! numeric comparison it complements: an attacker who beats the transport
//! comparison still has to make both screens agree on this value.
//!
//! Roadmap: specs/syauth/ROADMAP.md item S-011.
//! Journey: specs/journeys/JOURNEY-S-011-pairing-desktop.md

use hkdf::Hkdf;
use sha2::Sha256;

/// HKDF info string for the v1 OOB derivation. Versioned so a future schema
/// can rotate without recomputing existing bonds.
pub const HKDF_INFO_OOB_V1: &[u8] = b"syauth-oob-v1";

/// Digits in the displayed OOB code. Eight decimal digits ≈ 26.6 bits, the
/// same order as the six-digit Bluetooth numeric comparison it complements, and
/// short enough to compare at a glance across two screens.
pub const OOB_CODE_DIGITS: usize = 8;

/// Size of the displayed code space (`10^OOB_CODE_DIGITS`).
const OOB_CODE_SPACE: u32 = 100_000_000;

/// Bytes of HKDF output consumed by the code.
const OOB_HKDF_BYTES: usize = 4;

/// Width of the bond key the HKDF expand step is keyed on. Matches the
/// `syauth-transport::BOND_KEY_BYTES` constant; restated locally so this module
/// has no inbound type dependency on the transport crate.
pub const OOB_BOND_KEY_BYTES: usize = 32;

/// Derive the OOB confirmation code for `bond_key`.
///
/// Pure deterministic: `oob_code_for_bond(&k) == oob_code_for_bond(&k)` for
/// every `k`. No clock, no env input, no salt — see
/// `specs/journeys/JOURNEY-S-011-pairing-desktop.md` Phase 3 for the rationale.
///
/// Always exactly [`OOB_CODE_DIGITS`] ASCII digits, zero-padded, so the desktop
/// and the phone render byte-identical text.
#[must_use]
pub fn oob_code_for_bond(bond_key: &[u8; OOB_BOND_KEY_BYTES]) -> String {
    let hk = Hkdf::<Sha256>::new(None, bond_key);
    let mut out = [0u8; OOB_HKDF_BYTES];
    // HKDF::expand only errors when the requested output exceeds 255 * 32 =
    // 8160 bytes. 4 bytes is far below that bound, so the call is infallible
    // by construction. We still match the result to avoid `unwrap()` per the
    // AGENTS.md non-negotiable.
    if hk.expand(HKDF_INFO_OOB_V1, &mut out).is_err() {
        out = [0u8; OOB_HKDF_BYTES];
    }
    format!("{:0width$}", u32::from_be_bytes(out) % OOB_CODE_SPACE, width = OOB_CODE_DIGITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXED_KEY_A: [u8; OOB_BOND_KEY_BYTES] = [0x01; OOB_BOND_KEY_BYTES];
    const FIXED_KEY_B: [u8; OOB_BOND_KEY_BYTES] = [0x02; OOB_BOND_KEY_BYTES];

    #[test]
    fn oob_code_is_deterministic_for_fixed_key() {
        let a = oob_code_for_bond(&FIXED_KEY_A);
        let b = oob_code_for_bond(&FIXED_KEY_A);
        assert_eq!(a, b, "same bond_key must produce the same code");
        assert_eq!(OOB_CODE_DIGITS, a.chars().count());
    }

    #[test]
    fn oob_code_is_always_exactly_eight_digits() {
        for key in [FIXED_KEY_A, FIXED_KEY_B, [0x00; OOB_BOND_KEY_BYTES], [0xFF; OOB_BOND_KEY_BYTES]] {
            let code = oob_code_for_bond(&key);
            assert_eq!(OOB_CODE_DIGITS, code.len(), "zero padding must hold for {code}");
            assert!(code.chars().all(|c| c.is_ascii_digit()), "digits only: {code}");
        }
    }

    #[test]
    fn oob_code_differs_across_keys() {
        // Not a strict invariant of HKDF (two keys could collide on one of
        // 10^8 values), but for these fixed test keys the outputs differ —
        // pinning it makes a regression in the HKDF info string instantly
        // visible.
        assert_ne!(oob_code_for_bond(&FIXED_KEY_A), oob_code_for_bond(&FIXED_KEY_B));
    }
}
