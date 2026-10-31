# The olang book

This book is the guide to olang, a programming language for small
tools, data work and web apps that you can trust after they are written.
It starts with what olang is and why it exists, teaches the language
through one worked example, and then goes as deep as you like: the full
language, the standard library, the tools, olang in the browser, and how
the engines underneath work.

You don't have to read it in order. If you are new, the first three
chapters are the place to start. The rest is easier to dip into when you
need something.

Every olang example in the book is run by the test suite, so the
examples do what the text says they do.

## Contents

### Getting started

| Chapter | Contents |
|---|---|
| [Introduction](introduction.md) | What olang is, why you might want it, and how this book is laid out. |
| [Installation](installation.md) | Getting olang onto your machine, running your first program, and the interactive prompt. |
| [A tour of olang](tour.md) | Learn the language by building one program, from a few lines of text to a small statistical report. |
| [Common pitfalls](pitfalls.md) | The things that tend to catch people out, and what to write instead. |
| [Stability and compatibility](stability.md) | What 1.0 promises: which parts are stable, which are still experimental, which platforms are covered, and what a version number means. |

### The language

| Chapter | Contents |
|---|---|
| [Language reference](language.md) | The precise rules for every part of the language, from values and operators to pattern matching, types, errors, concurrency and modules. |
| [Types and gradual typing](types.md) | Adding types when you want them: how annotations are checked as the program runs, and how `olang check` catches mistakes before it does. |
| [Macros](macros.md) | Writing olang code that writes olang code: `meta fn`, `@` calls, decorators, and the rules they follow. |

### The standard library

| Chapter | Contents |
|---|---|
| [Standard library reference](stdlib.md) | Every built-in function and module, with what each one takes and returns. |
| [The data stack](ods.md) | Working with data: columns and tables, grouping and joins, dates, matrices, statistics, machine learning, and charts. |

### Tools

| Chapter | Contents |
|---|---|
| [Command-line tooling](tooling.md) | Everything the `olang` command does, from running and testing programs to profiling, building, and recording and replaying runs. |
| [Packages and dependencies](packages.md) | Projects and `otc`: dependencies, your personal shelf of libraries, capabilities, and binaries that carry their own source. |
| [Notebooks](notebooks.md) | Notebooks as plain Markdown files with olang cells that run, take parameters, and keep their output. |
| [Writing fast olang](performance.md) | How to write code the fast engines like, how to see what they decided, and how olang measures up. |
| [Editor support](editors.md) | Setting up VS Code, Zed or any other editor that speaks the language server protocol. |

### The browser

| Chapter | Contents |
|---|---|
| [olang in the browser](wasm.md) | Running olang inside a web page: the page itself, events, drawing, views, routing and background workers. |
| [The web SDK](web-sdk.md) | Building a whole web app in olang, with one set of routes and views shared by the server and the browser. |

### Going deeper

| Chapter | Contents |
|---|---|
| [Architecture and internals](internals.md) | How the implementation fits together, for anyone who wants to read or work on it. |
| [The execution model: OVM and JIT](ovm.md) | The bytecode VM and the JIT in depth: what each one compiles, how each stays faithful to the interpreter, and what they cost and gain. |

## Reading paths

If you want to learn the language, read the [Introduction](introduction.md)
and [Installation](installation.md), work through [A tour of
olang](tour.md) and [Common pitfalls](pitfalls.md), and then keep the
[Language reference](language.md) open while you read the programs in
[`examples/`](../examples/).

If you are here to analyse data, go to [The data stack](ods.md), and use
the tables in the [Standard library
reference](stdlib.md#ods--series-and-frames) as you go.

To build a project of your own, read [Packages and
dependencies](packages.md) and [Command-line tooling](tooling.md). For
the web, start with [olang in the browser](wasm.md) and then [The web
SDK](web-sdk.md).

To make a program faster, read [Writing fast olang](performance.md) and
then [The execution model](ovm.md). And if you want to work on olang
itself, [Architecture and internals](internals.md) and [The execution
model](ovm.md) are where to begin.

## Conventions

A block marked ` ```olang ` is run by the test suite and works exactly
as written. A block marked ` ```olang no-run ` is checked for syntax but
not run, usually because it needs files, a network, a browser or several
modules at once.

Operations that can fail return a `Result`, which holds either a value
or an error. To keep them short, the examples often call `unwrap` to
take the value out; real programs usually handle the error instead.

[Stability and compatibility](stability.md) is the official word on
what is stable. If anything else in the book seems to disagree with it,
that chapter wins.
