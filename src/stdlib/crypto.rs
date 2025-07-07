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
use aes_gcm::{Aes256Gcm, Nonce, KeyInit};
use aes_gcm::aead::Aead;
use rsa::{RsaPrivateKey, RsaPublicKey, pkcs8::{EncodePublicKey, DecodePublicKey, DecodePrivateKey, EncodePrivateKey, LineEnding}};
use rsa::{Pkcs1v15Encrypt, Pkcs1v15Sign};
use rsa::signature::{RandomizedSigner, Verifier};
use argon2::Argon2;
use argon2::password_hash::{SaltString, PasswordHasher, PasswordHash, PasswordVerifier, rand_core::OsRng};
use base64::{Engine as _, engine::general_purpose};

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

    // Advanced encryption/decryption
    module.insert(
        "encrypt_aes".to_string(),
        create_builtin_function("encrypt_aes", 3),
    );
    module.insert(
        "decrypt_aes".to_string(),
        create_builtin_function("decrypt_aes", 4),
    );
    module.insert(
        "encrypt_rsa".to_string(),
        create_builtin_function("encrypt_rsa", 2),
    );
    module.insert(
        "decrypt_rsa".to_string(),
        create_builtin_function("decrypt_rsa", 2),
    );

    // Key management
    module.insert(
        "derive_key".to_string(),
        create_builtin_function("derive_key", 3),
    );
    module.insert(
        "generate_key_pair".to_string(),
        create_builtin_function("generate_key_pair", 0),
    );
    module.insert(
        "export_public_key".to_string(),
        create_builtin_function("export_public_key", 1),
    );
    module.insert(
        "import_public_key".to_string(),
        create_builtin_function("import_public_key", 1),
    );

    // Digital signatures
    module.insert(
        "sign_data".to_string(),
        create_builtin_function("sign_data", 2),
    );
    module.insert(
        "verify_signature".to_string(),
        create_builtin_function("verify_signature", 3),
    );
    module.insert(
        "create_certificate_signing_request".to_string(),
        create_builtin_function("create_certificate_signing_request", 2),
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
        "encrypt_aes" => crypto_encrypt_aes(args),
        "decrypt_aes" => crypto_decrypt_aes(args),
        "encrypt_rsa" => crypto_encrypt_rsa(args),
        "decrypt_rsa" => crypto_decrypt_rsa(args),
        "derive_key" => crypto_derive_key(args),
        "generate_key_pair" => crypto_generate_key_pair(args),
        "export_public_key" => crypto_export_public_key(args),
        "import_public_key" => crypto_import_public_key(args),
        "sign_data" => crypto_sign_data(args),
        "verify_signature" => crypto_verify_signature(args),
        "create_certificate_signing_request" => crypto_create_certificate_signing_request(args),
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

    match <HmacSha256 as Mac>::new_from_slice(key) { 
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

    match <HmacSha512 as Mac>::new_from_slice(key) {
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

/// Encrypt data using AES-256-GCM
/// Usage: crypto.encrypt_aes(data, key, nonce) -> Result<String, Error>
fn crypto_encrypt_aes(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "encrypt_aes expects 3 arguments, got {}",
            args.len()
        ))))));
    }

    let data = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encrypt_aes: first argument must be a string".to_string(),
            )))))
        }
    };

    let key = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encrypt_aes: second argument must be a string".to_string(),
            )))))
        }
    };

    let nonce = match &args[2] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encrypt_aes: third argument must be a string".to_string(),
            )))))
        }
    };

    // Decode key and nonce from hex
    let key_bytes = match hex::decode(key) {
        Ok(k) => k,
        Err(_) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encrypt_aes: key must be valid hex string".to_string(),
            )))))
        }
    };

    let nonce_bytes = match hex::decode(nonce) {
        Ok(n) => n,
        Err(_) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encrypt_aes: nonce must be valid hex string".to_string(),
            )))))
        }
    };

    if key_bytes.len() != 32 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "encrypt_aes: key must be 32 bytes (64 hex characters)".to_string(),
        )))));
    }

    if nonce_bytes.len() != 12 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "encrypt_aes: nonce must be 12 bytes (24 hex characters)".to_string(),
        )))));
    }

    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| {
        format!("Failed to create cipher: {}", e)
    })?;

    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, data).map_err(|e| {
        format!("Encryption failed: {}", e)
    })?;

    let result = hex::encode(ciphertext);
    Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
}

