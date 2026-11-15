//! What help says a function returns is what it returns (OL-420).
//!
//! `olang check` refuses `!` and arithmetic on a `Result`, and it learns
//! which functions answer one from help's return types. When the crypto
//! hashes, HMAC, `secure_compare` and base64's encoders came to answer
//! their values directly, help still said `Result`, so
//! `!crypto.secure_compare(a, b)` was refused though it answers a Bool.
//! Each example here that stands alone is run, and its answer is a
//! `Result` exactly when help says so.

use olang::help::HelpSystem;
use olang::{Interpreter, Parser};

#[test]
fn an_example_answers_the_kind_help_says() {
    let help = HelpSystem::new();
    let mut checked = 0;
    let mut wrong = Vec::new();
    for name in help.get_function_names() {
        let module = name.split('.').next().unwrap_or("");
        if !["crypto", "base64", "str", "re", "math", "json"].contains(&module) {
            continue;
        }
        let doc = help.get_function(&name).expect("a listed function");
        let says_result = doc.return_type.starts_with("Result");
        for example in &doc.examples {
            let code = example.split("//").next().unwrap_or("").trim();
            if !code.starts_with(&format!("{name}(")) || !code.ends_with(')') {
                continue;
            }
            let Ok(program) = Parser::new().parse(&format!("typeof({code})")) else {
                continue;
            };
            let mut interpreter = Interpreter::new();
            let Ok(kind) = interpreter.eval_program(program) else {
                continue; // a free name, or an example of an error
            };
            let kind = kind.to_string();
            if kind.contains("Result") != says_result {
                wrong.push(format!(
                    "{name}: help says {}, `{code}` answers {kind}",
                    doc.return_type
                ));
            }
            checked += 1;
        }
    }
    wrong.dedup();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(checked > 50, "only {checked} examples ran");
}
