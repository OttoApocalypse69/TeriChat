# Historical evidence: MLS foundation slice only

Everything in this folder was recorded for the first #6 slice (the
`mls-foundation` feature) at commit `5cf0a590c0298be198eee7095053f0f736d150fb`
on `feat/6-mls-foundation`. `manifest.json`, `HANDOFF.md`, the logs and their
source hashes describe that commit only. `finalize.py` asserts that head, so it
fails on any later revision. That failure is expected; it does not mean the
current code is broken.

None of it covers later work: leaf rotation, the `mls-persist` encrypted
storage (`src/mls_persist.rs`, `tests/mls_persist.rs`), or anything after
them. The verification record for that code is the acceptance table in the
pull request that landed it (#68) and that PR's CI runs. Do not cite this
folder as evidence for the persistence implementation or for any production
E2EE claim.
