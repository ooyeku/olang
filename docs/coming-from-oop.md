# Coming from object-oriented languages

Part of [the olang book](README.md) · [A tour of olang](tour.md) · [Writing idiomatic olang](idioms.md).

olang has no classes. If you write Python, Java, C# or TypeScript, the
first thing you look for is where to put a class, and this chapter
answers that. Each section takes one thing a class gives you and shows
what olang has in its place. Every example runs as it stands.

## Table of contents

1. [An object is a record, a method is a function](#an-object-is-a-record-a-method-is-a-function)
2. [A constructor is a function](#a-constructor-is-a-function)
3. [Changing an object gives you a new one](#changing-an-object-gives-you-a-new-one)
4. [A class is a module](#a-class-is-a-module)
5. [Subclasses of one parent are an enum](#subclasses-of-one-parent-are-an-enum)
6. [An interface is a record of functions](#an-interface-is-a-record-of-functions)
7. [Inheriting and overriding is a spread](#inheriting-and-overriding-is-a-spread)
8. [State that must have an identity is a closure](#state-that-must-have-an-identity-is-a-closure)
9. [Why olang stops there](#why-olang-stops-there)

## An object is a record, a method is a function

A record holds the fields. A function that takes the record first does
what a method would do. The pipe `|>` puts the value in front of the
function, so a chain reads in the order it runs, the way a chain of
method calls does.

```olang
type Account = { owner: String, balance: Int }

fn deposit(account: Account, amount: Int) -> Account = { ...account, balance: account.balance + amount }
fn describe(account: Account) = account.owner + " has " + to_string(account.balance)

let account = { owner: "Ola", balance: 0 }
println(account |> deposit(50) |> deposit(20) |> describe())   // Ola has 70
```

`account.deposit(50)` is not olang: a dot reads a field. Write
`deposit(account, 50)` or `account |> deposit(50)`.

The `type` line is optional. It names the shape, so `olang check` can
tell you when a function reads a field the record does not have.

## A constructor is a function

There is no `new` and no `__init__`. A function that answers a record
is a constructor, and it can check its arguments and answer an error
the caller has to deal with.

```olang
fn new_account(owner, opening_balance = 0) =
    if opening_balance < 0 => Err("an account cannot open below zero")
    else => Ok({ owner: owner, balance: opening_balance })

println(show(new_account("Ola", 10)))   // Ok({ owner: "Ola", balance: 10 })
println(show(new_account("Ola", -5)))   // Err("an account cannot open below zero")
```

When a value is built in many places and all of them must agree, use a
`struct`: its literal must name every field and refuses one that is not
declared.

```olang
type Point = struct { x: Int, y: Int }

let origin = Point { x: 0, y: 0 }
println(origin.x + origin.y)   // 0
```

## Changing an object gives you a new one

A method that changes `self` has no counterpart, because a record
cannot be changed. A function answers a new record, and the spread
`{ ...old, field: value }` copies the rest. The old value is still what
it was, so nothing that held it is surprised.

```olang
let before = { owner: "Ola", balance: 70 }
let after = { ...before, balance: before.balance - 30 }

println(before.balance)   // 70
println(after.balance)    // 40
```

Inside one function, `let mut` gives you a name you can assign again,
which is what a loop needs.

```olang
let mut total = 0
for amount in [50, 20, -30] { total = total + amount }
println(total)   // 40
```

## A class is a module

A file is a module. The functions that work on one kind of value live
in one file, and the file's name does the work of the class's name. A
function marked `share` is public. One without it is private to the
file.

```olang no-run
// account.ol
share fn new_account(owner) = { owner: owner, balance: 0 }
share fn deposit(account, amount) = with_balance(account, account.balance + amount)

// not shared: only this file can call it
fn with_balance(account, balance) = { ...account, balance: balance }
```

```olang no-run
// main.ol
use account {}

let mine = account.new_account("Ola") |> account.deposit(50)
println(mine.balance)   // 50
```

`use account {}` brings the module in under its name, so a call reads
`account.deposit(…)`. That is why olang's own code has no prefixes on
function names: the module is the prefix.

## Subclasses of one parent are an enum

A base class with a fixed set of subclasses, each overriding the same
method, is an enum and a `match`. Each case carries its own data. The
`match` puts every case's behaviour in one place, and `olang check`
warns when a `match` forgets a case, which a missing override never
does.

```olang
type Shape = enum { Circle(Float), Rectangle(Float, Float) }

fn area(shape) = match shape {
    Circle(radius) => 3.14159 * radius * radius,
    Rectangle(width, height) => width * height,
}

println(area(Rectangle(3.0, 4.0)))   // 12.0
println(area(Circle(1.0)))           // 3.14159
```

A new operation is one new function with one `match`. A new case is one
line in the enum, and `olang check` then points at each `match` that
needs it.

## An interface is a record of functions

When the set of implementations is open, so that code you have not
written yet can supply one, pass a record whose fields are functions.
Anything with those fields fits. Nothing has to declare that it
implements anything.

```olang
type Store = { get: (String) -> String, name: String }

fn greeting_from(store: Store) = "From " + store.name + ": " + (store.get)("greeting")

let english = { name: "english", get: (key) => if key == "greeting" => "hello" else => "" }
let yoruba = { name: "yoruba", get: (key) => if key == "greeting" => "bawo" else => "" }

println(greeting_from(english))   // From english: hello
println(greeting_from(yoruba))    // From yoruba: bawo
```

A field that holds a function is called with the field in brackets:
`(store.get)("greeting")`.

This is also how you swap a real thing for a test's stand-in. The test
passes a record with the same fields.

## Inheriting and overriding is a spread

Inheritance is used for two things. One is the fixed set of kinds, and
that is the enum above. The other is "the same as that one, except":
that is a spread, which keeps every field and replaces the ones you
name.

```olang
let base_theme = { background: "white", text: "black", accent: "blue" }
let night_theme = { ...base_theme, background: "black", text: "white" }

println(night_theme.accent)       // blue
println(night_theme.background)   // black
```

Behaviour is overridden the same way, because a function is a value a
field can hold.

```olang
let plain = { name: "plain", greet: (who) => "Hello, " + who }
let loud = { ...plain, name: "loud", greet: (who) => "HELLO, " + str.to_upper(who) }

println((plain.greet)("Ola"))   // Hello, Ola
println((loud.greet)("Ola"))    // HELLO, OLA
```

There is no `super`. To build on what the base did, call the base's
function by name inside the new one.

## State that must have an identity is a closure

Sometimes a thing really is one thing that changes over time: a
counter, a cache, a connection. A function can hold a `cell`, and the
functions it answers share it.

```olang
fn new_counter() = {
    let count = cell(0)
    { next: () => { cell.set(count, cell.get(count) + 1); cell.get(count) } }
}

let counter = new_counter()
let first_number = (counter.next)()
let second_number = (counter.next)()
println(second_number)   // 2
```

Reach for this rarely. Most of what a program holds reads better as
values passed in and values answered, and a `cell` is for the few
places where the same thing must be seen to change from two sides.

## Why olang stops there

Every example above is a value going into a function and a value
coming out. That is what lets `olang check` follow a record's fields
through a program, lets a test call any function with nothing set up
first, and lets a person reading one function know that the value they
are looking at was not changed somewhere else.

A class with fields its methods change, and a subclass that changes
what a parent's method does, both hide where a value was changed and
where behaviour comes from. olang is meant to be written quickly, often
by a model, and then read and checked by a person. It keeps the forms
that person can follow.

Next: [Writing idiomatic olang](idioms.md) for the habits in full, and
[Common pitfalls](pitfalls.md) for what tends to catch people out.
