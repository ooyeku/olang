use crate::ast::Value;
use rand::distributions::Alphanumeric;
use rand::prelude::*;
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Thread-safe random number generator
static RNG: OnceLock<Mutex<StdRng>> = OnceLock::new();

/// Initialize the global random number generator
fn get_rng() -> &'static Mutex<StdRng> {
    RNG.get_or_init(|| Mutex::new(StdRng::from_entropy()))
}

/// Error types for random operations
#[derive(Debug, thiserror::Error)]
pub enum RandomError {
    #[error("Invalid range: {message}")]
    InvalidRange { message: String },
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Empty collection provided")]
    EmptyCollection,
}

/// Creates the random module with all random generation functions
pub fn create_random_module() -> Value {
    let mut module = HashMap::new();

    // Random number generation
    module.insert("random".to_string(), create_builtin_function("random", 0));
    module.insert("randint".to_string(), create_builtin_function("randint", 2));
    module.insert("uniform".to_string(), create_builtin_function("uniform", 2));
    module.insert("gauss".to_string(), create_builtin_function("gauss", 2));

    // Random boolean and choices
    module.insert(
        "randbool".to_string(),
        create_builtin_function("randbool", 0),
    );
    module.insert("choice".to_string(), create_builtin_function("choice", 1));
    module.insert("choices".to_string(), create_builtin_function("choices", 2));
    module.insert("sample".to_string(), create_builtin_function("sample", 2));

    // Random string generation
    module.insert("randstr".to_string(), create_builtin_function("randstr", 1));
    module.insert(
        "randstr_alpha".to_string(),
        create_builtin_function("randstr_alpha", 1),
    );
    module.insert(
        "randstr_numeric".to_string(),
        create_builtin_function("randstr_numeric", 1),
    );
    module.insert(
        "randstr_alnum".to_string(),
        create_builtin_function("randstr_alnum", 1),
    );

    // Utility functions
    module.insert("shuffle".to_string(), create_builtin_function("shuffle", 1));
    module.insert("seed".to_string(), create_builtin_function("seed", 1));

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("random.{}", name),
        arity,
    })
}

/// Main dispatcher for random function calls
pub fn call_random_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "random" => random_random(args),
        "randint" => random_randint(args),
        "uniform" => random_uniform(args),
        "gauss" => random_gauss(args),
        "randbool" => random_randbool(args),
        "choice" => random_choice(args),
        "choices" => random_choices(args),
        "sample" => random_sample(args),
        "randstr" => random_randstr(args),
        "randstr_alpha" => random_randstr_alpha(args),
        "randstr_numeric" => random_randstr_numeric(args),
        "randstr_alnum" => random_randstr_alnum(args),
        "shuffle" => random_shuffle(args),
        "seed" => random_seed(args),
        _ => Err(format!("Unknown random function: {}", name).into()),
    }
}

/// Random float between 0.0 and 1.0 (exclusive)
/// Usage: random.random() -> 0.123456...
fn random_random(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("random expects 0 arguments, got {}", args.len()).into());
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let value: f64 = rng.gen();

    Ok(Value::Float(value))
}

/// Random integer between min and max (inclusive)
/// Usage: random.randint(1, 10) -> 7
fn random_randint(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("randint expects 2 arguments, got {}", args.len()).into());
    }

    let min = match &args[0] {
        Value::Integer(n) => *n,
        _ => return Err("randint: first argument must be an integer".into()),
    };

    let max = match &args[1] {
        Value::Integer(n) => *n,
        _ => return Err("randint: second argument must be an integer".into()),
    };

    if min > max {
        return Err("randint: min cannot be greater than max".into());
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let value = rng.gen_range(min..=max);

    Ok(Value::Integer(value))
}

