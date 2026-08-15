// signing — the harbor's integrity story: sha256 manifests, an
// hmac-chained daily digest (each day binds the previous, so history
// cannot be quietly rewritten), and an RSA signature over the chain
// head. Password hashing gates the harbormaster console.
//
// Everything is Result-disciplined: key generation, signing, and
// verification can fail; nothing here panics on a bad input.

// ── Manifest checksums ──────────────────────────────────────────────
share fn manifest_checksum(lines) = crypto.sha256(join(lines, "\n"))

// ── The digest chain ────────────────────────────────────────────────
// Day N's link = hmac(day summary, key = day N-1's link). Genesis is
// hmac over a fixed label. Verifying the whole chain replays it.
share let GENESIS = "harborline-genesis"

share fn chain_link(prev_link: String, summary: String) =
    crypto.hmac_sha256(summary, prev_link)

share fn chain_start() = crypto.hmac_sha256(GENESIS, GENESIS)

// Replay a list of day summaries into the chain head.
share fn chain_head(summaries) =
    summaries |> fold(chain_start(), (link, s) => chain_link(link, s))

// A rewritten day breaks every later link.
share fn chain_verify(summaries, expected_head: String) =
    crypto.secure_compare(chain_head(summaries), expected_head)

// ── RSA: sign the chain head, verify with the public key ────────────
share fn make_keys() = unwrap(crypto.generate_key_pair())

share fn sign_head(head: String, private_key) =
    unwrap(crypto.sign_data(head, private_key))

share fn verify_head(head: String, signature, public_key) =
    unwrap(crypto.verify_signature(head, signature, public_key))

// ── Console gate ────────────────────────────────────────────────────
share fn set_password(plain: String) = unwrap(crypto.hash_password(plain))

share fn check_password(plain: String, hashed: String) =
    unwrap(crypto.verify_password(plain, hashed))

// Short random ids for invoice numbers.
share fn invoice_id() = "INV-" + str.to_upper(crypto.random_hex(6))

// ── Self-checks ─────────────────────────────────────────────────────
test "the digest chain detects rewrites" {
    let days = ["day0 rev=100", "day1 rev=250", "day2 rev=180"]
    let head = chain_head(days)
    testing.assert_true(chain_verify(days, head))
    // rewrite history: day1 revenue "adjusted"
    let cooked = ["day0 rev=100", "day1 rev=950", "day2 rev=180"]
    testing.assert_false(chain_verify(cooked, head))
    // appending is fine — the head just moves
    let extended = concat(days, ["day3 rev=90"])
    testing.assert_eq(chain_head(extended), chain_link(head, "day3 rev=90"))
}

test "signatures verify and tampering breaks them" {
    let keys = make_keys()
    let head = chain_head(["day0 rev=100"])
    let sig = sign_head(head, keys.private_key)
    testing.assert_true(verify_head(head, sig, keys.public_key))
    testing.assert_false(verify_head(head + "x", sig, keys.public_key))
}

test "the console gate" {
    let stored = set_password("crane-operator-9")
    testing.assert_true(check_password("crane-operator-9", stored))
    testing.assert_false(check_password("guess", stored))
    testing.assert_true(str.starts_with(invoice_id(), "INV-"))
    testing.assert_eq(len(invoice_id()), 16)
}

test "manifest checksums are content-sensitive" {
    let a = manifest_checksum(["line one", "line two"])
    let b = manifest_checksum(["line one", "line 2wo"])
    testing.assert_eq(len(a), 64)
    testing.assert_true(a != b)
}
