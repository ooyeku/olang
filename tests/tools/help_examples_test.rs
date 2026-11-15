//! Every `:help` example must be real olang. The registry once carried
//! fictional syntax (`List[Int]` annotations, `let a(x) => ...`) that
//! taught users a language that does not exist; this guard makes that
//! class of rot a test failure. Each example is stripped of its
//! trailing `// result` comment and parsed; `->` result notation and
//! pseudo-code refuse here. Examples may reference free variables
//! (parse-only), but they must be *syntax* a user can type.

use olang::help::HelpSystem;

/// The code part of an example: everything before a trailing
/// `// ...` comment (string literals containing `//` survive because
/// we only strip when the `//` is outside quotes).
fn code_of(example: &str) -> String {
    let mut in_str = false;
    let mut prev = '\0';
    let bytes: Vec<char> = example.chars().collect();
    for i in 0..bytes.len() {
        let c = bytes[i];
        if c == '"' && prev != '\\' {
            in_str = !in_str;
        }
        if !in_str && c == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
            return bytes[..i].iter().collect::<String>().trim().to_string();
        }
        prev = c;
    }
    example.trim().to_string()
}

#[test]
fn every_help_example_parses() {
    let help = HelpSystem::new();
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for name in help.get_function_names() {
        // REPL commands (`:time`, `:ml`, ...) document terminal input,
        // not olang programs — their examples are transcripts.
        if name.starts_with(':') {
            continue;
        }
        let Some(doc) = help.get_function(&name) else {
            continue;
        };
        for example in &doc.examples {
            let code = code_of(example);
            if code.is_empty() {
                failures.push(format!("{name}: empty code in example {example:?}"));
                continue;
            }
            checked += 1;
            if let Err(e) = olang::parser::Parser::new().parse(&code) {
                failures.push(format!("{name}: example does not parse: {code:?} — {e}"));
            }
        }
    }
    assert!(checked > 300, "example corpus vanished ({checked} checked)");
    assert!(
        failures.is_empty(),
        "{} fictional examples in :help:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_entry_is_complete() {
    // A registry entry a user meets must explain itself: a real
    // description, at least one example, and a stated return type.
    let help = HelpSystem::new();
    let mut failures = Vec::new();
    for name in help.get_function_names() {
        let Some(doc) = help.get_function(&name) else {
            continue;
        };
        if doc.description.trim().len() < 20 {
            failures.push(format!(
                "{name}: description too thin: {:?}",
                doc.description
            ));
        }
        if doc.examples.is_empty() {
            failures.push(format!("{name}: no examples"));
        }
        if doc.return_type.trim().is_empty() {
            failures.push(format!("{name}: no return type"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} incomplete :help entries:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