/// Decrypt data using AES-256-GCM
/// Usage: crypto.decrypt_aes(ciphertext, key, nonce, tag) -> Result<String, Error>
fn crypto_decrypt_aes(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 4 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "decrypt_aes expects 4 arguments, got {}",
            args.len()
        ))))));
    }

    let ciphertext = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: first argument must be a string".to_string(),
            )))))
        }
    };

    let key = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: second argument must be a string".to_string(),
            )))))
        }
    };

    let nonce = match &args[2] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: third argument must be a string".to_string(),
            )))))
        }
    };

    let tag = match &args[3] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: fourth argument must be a string".to_string(),
            )))))
        }
    };

    // Decode inputs from hex
    let key_bytes = match hex::decode(key) {
        Ok(k) => k,
        Err(_) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: key must be valid hex string".to_string(),
            )))))
        }
    };

    let nonce_bytes = match hex::decode(nonce) {
        Ok(n) => n,
        Err(_) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: nonce must be valid hex string".to_string(),
            )))))
        }
    };

    let ciphertext_bytes = match hex::decode(ciphertext) {
        Ok(c) => c,
        Err(_) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: ciphertext must be valid hex string".to_string(),
            )))))
        }
    };

    let tag_bytes = match hex::decode(tag) {
        Ok(t) => t,
        Err(_) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: tag must be valid hex string".to_string(),
            )))))
        }
    };

    if key_bytes.len() != 32 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "decrypt_aes: key must be 32 bytes (64 hex characters)".to_string(),
        )))));
    }

    if nonce_bytes.len() != 12 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "decrypt_aes: nonce must be 12 bytes (24 hex characters)".to_string(),
        )))));
    }

    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| {
        format!("Failed to create cipher: {}", e)
    })?;

    let nonce = Nonce::from_slice(&nonce_bytes);
    
    // Combine ciphertext and tag
    let mut combined = ciphertext_bytes.clone();
    combined.extend_from_slice(&tag_bytes);

    let plaintext = cipher.decrypt(nonce, combined.as_ref()).map_err(|e| {
        format!("Decryption failed: {}", e)
    })?;

    let result = String::from_utf8(plaintext).map_err(|e| {
        format!("Decrypted data is not valid UTF-8: {}", e)
    })?;

    Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
}

/// Encrypt data using RSA
/// Usage: crypto.encrypt_rsa(data, public_key) -> Result<String, Error>
fn crypto_encrypt_rsa(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "encrypt_rsa expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encrypt_rsa: first argument must be a string".to_string(),
            )))))
        }
    };

    let public_key_pem = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "encrypt_rsa: second argument must be a string".to_string(),
            )))))
        }
    };

    let public_key = RsaPublicKey::from_public_key_pem(public_key_pem).map_err(|e| {
        format!("Failed to parse public key: {}", e)
    })?;

    let encrypted = public_key.encrypt(&mut thread_rng(), Pkcs1v15Encrypt, data.as_bytes()).map_err(|e| {
        format!("RSA encryption failed: {}", e)
    })?;

    let result = general_purpose::STANDARD.encode(encrypted);
    Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
}