/// Random float between min and max
/// Usage: random.uniform(0.0, 1.0) -> 0.456789...
fn random_uniform(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("uniform expects 2 arguments, got {}", args.len()).into());
    }

    let min = match &args[0] {
        Value::Integer(n) => *n as f64,
        Value::Float(f) => *f,
        _ => return Err("uniform: first argument must be a number".into()),
    };

    let max = match &args[1] {
        Value::Integer(n) => *n as f64,
        Value::Float(f) => *f,
        _ => return Err("uniform: second argument must be a number".into()),
    };

    if !min.is_finite() || !max.is_finite() {
        return Err("uniform: min and max must be finite numbers".into());
    }

    if min >= max {
        return Err("uniform: min must be less than max".into());
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let value = rng.gen_range(min..max);

    Ok(Value::Float(value))
}

/// Random number from normal distribution
/// Usage: random.gauss(0.0, 1.0) -> normally distributed around 0 with std dev 1
fn random_gauss(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("gauss expects 2 arguments, got {}", args.len()).into());
    }

    let mean = match &args[0] {
        Value::Integer(n) => *n as f64,
        Value::Float(f) => *f,
        _ => return Err("gauss: first argument (mean) must be a number".into()),
    };

    let std_dev = match &args[1] {
        Value::Integer(n) => *n as f64,
        Value::Float(f) => *f,
        _ => return Err("gauss: second argument (std_dev) must be a number".into()),
    };

    // is_finite() rejects NaN and infinities in one check
    if !std_dev.is_finite() || std_dev <= 0.0 || !mean.is_finite() {
        return Err("gauss: standard deviation must be a positive finite number".into());
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let normal = rand_distr::Normal::new(mean, std_dev)
        .map_err(|e| format!("gauss: invalid distribution parameters: {}", e))?;
    let value = normal.sample(&mut *rng);

    Ok(Value::Float(value))
}

/// Random boolean
/// Usage: random.randbool() -> true or false
fn random_randbool(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("randbool expects 0 arguments, got {}", args.len()).into());
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let value: bool = rng.gen();

    Ok(Value::Boolean(value))
}

/// Choose random element from list or range
/// Usage: random.choice([1, 2, 3, 4]) -> 3 or random.choice(1..10) -> 7
fn random_choice(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("choice expects 1 argument, got {}", args.len()).into());
    }

    match &args[0] {
        Value::List(items) => {
            if items.is_empty() {
                return Err("choice: cannot choose from empty list".into());
            }

            let rng = get_rng();
            let mut rng = rng.lock().unwrap();
            let index = rng.gen_range(0..items.len());

            Ok(items[index].clone())
        }
        Value::Range {
            start,
            end,
            inclusive,
        } => {
            let empty = if *inclusive {
                start > end
            } else {
                start >= end
            };
            if empty {
                return Err("choice: cannot choose from empty range".into());
            }

            let rng = get_rng();
            let mut rng = rng.lock().unwrap();

            let random_value = if *inclusive {
                rng.gen_range(*start..=*end)
            } else {
                rng.gen_range(*start..*end)
            };

            Ok(Value::Integer(random_value))
        }
        _ => Err("choice: argument must be a list or range".into()),
    }
}

/// Choose multiple random elements with replacement
/// Usage: random.choices([1, 2, 3], 5) -> [2, 1, 3, 2, 1] or random.choices(1..10, 5) -> [3, 7, 1, 9, 4]
fn random_choices(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("choices expects 2 arguments, got {}", args.len()).into());
    }

    let k = match &args[1] {
        Value::Integer(n) if *n >= 0 => *n as usize,
        Value::Integer(_) => return Err("choices: count cannot be negative".into()),
        _ => return Err("choices: second argument must be an integer".into()),
    };

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let mut result = Vec::new();

    match &args[0] {
        Value::List(items) => {
            if items.is_empty() {
                return Err("choices: cannot choose from empty list".into());
            }

            for _ in 0..k {
                let index = rng.gen_range(0..items.len());
                result.push(items[index].clone());
            }
        }
        Value::Range {
            start,
            end,
            inclusive,
        } => {
            let empty = if *inclusive {
                start > end
            } else {
                start >= end
            };
            if empty {
                return Err("choices: cannot choose from empty range".into());
            }

            for _ in 0..k {
                let random_value = if *inclusive {
                    rng.gen_range(*start..=*end)
                } else {
                    rng.gen_range(*start..*end)
                };
                result.push(Value::Integer(random_value));
            }
        }
        _ => return Err("choices: first argument must be a list or range".into()),
    }

    Ok(Value::List(result.into()))
}

