// contracts — load-time-checked boundaries as an imported macro library.
//
// Two kinds of guarantee, at two different times. @ensure and @require
// are runtime contracts that quote their own source when they fail —
// something no function can do, because an expression's text is gone by
// runtime. @fmtc is a compile-checked format string: a placeholder
// count that does not match its arguments refuses to expand, so the
// program never loads with the mismatch — the error every logging
// library discovers in production, moved to the moment the file parses.

// ── @require / @ensure: contracts that quote their source ────────────
// Identical machinery, different prefixes: require for preconditions,
// ensure for postconditions and invariants. A violation raises with the
// exact source of the condition that failed.
meta fn require(cond) = `{
    if !(${cond}) => unwrap(Err("contract violated (require): " + ${meta.lit(cond)}))
    else => ()
}`

meta fn ensure(cond) = `{
    if !(${cond}) => unwrap(Err("contract violated (ensure): " + ${meta.lit(cond)}))
    else => ()
}`

// ── @fmtc: a format string checked at expansion time ─────────────────
// The format must be a string literal and the arguments a list literal,
// because the check happens before the program runs: the number of {}
// placeholders is counted against the number of list elements, and a
// mismatch is a load error naming both counts. The output indexes the
// list at runtime, so the arguments themselves stay arbitrary
// expressions.
meta fn fmtc(fmt, args) = {
    let fmt_value = match meta.eval(fmt) {
        Ok(v) => v,
        Err(e) => unwrap(Err("@fmtc: the format must be a string literal: " + e))
    }
    let holes = len(str.split(fmt_value, "{}")) - 1
    let arg_node = head(unwrap(meta.parse(args)))
    let n_args = len(map_get(map_get(arg_node, "value"), "items"))
    if holes != n_args =>
        unwrap(Err(`@fmtc: the format string has ${holes} placeholder(s) but ${n_args} argument(s) were given`))
    else => ()
    let vals = meta.fresh("vals")
    let parts = str.split(fmt_value, "{}")
    let mut out = meta.lit(head(parts))
    let mut i = 0
    while i < holes {
        out = out + ` + show(${vals}[${i}]) + ` + meta.lit(parts[i + 1])
        i = i + 1
    }
    `{
    let ${vals} = ${args}
    ${out}
}`
}
