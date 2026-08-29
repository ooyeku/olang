// vessels — who arrives at the harbor: generation, validation, naming.
//
// Vessels are structs with a trait for presentation; callsigns are
// validated with regex; report filenames come from a slugifier. The
// generator is driven entirely by the caller's seeded `random` stream,
// so a whole voyage of arrivals is reproducible from one --seed.

use prelude { clamp }
use cargo { Cargo, Container, Bulk, Reefer, Hazmat, describe_cargo }

// ── The vessel record ───────────────────────────────────────────────
share type Vessel = struct {
    name: String,
    callsign: String,
    draft: Float,       // metres of water she needs
    hold: [Cargo],      // what she carries
    eta_tick: Int       // when she appeared in the roads
}

// A trait gives every vessel a uniform presentation surface; `loud`
// shows a default method calling back into the required one.
share trait Hailing {
    fn hail(self) -> String
    fn loud_hail(self) -> String = str.to_upper(self.hail())
}

impl Hailing for Vessel {
    fn hail(self) = `${self.name} [${self.callsign}] draft ${self.draft}m`
}

share fn hail_vessel(v) = v.hail()
share fn loud_hail_vessel(v) = v.loud_hail()

// ── Validation: a callsign is four letters, a dash, three digits ────
share fn valid_callsign(s) = unwrap(re.is_match("^[A-Z]{4}-\\d{3}$", s))

// Slugify a vessel name for filenames: "MV Iron Duke" -> "mv-iron-duke".
share fn slugify(title) = {
    let lower = str.to_lower(str.trim(title))
    let hyphenated = unwrap(re.replace_all("[^a-z0-9]+", lower, "-"))
    unwrap(re.replace_all("^-|-$", hyphenated, ""))
}

// ── Generation from the seeded stream ───────────────────────────────
let PREFIXES = ["MV", "SS", "MS"]
let NAMES = ["Iron Duke", "Meridian Star", "Cormorant", "Halcyon", "Tern",
    "Boreal Wind", "Ardent", "Pelagic Dawn", "Kestrel", "Long Reach",
    "Squall Line", "Fair Isle", "Petrel", "Windward", "Amber Wake"]

fn letters(n) = {
    let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    range(0, n) |> fold("", (acc, i) =>
        acc + str.char_at(alphabet, random.randint(0, 25)))
}

fn random_cargo() = {
    let roll = random.randint(0, 9)
    if roll < 4 => Container(random.randint(8, 60))
    else => if roll < 7 => Bulk(to_float(random.randint(40, 400)))
    else => if roll < 9 => Reefer(random.randint(4, 24))
    else => Hazmat(random.randint(1, 9), to_float(random.randint(20, 120)))
}

// A fresh arrival. Hold size scales loosely with draft, so big ships
// carry more — the scheduler and the crews both feel it.
share fn arrival(tick: Int) = {
    let draft = clamp(to_float(random.randint(45, 140)) / 10.0, 4.5, 14.0)
    let pieces = clamp(random.randint(1, 1 + to_int(draft / 3.0)), 1, 5)
    Vessel {
        name: random.choice(PREFIXES) + " " + random.choice(NAMES),
        callsign: letters(4) + "-" + show(random.randint(100, 999)),
        draft: draft,
        hold: map(range(0, pieces), (i) => random_cargo()),
        eta_tick: tick
    }
}

// One line for the traffic log: every hold item, described.
share fn manifest_line(v) = {
    let goods = v.hold |> map(describe_cargo) |> join("; ")
    `${v.callsign} ${v.name} | ${goods}`
}

// ── Self-checks ─────────────────────────────────────────────────────
test "callsign validation" {
    testing.assert_true(valid_callsign("ABCD-123"))
    testing.assert_false(valid_callsign("AB-123"))
    testing.assert_false(valid_callsign("ABCD-12"))
    testing.assert_false(valid_callsign("abcd-123"))
}

test "slugify makes filenames" {
    testing.assert_eq(slugify("MV Iron Duke"), "mv-iron-duke")
    testing.assert_eq(slugify("  Fair Isle!  "), "fair-isle")
}

test "arrivals are well-formed and reproducible" {
    random.seed(11)
    let v = arrival(3)
    testing.assert_true(valid_callsign(v.callsign))
    testing.assert_true(v.draft >= 4.5 && v.draft <= 14.0)
    testing.assert_true(len(v.hold) >= 1 && len(v.hold) <= 5)
    testing.assert_eq(v.eta_tick, 3)
    // same seed, same ship
    random.seed(11)
    let w = arrival(3)
    testing.assert_eq(w.name, v.name)
    testing.assert_eq(w.callsign, v.callsign)
}

test "the hailing trait dispatches" {
    random.seed(4)
    let v = arrival(0)
    testing.assert_true(str.contains(hail_vessel(v), v.callsign))
    testing.assert_eq(loud_hail_vessel(v), str.to_upper(hail_vessel(v)))
}