/// Choose multiple random elements without replacement
/// Usage: random.sample([1, 2, 3, 4, 5], 3) -> [2, 5, 1] or random.sample(1..10, 3) -> [7, 2, 9]
fn random_sample(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("sample expects 2 arguments, got {}", args.len()).into());
    }

    let k = match &args[1] {
        Value::Integer(n) if *n >= 0 => *n as usize,
        Value::Integer(_) => return Err("sample: count cannot be negative".into()),
        _ => return Err("sample: second argument must be an integer".into()),
    };

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();

    match &args[0] {
        Value::List(items) => {
            if items.is_empty() {
                return Err("sample: cannot sample from empty list".into());
            }

            if k > items.len() {
                return Err("sample: sample size cannot be larger than population".into());
            }

            let mut items_mut: Vec<_> = items.iter().cloned().collect();
            items_mut.shuffle(&mut *rng);
            items_mut.truncate(k);

            Ok(Value::List(items_mut.into()))
        }
        Value::Range {
            start,
            end,
            inclusive,
        } => {
            // Compute the size in i128 so reversed or extreme bounds can't wrap
            let range_size = if *inclusive {
                *end as i128 - *start as i128 + 1
            } else {
                *end as i128 - *start as i128
            };

            if range_size <= 0 {
                return Err("sample: cannot sample from empty range".into());
            }

            if k as i128 > range_size {
                return Err("sample: sample size cannot be larger than range size".into());
            }

            // For small ranges, shuffle all values; for large ones, draw
            // distinct values by rejection sampling instead of materializing
            // the whole range.
            if range_size <= 1_000_000 {
                let mut range_values: Vec<i64> = if *inclusive {
                    (*start..=*end).collect()
                } else {
                    (*start..*end).collect()
                };

                range_values.shuffle(&mut *rng);
                range_values.truncate(k);

                let result: Vec<Value> = range_values.into_iter().map(Value::Integer).collect();
                Ok(Value::List(result.into()))
            } else {
                let mut seen = std::collections::HashSet::with_capacity(k);
                let mut result = Vec::with_capacity(k);
                while result.len() < k {
                    let value = if *inclusive {
                        rng.gen_range(*start..=*end)
                    } else {
                        rng.gen_range(*start..*end)
                    };
                    if seen.insert(value) {
                        result.push(Value::Integer(value));
                    }
                }
                Ok(Value::List(result.into()))
            }
        }
        _ => Err("sample: first argument must be a list or range".into()),
    }
}

/// Generate random string with custom characters
/// Usage: random.randstr(10) -> random 10-character string
fn random_randstr(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("randstr expects 1 argument, got {}", args.len()).into());
    }

    let length = match &args[0] {
        Value::Integer(n) if *n >= 0 => *n as usize,
        Value::Integer(_) => return Err("randstr: length cannot be negative".into()),
        _ => return Err("randstr: argument must be an integer".into()),
    };

    if length == 0 {
        return Ok(Value::String("".to_string().into()));
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let chars: String = (0..length)
        .map(|_| rng.sample(Alphanumeric) as char)
        .collect();

    Ok(Value::String(chars.into()))
}

/// Generate random alphabetic string
/// Usage: random.randstr_alpha(8) -> "AbcDefGh"
fn random_randstr_alpha(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("randstr_alpha expects 1 argument, got {}", args.len()).into());
    }

    let length = match &args[0] {
        Value::Integer(n) if *n >= 0 => *n as usize,
        Value::Integer(_) => return Err("randstr_alpha: length cannot be negative".into()),
        _ => return Err("randstr_alpha: argument must be an integer".into()),
    };

    if length == 0 {
        return Ok(Value::String("".to_string().into()));
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let chars: String = (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..alphabet.len());
            alphabet.chars().nth(idx).unwrap()
        })
        .collect();

    Ok(Value::String(chars.into()))
}

