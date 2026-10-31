# Release signing

Every release carries a `SHA256SUMS` file listing a checksum, a
fingerprint, for each download. A checksum only proves a file arrived
intact; it cannot prove who made it, because `SHA256SUMS` sits on the
same server as the files it describes. So the release workflow also
signs `SHA256SUMS` with a private key that lives only in a GitHub
secret, and the installers check that signature against the public key
written into `install.sh` and `install.ps1`. Someone who could swap the
files on the release page still could not produce a signature that
matches.

The key is an ECDSA P-256 key, because `openssl` on every Mac (LibreSSL)
and every Linux can check one, and Windows can check it with .NET. No
extra tool is needed on the user's side.

## Setting it up (once)

On your own computer, outside the repository:

```bash
# 1. the key pair
openssl ecparam -name prime256v1 -genkey -noout -out olang-release.key
openssl ec -in olang-release.key -pubout -out olang-release.pub

# 2. the private half becomes a repository secret
gh secret set OLANG_SIGNING_KEY --repo ooyeku/olang < olang-release.key
```

Keep `olang-release.key` somewhere safe and offline (a password manager
is fine), then delete the copy on disk. Anyone holding it can sign
releases.

Then, in one commit, before the first signed release is tagged:

1. Copy `olang-release.pub` to `dist/signing/olang-release.pub`.
2. Paste its contents into `install.sh` as `SIGNING_KEY` (the whole PEM,
   `-----BEGIN PUBLIC KEY-----` through `-----END PUBLIC KEY-----`,
   inside the double quotes) and into `install.ps1` as `$SigningKey`.
3. Set `SIGNED_FROM` in `install.sh` and `$SignedFrom` in `install.ps1`
   to the tag of that first signed release, for example `v1.0.0`.
   Releases older than it install with a note that only their checksum
   was checked; releases from it on must carry a good signature.

`tests/install_script_test.rs` checks that the three copies of the key
agree. The release workflow refuses to publish when `install.sh` carries
a key but the secret is missing, or when the signature it made does not
verify against `install.sh`'s key.

## What each release then carries

- `SHA256SUMS`, the checksums of every download;
- `SHA256SUMS.sig`, a DER-encoded ECDSA signature of it;
- a GitHub build provenance attestation for each download, which
  records the workflow run that built it.

To check a download by hand:

```bash
openssl dgst -sha256 -verify olang-release.pub -signature SHA256SUMS.sig SHA256SUMS
shasum -a 256 -c SHA256SUMS --ignore-missing
gh attestation verify olang-1.0.0-macos-arm64.tar.gz --repo ooyeku/olang
```

## Rotating the key

Make a new pair as above, replace the secret, the public key file and
both installers' copies in one commit, and set `SIGNED_FROM` to the next
release. Older releases signed with the old key then fail the check,
so also re-run the release workflow for any release people still
install by version, which re-signs it with the new key.

## macOS notarisation (optional)

The macOS downloads are ad-hoc signed. That is enough for the installer,
because `curl` does not mark files as coming from the internet, but a
tarball downloaded with a browser is quarantined, and Gatekeeper refuses
to open the programs until the user clears the mark. Notarised programs
open normally.

Notarisation needs a paid Apple Developer account. With one, add these
secrets and the release workflow signs both programs with your
Developer ID and the hardened runtime (`dist/macos/entitlements.plist`
allows the JIT) and has Apple notarise them. Without them it skips the
step.

| Secret | What it holds |
|---|---|
| `APPLE_DEVELOPER_ID_P12` | your "Developer ID Application" certificate and key, exported from Keychain Access as a .p12, then `base64 -i cert.p12` |
| `APPLE_DEVELOPER_ID_P12_PASSWORD` | the password you gave that .p12 |
| `APPLE_NOTARY_KEY` | an App Store Connect API key (.p8, from Users and Access, Integrations), as `base64 -i AuthKey_XXXX.p8` |
| `APPLE_NOTARY_KEY_ID` | that key's ID |
| `APPLE_NOTARY_ISSUER_ID` | the issuer ID shown on the same page |

Command-line programs cannot have the notarisation ticket stapled to
them, so a Mac checks with Apple the first time it opens a quarantined
copy, which needs a network connection.
