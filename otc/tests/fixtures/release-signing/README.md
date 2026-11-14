# Test-only release signing fixtures

These files test `otc update`'s check of a release's signature
(otc/src/commands/update.rs). They are **not** olang's release key.

- `test-release.pub`: the public half of a throwaway ECDSA P-256 key,
  made for these tests only. Its private half was deleted once the two
  signatures below were made, so nothing can sign with it again.
- `SHA256SUMS`: a sample checksum list.
- `SHA256SUMS.sig`: its signature by the test key
  (`openssl dgst -sha256 -sign test.key -out SHA256SUMS.sig SHA256SUMS`).
- `SHA256SUMS.other-key.sig`: its signature by a second throwaway key,
  which must not verify with `test-release.pub`.

olang's real public key is pinned in `install.sh`, `install.ps1` and
`otc`'s `RELEASE_SIGNING_KEY` (dist/signing/README.md).