/// Generate random numeric string
/// Usage: random.randstr_numeric(6) -> "123456"
fn random_randstr_numeric(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("randstr_numeric expects 1 argument, got {}", args.len()).into());
    }

    let length = match &args[0] {
        Value::Integer(n) if *n >= 0 => *n as usize,
        Value::Integer(_) => return Err("randstr_numeric: length cannot be negative".into()),
        _ => return Err("randstr_numeric: argument must be an integer".into()),
    };

    if length == 0 {
        return Ok(Value::String("".to_string().into()));
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let digits = "0123456789";
    let chars: String = (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..digits.len());
            digits.chars().nth(idx).unwrap()
        })
        .collect();

    Ok(Value::String(chars.into()))
}

/// Generate random alphanumeric string
/// Usage: random.randstr_alnum(10) -> "a1B2c3D4e5"
fn random_randstr_alnum(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("randstr_alnum expects 1 argument, got {}", args.len()).into());
    }

    let length = match &args[0] {
        Value::Integer(n) if *n >= 0 => *n as usize,
        Value::Integer(_) => return Err("randstr_alnum: length cannot be negative".into()),
        _ => return Err("randstr_alnum: argument must be an integer".into()),
    };

    if length == 0 {
        return Ok(Value::String("".to_string().into()));
    }

    let rng = get_rng();
    let mut rng = rng.lock().unwrap();
    let chars: String = (0..length)
        .map(|_| rng.sample(Alphanumeric) as char)
        .collect();

    Ok(Value::String(chars.into()))
}

/// Shuffle a list in place and return shuffled copy
/// Usage: random.shuffle([1, 2, 3, 4]) -> [3, 1, 4, 2]
fn random_shuffle(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("shuffle expects 1 argument, got {}", args.len()).into());
    }

    match &args[0] {
        Value::List(items) => {
            let rng = get_rng();
            let mut rng = rng.lock().unwrap();
            let mut items_mut: Vec<_> = items.iter().cloned().collect();
            items_mut.shuffle(&mut *rng);

            Ok(Value::List(items_mut.into()))
        }
        _ => Err("shuffle: argument must be a list".into()),
    }
}

