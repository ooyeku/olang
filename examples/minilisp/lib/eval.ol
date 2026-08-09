// The evaluator: LVal syntax trees -> LVal values.
//
// Environments are olang maps threaded functionally: `define` returns an
// extended environment, closures capture the map at lambda creation
// (olang's own capture-by-value, one level down). Errors are Err values
// all the way up — a mini-Lisp error never throws.

use lib.reader { LVal }

// ── environment ──

fn env_get(env, name) = {
    if map_has_key(env, name) => Ok(map_get(env, name))
    else => Err("unbound symbol: " + name)
}

// ── evaluation ──

// eval returns Ok((value, env)) so define can extend the environment.
share fn eval(form, env) = match form {
    LNum(n) => Ok((LNum(n), env)),
    LBool(b) => Ok((LBool(b), env)),
    LStr(s) => Ok((LStr(s), env)),
    LSym(name) => match env_get(env, name) {
        Ok(v) => Ok((v, env)),
        Err(e) => Err(e)
    },
    LList(items) => {
        if len(items) == 0 => Ok((LList([]), env))
        else => eval_list(items, env)
    },
    other => Ok((other, env))
}

fn eval_list(items, env) = {
    let head = items[0]
    match head {
        LSym(op) => {
            if op == "define" => eval_define(items, env)
            else if op == "if" => eval_if(items, env)
            else if op == "lambda" => eval_lambda(items, env)
            else if op == "quote" => Ok((items[1], env))
            else if op == "begin" => eval_begin(skip(items, 1), env)
            else => eval_call(items, env)
        },
        _ => eval_call(items, env)
    }
}

fn eval_define(items, env) = {
    match items[1] {
        LSym(name) => match eval(items[2], env) {
            Ok(pair) => {
                let (value, env2) = pair
                // A defined lambda becomes self-aware: the name is bound
                // into its own call environment at apply time — the same
                // call-time self-definition olang's interpreter uses for
                // its own named functions.
                let bound = match value {
                    LFn(params, body, captured) => LRec(name, params, body, captured),
                    other => other
                }
                Ok((bound, map_set(env2, name, bound)))
            },
            Err(e) => Err(e)
        },
        _ => Err("define: first argument must be a symbol")
    }
}

fn eval_if(items, env) = {
    match eval(items[1], env) {
        Ok(pair) => {
            let (cond, env2) = pair
            let truthy = match cond { LBool(b) => b, LNum(n) => n != 0, _ => true }
            if truthy => eval(items[2], env2)
            else if len(items) > 3 => eval(items[3], env2)
            else => Ok((LList([]), env2))
        },
        Err(e) => Err(e)
    }
}

fn eval_lambda(items, env) = {
    match items[1] {
        LList(params) => Ok((LFn(params, items[2], env), env)),
        _ => Err("lambda: parameter list expected")
    }
}

fn eval_begin(forms, env) = {
    let mut current = env
    let mut result = LList([])
    let mut failure = ""
    for form in forms {
        if failure == "" => {
            match eval(form, current) {
                Ok(pair) => {
                    let (v, e2) = pair
                    result = v
                    current = e2
                },
                Err(e) => { failure = e }
            }
        }
    }
    if failure == "" => Ok((result, current)) else => Err(failure)
}

// Evaluate every argument left to right; Ok(list) or the first Err.
fn eval_args(items, env) = {
    let mut out = []
    let mut failure = ""
    for item in items {
        if failure == "" => {
            match eval(item, env) {
                Ok(pair) => { let (v, _) = pair; out = concat(out, [v]) },
                Err(e) => { failure = e }
            }
        }
    }
    if failure == "" => Ok(out) else => Err(failure)
}

fn eval_call(items, env) = {
    match eval(items[0], env) {
        Ok(pair) => {
            let (callee, env2) = pair
            match eval_args(skip(items, 1), env2) {
                Ok(args) => match callee {
                    LPrim(name) => match apply_prim(name, args) {
                        Ok(v) => Ok((v, env2)),
                        Err(e) => Err(e)
                    },
                    LFn(params, body, captured) =>
                        apply_fn(params, body, captured, args, env2),
                    LRec(name, params, body, captured) =>
                        apply_fn(params, body,
                                 map_set(captured, name, LRec(name, params, body, captured)),
                                 args, env2),
                    _ => Err("not callable: " + show(callee))
                },
                Err(e) => Err(e)
            }
        },
        Err(e) => Err(e)
    }
}

fn apply_fn(params, body, captured, args, caller_env) = {
    if len(params) != len(args) =>
        Err("arity: expected " + show(len(params)) + ", got " + show(len(args)))
    else => {
        let mut call_env = captured
        let mut i = 0
        for p in params {
            match p {
                LSym(pname) => { call_env = map_set(call_env, pname, args[i]) },
                _ => {}
            }
            i = i + 1
        }
        match eval(body, call_env) {
            Ok(rpair) => { let (v, _) = rpair; Ok((v, caller_env)) },
            Err(e) => Err(e)
        }
    }
}

// ── primitives ──

fn num2(args, name) = {
    if len(args) != 2 => Err(name + ": two arguments expected")
    else => match (args[0], args[1]) {
        (LNum(a), LNum(b)) => Ok((a, b)),
        _ => Err(name + ": numbers expected")
    }
}

fn apply_prim(name, args) = {
    if name == "+" => match num2(args, "+") { Ok(p) => { let (a, b) = p; Ok(LNum(a + b)) }, Err(e) => Err(e) }
    else if name == "-" => match num2(args, "-") { Ok(p) => { let (a, b) = p; Ok(LNum(a - b)) }, Err(e) => Err(e) }
    else if name == "*" => match num2(args, "*") { Ok(p) => { let (a, b) = p; Ok(LNum(a * b)) }, Err(e) => Err(e) }
    else if name == "=" => match num2(args, "=") { Ok(p) => { let (a, b) = p; Ok(LBool(a == b)) }, Err(e) => Err(e) }
    else if name == "<" => match num2(args, "<") { Ok(p) => { let (a, b) = p; Ok(LBool(a < b)) }, Err(e) => Err(e) }
    else if name == ">" => match num2(args, ">") { Ok(p) => { let (a, b) = p; Ok(LBool(a > b)) }, Err(e) => Err(e) }
    else if name == "list" => Ok(LList(args))
    else if name == "car" => match args[0] { LList(xs) => if len(xs) > 0 => Ok(xs[0]) else => Err("car: empty list"), _ => Err("car: list expected") }
    else if name == "cdr" => match args[0] { LList(xs) => Ok(LList(skip(xs, 1))), _ => Err("cdr: list expected") }
    else if name == "cons" => match args[1] { LList(xs) => Ok(LList(concat([args[0]], xs))), _ => Err("cons: list expected") }
    else if name == "null?" => match args[0] { LList(xs) => Ok(LBool(len(xs) == 0)), _ => Ok(LBool(false)) }
    else => Err("unknown primitive: " + name)
}

// ── the standard environment ──

share fn base_env() = {
    let mut env = #{}
    for name in ["+", "-", "*", "=", "<", ">", "list", "car", "cdr", "cons", "null?"] {
        env = map_set(env, name, LPrim(name))
    }
    env
}

// Run a whole program: evaluate each form, threading the environment.
share fn run(forms) = {
    let mut env = base_env()
    let mut result = LList([])
    let mut failure = ""
    for form in forms {
        if failure == "" => {
            match eval(form, env) {
                Ok(pair) => { let (v, e2) = pair; result = v; env = e2 },
                Err(e) => { failure = e }
            }
        }
    }
    if failure == "" => Ok(result) else => Err(failure)
}
