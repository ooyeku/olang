// A concrete workflow defined on top of the generic engine: expense approval.
//
// An expense is submitted, reviewed, and paid. Small amounts a manager can
// approve outright; larger ones (over $1000) need a second director approval.
// The branch is expressed purely with guards on the expense amount, which the
// engine carries in the context.

use lib.machine { transition, always, keep }

let THRESHOLD = 1000

// Actions record who acted, without mutating the caller's context.
fn record(field) = (ctx, event) => map_set(ctx, field, event.actor)

// Guards branch on the amount held in the context.
fn amount_at_most(limit) = (ctx, event) => map_get(ctx, "amount") <= limit
fn amount_over(limit) = (ctx, event) => map_get(ctx, "amount") > limit

// The transition table. Two `submitted --approve--> ...` rows share a
// from-state/event pair and are told apart by their amount guards.
share fn transitions() = [
    transition("draft", "submit", "submitted", always, record("submitter")),
    transition("submitted", "approve", "approved", amount_at_most(THRESHOLD), record("approver")),
    transition("submitted", "approve", "director_review", amount_over(THRESHOLD), record("manager")),
    transition("submitted", "reject", "rejected", always, record("rejected_by")),
    transition("director_review", "approve", "approved", always, record("approver")),
    transition("director_review", "reject", "rejected", always, record("rejected_by")),
    transition("approved", "pay", "paid", always, record("paid_by"))
]

share fn initial_state() = "draft"

// A fresh context for an expense of a given amount.
share fn new_expense(amount) = #{ "amount": amount }
