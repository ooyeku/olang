// ═══════════════════════════════════════════════════════════════════
// 04 — Standard Library Showcase
// Real tasks across the batteries-included stdlib: cryptography,
// math, dates, and JSON. Every result below is computed, not faked.
// ═══════════════════════════════════════════════════════════════════

println("═══ stdlib showcase ═══")

// ── crypto: hashing ─────────────────────────────────────────────────
println("── hashing ──")
println(`sha256("olang") = ${crypto.sha256("olang")}`)
println(`md5("olang")    = ${crypto.md5("olang")}`)
let mac = crypto.hmac_sha256("payload", "secret-key")
println(`hmac_sha256     = ${mac}`)

// ── crypto: password hashing (bcrypt) ───────────────────────────────
println("── password hashing ──")
let hashed = unwrap(crypto.hash_password("hunter2"))
println(`stored hash length: ${len(hashed)}`)
println(`verify correct:   ${unwrap(crypto.verify_password("hunter2", hashed))}`)
println(`verify wrong:     ${unwrap(crypto.verify_password("wrong", hashed))}`)

// ── crypto: RSA digital signatures ──────────────────────────────────
// Generate a keypair, sign a message with the private key, and verify
// with the public key. Tampering breaks verification.
println("── digital signatures ──")
let keys = unwrap(crypto.generate_key_pair())
let message = "transfer $100 to account 4021"
let signature = unwrap(crypto.sign_data(message, keys.private_key))
println(`message signed (${len(signature)} hex chars)`)
println(`genuine message verifies:  ${unwrap(crypto.verify_signature(message, signature, keys.public_key))}`)
println(`tampered message verifies:  ${unwrap(crypto.verify_signature("transfer $9000 to account 4021", signature, keys.public_key))}`)

// ── math: real numeric work ─────────────────────────────────────────
println("── math ──")
println(`pi = ${math.PI}, e = ${math.E}`)
println(`sqrt(2) = ${math.sqrt(2.0)}`)
println(`2^16    = ${math.pow(2.0, 16.0)}`)
println(`gcd(462, 1071) = ${math.gcd(462, 1071)}`)
println(`lcm(4, 6)      = ${math.lcm(4, 6)}`)
println(`12! = ${math.factorial(12)}`)

// Compute the standard deviation of a sample the honest way.
fn stddev(xs) = {
    let n = to_float(len(xs))
    let mean = (xs |> fold(0.0, (a, x) => a + x)) / n
    let variance = (xs |> fold(0.0, (a, x) => a + (x - mean) * (x - mean))) / n
    math.sqrt(variance)
}
let sample = [4.0, 8.0, 15.0, 16.0, 23.0, 42.0]
println(`stddev of sample = ${stddev(sample)}`)

// Trig identity check: sin^2 + cos^2 = 1
let angle = 0.7
let identity = math.pow(math.sin(angle), 2.0) + math.pow(math.cos(angle), 2.0)
println(`sin^2(0.7) + cos^2(0.7) = ${identity}`)

// ── dates: calendar arithmetic ──────────────────────────────────────
println("── dates ──")
let launch = "2026-08-05"
println(`launch date:       ${launch}`)
println(`+ 90 days:         ${unwrap(dates.add_days(launch, 90))}`)
println(`+ 2 months:        ${unwrap(dates.add_months(launch, 2))}`)
println(`weekday (0=Sun):   ${unwrap(dates.weekday(launch))}`)
println(`days since 2026-01-01: ${unwrap(dates.diff_days(launch, "2026-01-01"))}`)
println(`2024 a leap year?  ${dates.is_leap_year(2024)}`)
println(`2026 a leap year?  ${dates.is_leap_year(2026)}`)
println(`days in Feb 2024:  ${dates.days_in_month(2024, 2)}`)

// ── json: parse, query, transform ───────────────────────────────────
// The json module manipulates JSON *text*: parse validates, get/set
// query and update, prettify formats.
println("── json ──")
let doc = "{\"user\": \"ann\", \"score\": 42, \"tags\": [\"pro\", \"beta\"]}"
println(`valid json?  ${json.validate(doc)}`)
println(`user field:  ${unwrap(json.get(doc, "user"))}`)
println(`score field: ${unwrap(json.get(doc, "score"))}`)
println(`keys:        ${unwrap(json.get_keys(doc))}`)
let bumped = unwrap(json.set(doc, "score", "43"))
println(`after set:   ${bumped}`)

println("═══ showcase complete ═══")
