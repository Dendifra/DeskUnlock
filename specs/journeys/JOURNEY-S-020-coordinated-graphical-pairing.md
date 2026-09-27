# JOURNEY-S-020 — Coordinated graphical pairing

## Scope

DeskUnlock pairing is usable from the Linux Qt window and Android Compose
screen. Existing LESC, public-key exchange, bond-key derivation, OOB words,
Keystore and biometric requirements remain unchanged.

## State machine

`DISCOVERY -> CAPABILITY_NEGOTIATED -> LESC_VERIFIED -> KEYS_EXCHANGED ->
OOB_PENDING -> OOB_CONFIRMED_LOCAL/REMOTE -> PREPARED -> COMMIT_PENDING ->
COMMITTED -> BONDED`.

A `REJECT`, `CANCEL`, `TIMEOUT` or `ERROR` before commit aborts staged state.
A disconnect before commit aborts. A disconnect after the commit decision is
`UNCERTAIN`; neither UI claims success or claims rollback without a durable
reconciliation result.

## Version negotiation

The versioned application transaction is v2. Its fixed control messages carry
only version, transaction id and operation. Unknown versions, malformed sizes,
wrong transaction ids, replayed operations and out-of-order operations are
rejected fail-closed. A legacy peer cannot complete the graphical flow.

## Completion definition

Pairing is complete only after both endpoints have:

1. passed LESC numeric comparison and independent OOB confirmation;
2. staged and read back the inactive bond record;
3. exchanged `COMMIT` and `COMMIT_ACK`;
4. persisted and read back the committed record; and
5. exchanged `COMMITTED`.

Only then may either UI render “Associato”. Existing bonds are retained until
replacement reaches this point.

## User-visible identity

The desktop stores the name supplied by the connected phone candidate. Android
reads the desktop hostname from authenticated versioned GATT metadata. Names
are display labels only; `peer_id` remains derived from the existing public key.
No vendor name, address or personal hostname is used to select a peer.

## Test matrix

The core transaction tests cover both confirms, reject/cancel/timeout/error,
pre/post-commit disconnects, missing and duplicate ACKs, wrong transaction ids,
legacy packets and no false `BONDED`. Desktop integration covers a Galaxy S26
name. Android tests cover arbitrary hostnames and atomic bond replacement.

## Physical validation

A real Android/Linux Bluetooth run is still required to validate OEM GATT,
LESC dialog timing and the final bilateral commit exchange; no device is
installed or modified by repository tests.
