# DeskUnlock publication audit

Generated locally. This report intentionally avoids printing secret values.

- [x] PASS: MIT license file present
- [x] PASS: upstream syauth copyright notice preserved
- [x] PASS: README.md present
- [x] PASS: NOTICE.md present
- [x] PASS: SECURITY.md present
- [x] PASS: CONTRIBUTING.md present
- [x] PASS: THIRD_PARTY_LICENSES.md present
- [x] PASS: no known developer username/home path detected
- [ ] WARN: possible fixed UID 1000 assumption detected; review matches
- [ ] WARN: MAC-address-shaped literal detected; review test fixtures vs private device IDs
- [x] PASS: no private-key PEM marker detected
- [x] PASS: no obvious literal secret assignment detected
- [x] PASS: no common sensitive filenames found
- [x] PASS: no unexpected files larger than 10 MiB
- [ ] WARN: cargo-deny not installed; Rust dependency license audit still required
- [ ] WARN: Git worktree has uncommitted/untracked changes
- [ ] WARN: syauth identifiers remain; classify each as upstream attribution, compatibility identifier, or rename target
- [ ] WARN: desktop-specific integration references found; make support scope explicit

## Result

- Failures: 0
- Warnings: 6

**Audit gate:** no hard failures. Warnings still require review before publication.