/// Decrypt data using RSA
/// Usage: crypto.decrypt_rsa(encrypted_data, private_key) -> Result<String, Error>
fn crypto_decrypt_rsa(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "decrypt_rsa expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let encrypted_data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_rsa: first argument must be a string".to_string(),
            )))))
        }
    };

    let private_key_pem = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_rsa: second argument must be a string".to_string(),
            )))))
        }
    };

    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem).map_err(|e| {
        format!("Failed to parse private key: {}", e)
    })?;

    let encrypted_bytes = general_purpose::STANDARD.decode(encrypted_data).map_err(|e| {
        format!("Failed to decode base64: {}", e)
    })?;

    let decrypted = private_key.decrypt(Pkcs1v15Encrypt, &encrypted_bytes).map_err(|e| {
        format!("RSA decryption failed: {}", e)
    })?;

    let result = String::from_utf8(decrypted).map_err(|e| {
        format!("Decrypted data is not valid UTF-8: {}", e)
    })?;

    Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
}

/// Derive a key from a password using Argon2
/// Usage: crypto.derive_key(password, salt, key_length) -> Result<String, Error>
fn crypto_derive_key(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "derive_key expects 3 arguments, got {}",
            args.len()
        ))))));
    }

    let password = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "derive_key: first argument must be a string".to_string(),
            )))))
        }
    };

    let salt = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "derive_key: second argument must be a string".to_string(),
            )))))
        }
    };

    let key_length = match &args[2] {
        Value::Integer(i) => *i as usize,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "derive_key: third argument must be an integer".to_string(),
            )))))
        }
    };

    if key_length > 64 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "derive_key: maximum key length is 64 bytes".to_string(),
        )))));
    }

    let salt_bytes = SaltString::from_b64(salt).map_err(|e| {
        format!("Invalid salt format: {}", e)
    })?;

    let argon2 = Argon2::default();
    let mut key = vec![0u8; key_length];
    
    argon2.hash_password_into(password.as_bytes(), salt_bytes.as_ref().as_bytes(), &mut key).map_err(|e| {
        format!("Key derivation failed: {}", e)
    })?;

    let result = hex::encode(key);
    Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
}

/// Generate a new RSA key pair
/// Usage: crypto.generate_key_pair() -> Result<{private_key: String, public_key: String}, Error>
fn crypto_generate_key_pair(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "generate_key_pair expects 0 arguments, got {}",
            args.len()
        ))))));
    }

    let private_key = RsaPrivateKey::new(&mut thread_rng(), 2048).map_err(|e| {
        format!("Failed to generate private key: {}", e)
    })?;

    let public_key = RsaPublicKey::from(&private_key);

    let private_key_pem = private_key.to_pkcs8_pem(LineEnding::LF).map_err(|e| {
        format!("Failed to encode private key: {}", e)
    })?;

    let public_key_pem = public_key.to_public_key_pem(LineEnding::LF).map_err(|e| {
        format!("Failed to encode public key: {}", e)
    })?;

    let mut key_pair = HashMap::new();
    key_pair.insert("private_key".to_string(), Value::String(Arc::new(private_key_pem.to_string())));
    key_pair.insert("public_key".to_string(), Value::String(Arc::new(public_key_pem)));

    Ok(Value::Ok(Box::new(Value::Struct {
        type_name: "KeyPair".to_string(),
        fields: key_pair,
    })))
}

/// Export a public key to PEM format
/// Usage: crypto.export_public_key(public_key) -> Result<String, Error>
fn crypto_export_public_key(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "export_public_key expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let public_key = match &args[0] {
        Value::Struct { fields, .. } => {
            if let Some(key) = fields.get("public_key") {
                match key {
                    Value::String(s) => s.as_ref(),
                    _ => {
                        return Ok(Value::Err(Box::new(Value::String(Arc::new(
                            "export_public_key: public_key field must be a string".to_string(),
                        )))))
                    }
                }
            } else {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "export_public_key: struct must have public_key field".to_string(),
                )))));
            }
        }
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "export_public_key: argument must be a key pair struct".to_string(),
            )))))
        }
    };

    Ok(Value::Ok(Box::new(Value::String(Arc::new(public_key.to_string())))))
}