/// Seed the random number generator for reproducible results
/// Usage: random.seed(12345)
fn random_seed(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("seed expects 1 argument, got {}", args.len()).into());
    }

    let seed = match &args[0] {
        Value::Integer(n) => *n as u64,
        _ => return Err("seed: argument must be an integer".into()),
    };

    let rng = get_rng();
    let mut rng_guard = rng.lock().unwrap();
    *rng_guard = StdRng::seed_from_u64(seed);
    drop(rng_guard);

    Ok(Value::Unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Value;
    use std::sync::Arc;

    // Helper functions to create test values
    fn string_val(s: &str) -> Value {
        Value::String(Arc::new(s.to_string()))
    }

    fn int_val(n: i64) -> Value {
        Value::Integer(n)
    }

    fn float_val(f: f64) -> Value {
        Value::Float(f)
    }

    fn bool_val(b: bool) -> Value {
        Value::Boolean(b)
    }

    fn list_val(items: Vec<Value>) -> Value {
        Value::List(items.into())
    }

    // Helper to check if a value is within a range
    fn assert_in_range(value: &Value, min: f64, max: f64) {
        match value {
            Value::Integer(n) => {
                let f = *n as f64;
                assert!(
                    f >= min && f <= max,
                    "Value {} not in range [{}, {}]",
                    f,
                    min,
                    max
                );
            }
            Value::Float(f) => {
                assert!(
                    f >= &min && f <= &max,
                    "Value {} not in range [{}, {}]",
                    f,
                    min,
                    max
                );
            }
            _ => panic!("Expected numeric value, got {:?}", value),
        }
    }

    // Helper to check string properties
    fn assert_string_properties(value: &Value, expected_length: usize, char_set: &str) {
        match value {
            Value::String(s) => {
                assert_eq!(
                    s.len(),
                    expected_length,
                    "Expected length {}, got {}",
                    expected_length,
                    s.len()
                );
                for ch in s.chars() {
                    assert!(
                        char_set.contains(ch),
                        "Character '{}' not in allowed set '{}'",
                        ch,
                        char_set
                    );
                }
            }
            _ => panic!("Expected string value, got {:?}", value),
        }
    }

    #[test]
    fn test_create_random_module() {
        let module = create_random_module();
        if let Value::Struct { fields, .. } = module {
            // Test that all expected functions are present
            let expected_functions = vec![
                "random",
                "randint",
                "uniform",
                "gauss",
                "randbool",
                "choice",
                "choices",
                "sample",
                "randstr",
                "randstr_alpha",
                "randstr_numeric",
                "randstr_alnum",
                "shuffle",
                "seed",
            ];

            for func_name in expected_functions {
                assert!(
                    fields.contains_key(func_name),
                    "Missing function: {}",
                    func_name
                );
                if let Value::Builtin(builtin) = &fields[func_name] {
                    assert_eq!(builtin.name, format!("random.{}", func_name));
                } else {
                    panic!(
                        "Expected builtin function for {}, got: {:?}",
                        func_name, fields[func_name]
                    );
                }
            }

            assert_eq!(fields.len(), 14, "Expected 14 functions in random module");
        } else {
            panic!("Expected struct for random module, got: {:?}", module);
        }
    }

    #[test]
    fn test_random_basic_functionality() {
        // Test random() - should return float between 0.0 and 1.0
        let result = random_random(vec![]).unwrap();
        assert_in_range(&result, 0.0, 1.0);
        assert!(matches!(result, Value::Float(_)));

        // Test randbool() - should return boolean
        let result = random_randbool(vec![]).unwrap();
        assert!(matches!(result, Value::Boolean(_)));
    }

    #[test]
    fn test_randint_functionality() {
        // Test basic range
        let result = random_randint(vec![int_val(1), int_val(10)]).unwrap();
        assert_in_range(&result, 1.0, 10.0);
        assert!(matches!(result, Value::Integer(_)));

        // Test same min and max
        let result = random_randint(vec![int_val(5), int_val(5)]).unwrap();
        assert_eq!(result, int_val(5));

        // Test negative range
        let result = random_randint(vec![int_val(-10), int_val(-1)]).unwrap();
        assert_in_range(&result, -10.0, -1.0);
    }

    #[test]
    fn test_uniform_functionality() {
        // Test with floats
        let result = random_uniform(vec![float_val(0.5), float_val(1.5)]).unwrap();
        assert_in_range(&result, 0.5, 1.5);
        assert!(matches!(result, Value::Float(_)));

        // Test with integers
        let result = random_uniform(vec![int_val(1), int_val(10)]).unwrap();
        assert_in_range(&result, 1.0, 10.0);
        assert!(matches!(result, Value::Float(_)));

        // Test mixed types
        let result = random_uniform(vec![int_val(0), float_val(1.0)]).unwrap();
        assert_in_range(&result, 0.0, 1.0);
    }

    #[test]
    fn test_gauss_functionality() {
        // Test normal distribution - values should be around the mean
        // We can't test exact values, but we can test that it returns floats
        let result = random_gauss(vec![float_val(0.0), float_val(1.0)]).unwrap();
        assert!(matches!(result, Value::Float(_)));

        // Test with different parameters
        let result = random_gauss(vec![int_val(100), float_val(15.0)]).unwrap();
        assert!(matches!(result, Value::Float(_)));
    }

    #[test]
    fn test_choice_functionality() {
        // Test with integer list
        let list = list_val(vec![int_val(1), int_val(2), int_val(3)]);
        let result = random_choice(vec![list]).unwrap();

        // Result should be one of the original values
        assert!(matches!(
            result,
            Value::Integer(1) | Value::Integer(2) | Value::Integer(3)
        ));

        // Test with string list
        let list = list_val(vec![string_val("a"), string_val("b"), string_val("c")]);
        let result = random_choice(vec![list]).unwrap();
        assert!(matches!(result, Value::String(_)));
    }

    #[test]
    fn test_choices_functionality() {
        let list = list_val(vec![int_val(1), int_val(2), int_val(3)]);
        let result = random_choices(vec![list, int_val(5)]).unwrap();

        if let Value::List(items) = result {
            assert_eq!(items.len(), 5);
            // All items should be from the original list
            for item in items.iter() {
                assert!(matches!(
                    item,
                    Value::Integer(1) | Value::Integer(2) | Value::Integer(3)
                ));
            }
        } else {
            panic!("Expected list result from choices, got: {:?}", result);
        }
    }

    #[test]
    fn test_sample_functionality() {
        let list = list_val(vec![
            int_val(1),
            int_val(2),
            int_val(3),
            int_val(4),
            int_val(5),
        ]);
        let result = random_sample(vec![list, int_val(3)]).unwrap();

        if let Value::List(items) = result {
            assert_eq!(items.len(), 3);
            // All items should be unique (no duplicates in sample without replacement)
            // Since we can't use HashSet with Value, we'll check for duplicates differently
            for i in 0..items.len() {
                for j in (i + 1)..items.len() {
                    assert_ne!(items[i], items[j], "Found duplicate items in sample");
                }
                assert!(matches!(
                    items[i],
                    Value::Integer(1)
                        | Value::Integer(2)
                        | Value::Integer(3)
                        | Value::Integer(4)
                        | Value::Integer(5)
                ));
            }
        } else {
            panic!("Expected list result from sample, got: {:?}", result);
        }
    }

    #[test]
    fn test_string_generation() {
        // Test randstr
        let result = random_randstr(vec![int_val(10)]).unwrap();
        if let Value::String(s) = result {
            assert_eq!(s.len(), 10);
            // Should contain only alphanumeric characters
            for ch in s.chars() {
                assert!(ch.is_alphanumeric());
            }
        } else {
            panic!("Expected string from randstr, got: {:?}", result);
        }

        // Test randstr_alpha
        let result = random_randstr_alpha(vec![int_val(8)]).unwrap();
        assert_string_properties(
            &result,
            8,
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
        );

        // Test randstr_numeric
        let result = random_randstr_numeric(vec![int_val(6)]).unwrap();
        assert_string_properties(&result, 6, "0123456789");

        // Test randstr_alnum
        let result = random_randstr_alnum(vec![int_val(12)]).unwrap();
        if let Value::String(s) = result {
            assert_eq!(s.len(), 12);
            for ch in s.chars() {
                assert!(ch.is_alphanumeric());
            }
        } else {
            panic!("Expected string from randstr_alnum, got: {:?}", result);
        }

        // Test empty strings
        let result = random_randstr(vec![int_val(0)]).unwrap();
        assert_eq!(result, string_val(""));
    }

    #[test]
    fn test_shuffle_functionality() {
        let original = vec![int_val(1), int_val(2), int_val(3), int_val(4), int_val(5)];
        let list = list_val(original.clone());
        let result = random_shuffle(vec![list]).unwrap();

        if let Value::List(shuffled) = result {
            assert_eq!(shuffled.len(), 5);

            // All original elements should be present
            for original_item in &original {
                assert!(
                    shuffled.contains(original_item),
                    "Missing item after shuffle: {:?}",
                    original_item
                );
            }

            // No duplicates should be introduced
            for i in 0..shuffled.len() {
                for j in (i + 1)..shuffled.len() {
                    assert_ne!(
                        shuffled[i], shuffled[j],
                        "Found duplicate items after shuffle"
                    );
                }
            }
        } else {
            panic!("Expected list result from shuffle, got: {:?}", result);
        }
    }

    #[test]
    fn test_seed_functionality() {
        // Test that seed function works without errors
        let result = random_seed(vec![int_val(12345)]).unwrap();
        assert_eq!(result, Value::Unit);

        // Test that we can generate values after seeding
        let result = random_random(vec![]).unwrap();
        assert!(matches!(result, Value::Float(_)));

        // Note: We cannot reliably test deterministic behavior in parallel tests
        // due to global RNG state being shared across test threads
    }

    #[test]
    fn test_argument_validation_errors() {
        // Test functions with wrong number of arguments
        assert!(random_random(vec![int_val(1)]).is_err());
        assert!(random_randbool(vec![int_val(1)]).is_err());
        assert!(random_randint(vec![int_val(1)]).is_err());
        assert!(random_uniform(vec![int_val(1)]).is_err());
        assert!(random_gauss(vec![int_val(1)]).is_err());
        assert!(random_choice(vec![]).is_err());
        assert!(random_choices(vec![list_val(vec![int_val(1)])]).is_err());
        assert!(random_sample(vec![list_val(vec![int_val(1)])]).is_err());
        assert!(random_randstr(vec![]).is_err());
        assert!(random_shuffle(vec![]).is_err());
        assert!(random_seed(vec![]).is_err());

        // Test functions with wrong argument types
        assert!(random_randint(vec![string_val("not_int"), int_val(10)]).is_err());
        assert!(random_randint(vec![int_val(1), string_val("not_int")]).is_err());
        assert!(random_uniform(vec![string_val("not_num"), int_val(10)]).is_err());
        assert!(random_gauss(vec![string_val("not_num"), int_val(1)]).is_err());
        assert!(random_choice(vec![string_val("not_list")]).is_err());
        assert!(random_choices(vec![string_val("not_list"), int_val(2)]).is_err());
        assert!(random_choices(vec![list_val(vec![int_val(1)]), string_val("not_int")]).is_err());
        assert!(random_sample(vec![string_val("not_list"), int_val(2)]).is_err());
        assert!(random_sample(vec![list_val(vec![int_val(1)]), string_val("not_int")]).is_err());
        assert!(random_randstr(vec![string_val("not_int")]).is_err());
        assert!(random_shuffle(vec![string_val("not_list")]).is_err());
        assert!(random_seed(vec![string_val("not_int")]).is_err());
    }

    #[test]
    fn test_range_validation_errors() {
        // Test randint with invalid range
        assert!(random_randint(vec![int_val(10), int_val(5)]).is_err());

        // Test uniform with invalid range
        assert!(random_uniform(vec![float_val(1.0), float_val(1.0)]).is_err());
        assert!(random_uniform(vec![float_val(2.0), float_val(1.0)]).is_err());

        // Test gauss with invalid standard deviation
        assert!(random_gauss(vec![float_val(0.0), float_val(0.0)]).is_err());
        assert!(random_gauss(vec![float_val(0.0), float_val(-1.0)]).is_err());

        // Test sample with size larger than population
        let small_list = list_val(vec![int_val(1), int_val(2)]);
        assert!(random_sample(vec![small_list, int_val(5)]).is_err());
    }

    #[test]
    fn test_empty_collection_errors() {
        let empty_list = list_val(vec![]);

        // Test choice with empty list
        assert!(random_choice(vec![empty_list.clone()]).is_err());

        // Test choices with empty list
        assert!(random_choices(vec![empty_list.clone(), int_val(1)]).is_err());

        // Test sample with empty list
        assert!(random_sample(vec![empty_list, int_val(1)]).is_err());
    }

    #[test]
    fn test_call_random_function_dispatcher() {
        // Test that the dispatcher correctly routes to functions
        let result = call_random_function("random", vec![]);
        assert!(result.is_ok());

        let result = call_random_function("randint", vec![int_val(1), int_val(10)]);
        assert!(result.is_ok());

        let result = call_random_function("seed", vec![int_val(123)]);
        assert!(result.is_ok());

        // Test unknown function
        let result = call_random_function("unknown_function", vec![]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown random function"));
    }

    #[test]
    fn test_special_cases() {
        // Test randint with large range
        let result = random_randint(vec![int_val(i64::MIN), int_val(i64::MAX)]).unwrap();
        assert!(matches!(result, Value::Integer(_)));

        // Test uniform with very small range
        let result = random_uniform(vec![float_val(0.0), float_val(0.000001)]).unwrap();
        assert_in_range(&result, 0.0, 0.000001);

        // Test choices with k=0
        let list = list_val(vec![int_val(1), int_val(2)]);
        let result = random_choices(vec![list, int_val(0)]).unwrap();
        if let Value::List(items) = result {
            assert_eq!(items.len(), 0);
        } else {
            panic!(
                "Expected empty list for choices with k=0, got: {:?}",
                result
            );
        }

        // Test sample with k=0
        let list = list_val(vec![int_val(1), int_val(2)]);
        let result = random_sample(vec![list, int_val(0)]).unwrap();
        if let Value::List(items) = result {
            assert_eq!(items.len(), 0);
        } else {
            panic!("Expected empty list for sample with k=0, got: {:?}", result);
        }
    }

    #[test]
    fn test_distribution_properties() {
        // Set seed for reproducible test
        random_seed(vec![int_val(42)]).unwrap();

        // Test that randint covers the full range over multiple calls
        let mut values = std::collections::HashSet::new();
        for _ in 0..50 {
            let result = random_randint(vec![int_val(1), int_val(5)]).unwrap();
            if let Value::Integer(n) = result {
                values.insert(n);
            }
        }

        // We should see most values in the range (statistical test)
        assert!(
            values.len() >= 3,
            "Should see at least 3 different values in range 1-5"
        );

        // Test that uniform generates different values
        let mut uniform_values = Vec::new();
        for _ in 0..10 {
            let result = random_uniform(vec![float_val(0.0), float_val(1.0)]).unwrap();
            if let Value::Float(f) = result {
                uniform_values.push(f);
            }
        }

        // Check that we got different values (very unlikely to get duplicates)
        uniform_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let unique_count = uniform_values
            .windows(2)
            .filter(|w| (w[0] - w[1]).abs() > 1e-10)
            .count()
            + 1;
        assert!(
            unique_count >= 8,
            "Should generate mostly unique uniform values"
        );
    }

    #[test]
    fn test_string_generation_edge_cases() {
        // Test string generation with different lengths
        for length in [0, 1, 5, 100] {
            let result = random_randstr(vec![int_val(length)]).unwrap();
            if let Value::String(s) = result {
                assert_eq!(s.len(), length as usize);
            }

            let result = random_randstr_alpha(vec![int_val(length)]).unwrap();
            if let Value::String(s) = result {
                assert_eq!(s.len(), length as usize);
            }

            let result = random_randstr_numeric(vec![int_val(length)]).unwrap();
            if let Value::String(s) = result {
                assert_eq!(s.len(), length as usize);
            }
        }
    }

    #[test]
    fn test_list_operations_with_mixed_types() {
        // Test choice/sample/shuffle with mixed type lists
        let mixed_list = list_val(vec![
            int_val(42),
            string_val("hello"),
            bool_val(true),
            float_val(3.14),
        ]);

        // Test choice
        let result = random_choice(vec![mixed_list.clone()]).unwrap();
        // Should be one of the original values
        assert!(matches!(
            result,
            Value::Integer(42) | Value::String(_) | Value::Boolean(true) | Value::Float(_)
        ));

        // Test shuffle
        let result = random_shuffle(vec![mixed_list.clone()]).unwrap();
        if let Value::List(shuffled) = result {
            assert_eq!(shuffled.len(), 4);
        } else {
            panic!("Expected list from shuffle, got: {:?}", result);
        }

        // Test sample
        let result = random_sample(vec![mixed_list, int_val(2)]).unwrap();
        if let Value::List(sampled) = result {
            assert_eq!(sampled.len(), 2);
        } else {
            panic!("Expected list from sample, got: {:?}", result);
        }
    }
}
