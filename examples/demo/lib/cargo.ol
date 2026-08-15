// cargo — the harbor's domain types: cargo as a sum type, tariffs as a
// recursive expression tree, and the pattern-matching that prices them.
//
// This module is the demo's ADT showcase: payload variants, structural
// recursion over a tree, guards, and discriminated-union records — the
// shapes a robust olang system uses to make illegal states unrepresentable.

use prelude { round1 }

// ── Cargo: a real sum type ──────────────────────────────────────────
// Container(teu)        — twenty-foot-equivalent units
// Bulk(tonnes)          — loose goods by weight
// Reefer(teu)           — refrigerated containers (power while berthed)
// Hazmat(class, tonnes) — dangerous goods, IMO class 1..9
share type Cargo = enum {
    Container(Int),
    Bulk(Float),
    Reefer(Int),
    Hazmat(Int, Float)
}

share fn describe_cargo(c) = match c {
    Container(teu)   => `${teu} TEU containers`,
    Bulk(t)          => `${round1(t)}t bulk`,
    Reefer(teu)      => `${teu} TEU reefer`,
    Hazmat(class, t) => `${round1(t)}t hazmat (class ${class})`
}

// Unload effort in crew-minutes — the quantity the worker crews consume.
share fn unload_effort(c) = match c {
    Container(teu)   => teu * 3,
    Bulk(t)          => to_int(t * 2.0),
    Reefer(teu)      => teu * 4,
    Hazmat(class, t) => to_int(t * 3.0) + class * 10
}

// Hazmat above class 5 needs the sealed berth; guards express policy.
share fn needs_sealed_berth(c) = match c {
    Hazmat(class, t) if class > 5 => true,
    _ => false
}

// ── Tariff: pricing rules as a recursive expression tree ────────────
// A tariff is built, inspected, and evaluated as data. New pricing
// policy is a new tree, not new code — the interpreter below never
// changes. This is the expression-evaluator pattern in production dress.
share type Tariff = enum {
    Flat(Float),
    PerUnit(Float),
    Surcharge(Tariff, Float),
    Sum(Tariff, Tariff),
    AtLeast(Tariff, Float)
}

// Evaluate a tariff tree against a unit count (TEU or tonnes).
share fn eval_tariff(t, units: Float) = match t {
    Flat(amount)        => amount,
    PerUnit(rate)       => rate * units,
    Surcharge(inner, f) => eval_tariff(inner, units) * f,
    Sum(a, b)           => eval_tariff(a, units) + eval_tariff(b, units),
    AtLeast(inner, m)   => {
        let v = eval_tariff(inner, units)
        if v < m => m else => v
    }
}

// Pretty-print the tree the same way it is evaluated: one recursion,
// two interpretations.
share fn show_tariff(t) = match t {
    Flat(amount)        => `flat(${amount})`,
    PerUnit(rate)       => `${rate}/unit`,
    Surcharge(inner, f) => `(${show_tariff(inner)} x${f})`,
    Sum(a, b)           => `(${show_tariff(a)} + ${show_tariff(b)})`,
    AtLeast(inner, m)   => `max(${show_tariff(inner)}, ${m})`
}

// The harbor's published tariff schedule, one tree per cargo kind.
// Rates come from config; the trees encode structure.
share fn tariff_for(c, rates) = match c {
    Container(teu)   => AtLeast(PerUnit(map_get(rates, "container")), 80.0),
    Reefer(teu)      => Sum(AtLeast(PerUnit(map_get(rates, "container")), 80.0),
                            PerUnit(map_get(rates, "reefer_power"))),
    Bulk(t)          => AtLeast(PerUnit(map_get(rates, "bulk")), 120.0),
    Hazmat(class, t) => Surcharge(PerUnit(map_get(rates, "bulk")),
                                  1.0 + to_float(class) * map_get(rates, "hazmat_step"))
}

// Units the tariff meters: TEU for boxes, tonnes for loose cargo.
share fn tariff_units(c) = match c {
    Container(teu)   => to_float(teu),
    Reefer(teu)      => to_float(teu),
    Bulk(t)          => t,
    Hazmat(class, t) => t
}

// Price one cargo item under the schedule.
share fn price_cargo(c, rates) = eval_tariff(tariff_for(c, rates), tariff_units(c))

// A cargo kind label for ledger rows and reports.
share fn kind_of(c) = match c {
    Container(teu)   => "container",
    Bulk(t)          => "bulk",
    Reefer(teu)      => "reefer",
    Hazmat(class, t) => "hazmat"
}

// ── Self-checks ─────────────────────────────────────────────────────
fn test_rates() = #{
    "container": 2.5, "reefer_power": 1.25, "bulk": 1.1, "hazmat_step": 0.4
}

// Float expectations compare within epsilon — 1.1 * 100.0 is
// 110.00000000000001 in IEEE arithmetic, and a robust test suite never
// pretends otherwise.
fn near(a: Float, b: Float) = math.abs(a - b) < 0.000001

test "tariff trees evaluate structurally" {
    // 40 TEU at 2.5/unit = 100, above the 80 floor
    testing.assert_true(near(price_cargo(Container(40), test_rates()), 100.0))
    // 10 TEU at 2.5 = 25, floored to 80
    testing.assert_true(near(price_cargo(Container(10), test_rates()), 80.0))
    // reefer = container leg (floored 80) + power 1.25 * 20
    testing.assert_true(near(price_cargo(Reefer(20), test_rates()), 105.0))
    // hazmat class 5: 1.1 * 100t, surcharged x(1 + 5*0.4) ~= 330
    testing.assert_true(near(price_cargo(Hazmat(5, 100.0), test_rates()), 330.0))
}

test "the printer mirrors the evaluator" {
    let t = tariff_for(Container(1), test_rates())
    // The expectation interpolates the same floats the printer does, so
    // the assertion is stable under any float-display convention.
    testing.assert_eq(show_tariff(t), `max(${2.5}/unit, ${80.0})`)
}

test "policy guards" {
    testing.assert_true(needs_sealed_berth(Hazmat(7, 10.0)))
    testing.assert_false(needs_sealed_berth(Hazmat(3, 10.0)))
    testing.assert_false(needs_sealed_berth(Container(10)))
}
