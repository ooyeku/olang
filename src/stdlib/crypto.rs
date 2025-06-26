use crate::ast::Value;
use bcrypt;
use hex;
use hmac::{Hmac, Mac};
use md5::Md5;
use rand::{thread_rng, RngCore};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::collections::HashMap;
use std::sync::Arc;

/// Error types for Crypto operations
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Hash error: {message}")]
    HashError { message: String },
    #[error("HMAC error: {message}")]
    HmacError { message: String },
    #[error("Password error: {message}")]
    PasswordError { message: String },
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Type error: {message}")]
    TypeError { message: String },
    #[error("Encoding error: {message}")]
    EncodingError { message: String },
}

/// Creates the crypto module with all cryptographic functions
pub fn create_crypto_module() -> Value {
    let mut module = HashMap::new();

    // Hash functions
    module.insert("md5".to_string(), create_builtin_function("md5", 1));
    module.insert("sha1".to_string(), create_builtin_function("sha1", 1));
    module.insert("sha256".to_string(), create_builtin_function("sha256", 1));
    module.insert("sha512".to_string(), create_builtin_function("sha512", 1));

    // HMAC functions
    module.insert(
        "hmac_sha256".to_string(),
        create_builtin_function("hmac_sha256", 2),
    );
    module.insert(
        "hmac_sha512".to_string(),
        create_builtin_function("hmac_sha512", 2),
    );

    // Password hashing
    module.insert(
        "hash_password".to_string(),
        create_builtin_function("hash_password", 1),
    );
    module.insert(
        "verify_password".to_string(),
        create_builtin_function("verify_password", 2),
    );

    // Random generation
    module.insert(
        "random_bytes".to_string(),
        create_builtin_function("random_bytes", 1),
    );
    module.insert(
        "random_hex".to_string(),
        create_builtin_function("random_hex", 1),
    );

    // Encoding utilities
    module.insert(
        "hex_encode".to_string(),
        create_builtin_function("hex_encode", 1),
    );
    module.insert(
        "hex_decode".to_string(),
        create_builtin_function("hex_decode", 1),
    );

    // Constant-time comparison
    module.insert(
        "secure_compare".to_string(),
        create_builtin_function("secure_compare", 2),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("crypto.{}", name),
        arity,
    })
}

/// Main dispatcher for Crypto function calls
pub fn call_crypto_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "md5" => crypto_md5(args),
        "sha1" => crypto_sha1(args),
        "sha256" => crypto_sha256(args),
        "sha512" => crypto_sha512(args),
        "hmac_sha256" => crypto_hmac_sha256(args),
        "hmac_sha512" => crypto_hmac_sha512(args),
        "hash_password" => crypto_hash_password(args),
        "verify_password" => crypto_verify_password(args),
        "random_bytes" => crypto_random_bytes(args),
        "random_hex" => crypto_random_hex(args),
        "hex_encode" => crypto_hex_encode(args),
        "hex_decode" => crypto_hex_decode(args),
        "secure_compare" => crypto_secure_compare(args),
        _ => Err(format!("Unknown crypto function: {}", name).into()),
    }
}

/// Compute MD5 hash of input
/// Usage: crypto.md5("hello") -> Result<String, Error>
fn crypto_md5(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "md5 expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "md5: argument must be a string".to_string(),
            )))))
        }
    };

    let mut hasher = Md5::new();
    hasher.update(input);
    let result = hasher.finalize();
    let hex_string = hex::encode(result);

    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_string)))))
}

/// Compute SHA1 hash of input
/// Usage: crypto.sha1("hello") -> Result<String, Error>
fn crypto_sha1(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "sha1 expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "sha1: argument must be a string".to_string(),
            )))))
        }
    };

    let mut hasher = Sha1::new();
    hasher.update(input);
    let result = hasher.finalize();
    let hex_string = hex::encode(result);

    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_string)))))
}

/// Compute SHA256 hash of input
/// Usage: crypto.sha256("hello") -> Result<String, Error>
fn crypto_sha256(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "sha256 expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "sha256: argument must be a string".to_string(),
            )))))
        }
    };

    let mut hasher = Sha256::new();
    hasher.update(input);
    let result = hasher.finalize();
    let hex_string = hex::encode(result);

    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_string)))))
}

/// Compute SHA512 hash of input
/// Usage: crypto.sha512("hello") -> Result<String, Error>
fn crypto_sha512(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "sha512 expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "sha512: argument must be a string".to_string(),
            )))))
        }
    };

    let mut hasher = Sha512::new();
    hasher.update(input);
    let result = hasher.finalize();
    let hex_string = hex::encode(result);

    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_string)))))
}

/// Compute HMAC-SHA256
/// Usage: crypto.hmac_sha256("secret", "message") -> Result<String, Error>
fn crypto_hmac_sha256(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "hmac_sha256 expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let key = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "hmac_sha256: first argument must be a string".to_string(),
            )))))
        }
    };

    let message = match &args[1] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "hmac_sha256: second argument must be a string".to_string(),
            )))))
        }
    };

    type HmacSha256 = Hmac<Sha256>;

    match HmacSha256::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(message);
            let result = mac.finalize();
            let hex_string = hex::encode(result.into_bytes());
            Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_string)))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "HMAC error: {}",
            e
        )))))),
    }
}