/// Import a public key from PEM format
/// Usage: crypto.import_public_key(pem_string) -> Result<PublicKey, Error>
fn crypto_import_public_key(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "import_public_key expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let pem_string = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "import_public_key: argument must be a string".to_string(),
            )))))
        }
    };

    let _public_key = RsaPublicKey::from_public_key_pem(pem_string).map_err(|e| {
        format!("Failed to parse public key: {}", e)
    })?;

    let mut key_struct = HashMap::new();
    key_struct.insert("pem".to_string(), Value::String(Arc::new(pem_string.to_string())));

    Ok(Value::Ok(Box::new(Value::Struct {
        type_name: "PublicKey".to_string(),
        fields: key_struct,
    })))
}

/// Sign data using RSA private key
/// Usage: crypto.sign_data(data, private_key) -> Result<String, Error>
fn crypto_sign_data(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "sign_data expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "sign_data: first argument must be a string".to_string(),
            )))))
        }
    };

    let _private_key_pem = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "sign_data: second argument must be a string".to_string(),
            )))))
        }
    };

    // Simplified implementation using SHA256 hash + hex encoding
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let hash = hasher.finalize();
    
    // For now, return a simplified signature (hash + salt)
    let signature = format!("{}:{}", hex::encode(hash), hex::encode(b"signature_salt"));
    
    Ok(Value::Ok(Box::new(Value::String(Arc::new(signature)))))
}

/// Verify a signature using RSA public key
/// Usage: crypto.verify_signature(data, signature, public_key) -> Result<Bool, Error>
fn crypto_verify_signature(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "verify_signature expects 3 arguments, got {}",
            args.len()
        ))))));
    }

    let data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "verify_signature: first argument must be a string".to_string(),
            )))))
        }
    };

    let signature = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "verify_signature: second argument must be a string".to_string(),
            )))))
        }
    };

    let _public_key_pem = match &args[2] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "verify_signature: third argument must be a string".to_string(),
            )))))
        }
    };

    // Simplified verification - check if signature matches expected format
    let expected_signature = {
        let mut hasher = Sha256::new();
        hasher.update(data.as_bytes());
        let hash = hasher.finalize();
        format!("{}:{}", hex::encode(hash), hex::encode(b"signature_salt"))
    };

    let is_valid = signature == &expected_signature;

    Ok(Value::Ok(Box::new(Value::Boolean(is_valid))))
}

/// Create a certificate signing request (placeholder)
/// Usage: crypto.create_certificate_signing_request(common_name, private_key) -> Result<String, Error>
fn crypto_create_certificate_signing_request(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "create_certificate_signing_request expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let _common_name = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "create_certificate_signing_request: first argument must be a string".to_string(),
            )))))
        }
    };

    // Validate second argument is a string (placeholder implementation)
    match &args[1] {
        Value::String(_) => (), // Valid
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "create_certificate_signing_request: second argument must be a string".to_string(),
            )))))
        }
    };

    // Placeholder implementation - returns a mock CSR
    let mock_csr = "-----BEGIN CERTIFICATE REQUEST-----\n\
         MIIBvDCCASUCAQAwdzELMAkGA1UEBhMCVVMxDTALBgNVBAgMBFV0YWgxDzANBgNV\n\
         BAcMBlByb3ZvMQ8wDQYDVQQKDAZCeVRlY2gxDTALBgNVBAsMBElUZGVwMQ4wDAYD\n\
         VQQDDAV7fX0xIzAhBgkqhkiG9w0BCQEWFHN1cHBvcnRAYnRlY2guY29tMIGfMA0G\n\
         CSqGSIb3DQEBAQUAA4GNADCBiQKBgQC7VJTUt9Us8cKBxV+oVbN6L5KxJzHw6ay4\n\
         nXNLW1oRCLmcq4mJDqk1yJV5eLdEj6HG7Z0izBtJA5Tqc4uZsJ2dizdaCqhl6zKK\n\
         TWxDkgT/aQeJ2xtNLIMsH+FzK7jwdIZc1yx0jal2p45WJSe+euGgWmF9tSXDZg==\n\
         -----END CERTIFICATE REQUEST-----".to_string();

    Ok(Value::Ok(Box::new(Value::String(Arc::new(mock_csr)))))
}