/// Compute HMAC-SHA512
/// Usage: crypto.hmac_sha512("secret", "message") -> Result<String, Error>
fn crypto_hmac_sha512(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "hmac_sha512 expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let key = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "hmac_sha512: first argument must be a string".to_string(),
            )))))
        }
    };

    let message = match &args[1] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "hmac_sha512: second argument must be a string".to_string(),
            )))))
        }
    };

    type HmacSha512 = Hmac<Sha512>;

    match HmacSha512::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(message);
            let result = mac.finalize();
            let hex_string = hex::encode(result.into_bytes());
            Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_string)))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "HMAC error: {}",
            e
        )))))),
    }
}

/// Hash a password using bcrypt
/// Usage: crypto.hash_password("mypassword") -> Result<String, Error>
fn crypto_hash_password(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "hash_password expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let password = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "hash_password: argument must be a string".to_string(),
            )))))
        }
    };

    match bcrypt::hash(password, bcrypt::DEFAULT_COST) {
        Ok(hashed) => Ok(Value::Ok(Box::new(Value::String(Arc::new(hashed))))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Password hashing error: {}",
            e
        )))))),
    }
}

/// Verify a password against a bcrypt hash
/// Usage: crypto.verify_password("mypassword", "$2b$12$...") -> Result<Bool, Error>
fn crypto_verify_password(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "verify_password expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let password = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "verify_password: first argument must be a string".to_string(),
            )))))
        }
    };

    let hash = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "verify_password: second argument must be a string".to_string(),
            )))))
        }
    };

    match bcrypt::verify(password, hash) {
        Ok(is_valid) => Ok(Value::Ok(Box::new(Value::Boolean(is_valid)))),
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Password verification error: {}",
            e
        )))))),
    }
}

/// Generate cryptographically secure random bytes
/// Usage: crypto.random_bytes(32) -> Result<[Int], Error>
fn crypto_random_bytes(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "random_bytes expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let count = match &args[0] {
        Value::Integer(i) => *i as usize,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "random_bytes: argument must be an integer".to_string(),
            )))))
        }
    };

    if count > 1024 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "random_bytes: maximum 1024 bytes allowed".to_string(),
        )))));
    }

    let mut bytes = vec![0u8; count];
    thread_rng().fill_bytes(&mut bytes);

    let byte_values: Vec<Value> = bytes
        .into_iter()
        .map(|b| Value::Integer(b as i64))
        .collect();

    Ok(Value::Ok(Box::new(Value::List(byte_values.into()))))
}

/// Generate cryptographically secure random hex string
/// Usage: crypto.random_hex(16) -> Result<String, Error>
fn crypto_random_hex(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "random_hex expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let count = match &args[0] {
        Value::Integer(i) => *i as usize,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "random_hex: argument must be an integer".to_string(),
            )))))
        }
    };

    if count > 1024 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "random_hex: maximum 1024 bytes allowed".to_string(),
        )))));
    }

    let mut bytes = vec![0u8; count];
    thread_rng().fill_bytes(&mut bytes);
    let hex_string = hex::encode(bytes);

    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_string)))))
}

/// Encode bytes to hexadecimal string
/// Usage: crypto.hex_encode("hello") -> Result<String, Error>
fn crypto_hex_encode(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "hex_encode expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "hex_encode: argument must be a string".to_string(),
            )))))
        }
    };

    let hex_string = hex::encode(input);
    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_string)))))
}

/// Decode hexadecimal string to bytes (as string)
/// Usage: crypto.hex_decode("68656c6c6f") -> Result<String, Error>
fn crypto_hex_decode(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "hex_decode expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let hex_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "hex_decode: argument must be a string".to_string(),
            )))))
        }
    };

    match hex::decode(hex_str) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(string) => Ok(Value::Ok(Box::new(Value::String(Arc::new(string))))),
            Err(_) => Ok(Value::Err(Box::new(Value::String(Arc::new(
                "hex_decode: decoded bytes are not valid UTF-8".to_string(),
            ))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Hex decode error: {}",
            e
        )))))),
    }
}

/// Constant-time string comparison (prevents timing attacks)
/// Usage: crypto.secure_compare("secret1", "secret2") -> Result<Bool, Error>
fn crypto_secure_compare(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "secure_compare expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let str1 = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "secure_compare: first argument must be a string".to_string(),
            )))))
        }
    };

    let str2 = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "secure_compare: second argument must be a string".to_string(),
            )))))
        }
    };

    // Simple constant-time comparison
    let bytes1 = str1.as_bytes();
    let bytes2 = str2.as_bytes();

    if bytes1.len() != bytes2.len() {
        Ok(Value::Ok(Box::new(Value::Boolean(false))))
    } else {
        let mut result = 0u8;
        for i in 0..bytes1.len() {
            result |= bytes1[i] ^ bytes2[i];
        }
        Ok(Value::Ok(Box::new(Value::Boolean(result == 0))))
    }
}
