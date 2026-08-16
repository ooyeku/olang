use crate::ast::Value;
use aes_gcm::aead::{Aead, AeadCore};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::Argon2;
use base64::{Engine as _, engine::general_purpose};
use bcrypt;
use hex;
use hmac::{Hmac, Mac};
use md5::Md5;
// rand 0.10 renamed the core trait RngCore -> Rng (and the ergonomic
// extension trait Rng -> RngExt); rand::rng() replaces thread_rng().
use rand::Rng as _;
// The RustCrypto crates (rsa, aes-gcm) still take rand_core 0.6 RNGs, which
// rand 0.10's generators no longer implement — their own re-exported OsRng
// bridges that era (on wasm it routes through getrandom 0.2's registered
// custom backend, see src/playground.rs).
use aes_gcm::aead::OsRng as AeadOsRng;
use rsa::Pkcs1v15Encrypt;
use rsa::pkcs1v15::{
    Signature as RsaSignature, SigningKey as RsaSigningKey, VerifyingKey as RsaVerifyingKey,
};
use rsa::rand_core::OsRng as RsaOsRng;
use rsa::signature::{SignatureEncoding, Signer, Verifier};
use rsa::{
    RsaPrivateKey, RsaPublicKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding},
};
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

    // Advanced encryption/decryption (fixed AES implementation)
    module.insert(
        "encrypt_aes".to_string(),
        create_builtin_function("encrypt_aes", 2),
    );
    module.insert(
        "decrypt_aes".to_string(),
        create_builtin_function("decrypt_aes", 3),
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
        _ => Err(format!("Unknown crypto function: {}", name).into()),
    }
}

/// Compute MD5 hash of input
/// Usage: crypto.md5("hello") -> Result<String, Error>
fn crypto_md5(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("md5 expects 1 argument, got {}", args.len()).into());
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("md5: argument must be a string".to_string().into());
        }
    };

    let result = Md5::digest(input);
    let hex_string = hex::encode(result);

    Ok(Value::String(Arc::new(hex_string)))
}

/// Compute SHA1 hash of input
/// Usage: crypto.sha1("hello") -> Result<String, Error>
fn crypto_sha1(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("sha1 expects 1 argument, got {}", args.len()).into());
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("sha1: argument must be a string".to_string().into());
        }
    };

    let result = Sha1::digest(input);
    let hex_string = hex::encode(result);

    Ok(Value::String(Arc::new(hex_string)))
}

/// Compute SHA256 hash of input
/// Usage: crypto.sha256("hello") -> Result<String, Error>
fn crypto_sha256(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("sha256 expects 1 argument, got {}", args.len()).into());
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("sha256: argument must be a string".to_string().into());
        }
    };

    let result = Sha256::digest(input);
    let hex_string = hex::encode(result);

    Ok(Value::String(Arc::new(hex_string)))
}

/// Compute SHA512 hash of input
/// Usage: crypto.sha512("hello") -> Result<String, Error>
fn crypto_sha512(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("sha512 expects 1 argument, got {}", args.len()).into());
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("sha512: argument must be a string".to_string().into());
        }
    };

    let result = Sha512::digest(input);
    let hex_string = hex::encode(result);

    Ok(Value::String(Arc::new(hex_string)))
}

/// Compute HMAC-SHA256
/// Usage: crypto.hmac_sha256("secret", "message") -> Result<String, Error>
fn crypto_hmac_sha256(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("hmac_sha256 expects 2 arguments, got {}", args.len()).into());
    }

    let key = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("hmac_sha256: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let message = match &args[1] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("hmac_sha256: second argument must be a string"
                .to_string()
                .into());
        }
    };

    type HmacSha256 = Hmac<Sha256>;

    match <HmacSha256 as Mac>::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(message);
            let result = mac.finalize();
            let hex_string = hex::encode(result.into_bytes());
            Ok(Value::String(Arc::new(hex_string)))
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
        return Err(format!("hmac_sha512 expects 2 arguments, got {}", args.len()).into());
    }

    let key = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("hmac_sha512: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let message = match &args[1] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("hmac_sha512: second argument must be a string"
                .to_string()
                .into());
        }
    };

    type HmacSha512 = Hmac<Sha512>;

    match <HmacSha512 as Mac>::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(message);
            let result = mac.finalize();
            let hex_string = hex::encode(result.into_bytes());
            Ok(Value::String(Arc::new(hex_string)))
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
        return Err(format!("hash_password expects 1 argument, got {}", args.len()).into());
    }

    let password = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("hash_password: argument must be a string"
                .to_string()
                .into());
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
        return Err(format!("verify_password expects 2 arguments, got {}", args.len()).into());
    }

    let password = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("verify_password: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let hash = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("verify_password: second argument must be a string"
                .to_string()
                .into());
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
        return Err(format!("random_bytes expects 1 argument, got {}", args.len()).into());
    }

    let count = match &args[0] {
        Value::Integer(i) => {
            if *i < 0 {
                return Err("random_bytes: count must be non-negative"
                    .to_string()
                    .into());
            }
            *i as usize
        }
        _ => {
            return Err("random_bytes: argument must be an integer"
                .to_string()
                .into());
        }
    };

    if count > 1024 {
        return Err("random_bytes: maximum 1024 bytes allowed"
            .to_string()
            .into());
    }

    let mut bytes = vec![0u8; count];
    rand::rng().fill_bytes(&mut bytes);

    let byte_values: Vec<Value> = bytes
        .into_iter()
        .map(|b| Value::Integer(b as i64))
        .collect();

    Ok(Value::List(byte_values.into()))
}

/// Generate cryptographically secure random hex string
/// Usage: crypto.random_hex(16) -> Result<String, Error>
fn crypto_random_hex(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("random_hex expects 1 argument, got {}", args.len()).into());
    }

    let count = match &args[0] {
        Value::Integer(i) => {
            if *i < 0 {
                return Err("random_hex: count must be non-negative".to_string().into());
            }
            *i as usize
        }
        _ => {
            return Err("random_hex: argument must be an integer".to_string().into());
        }
    };

    if count > 1024 {
        return Err("random_hex: maximum 1024 bytes allowed".to_string().into());
    }

    let mut bytes = vec![0u8; count];
    rand::rng().fill_bytes(&mut bytes);
    let hex_string = hex::encode(bytes);

    Ok(Value::String(Arc::new(hex_string)))
}

/// Encode bytes to hexadecimal string
/// Usage: crypto.hex_encode("hello") -> Result<String, Error>
fn crypto_hex_encode(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("hex_encode expects 1 argument, got {}", args.len()).into());
    }

    let input = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("hex_encode: argument must be a string".to_string().into());
        }
    };

    let hex_string = hex::encode(input);
    Ok(Value::String(Arc::new(hex_string)))
}

/// Decode hexadecimal string to bytes (as string)
/// Usage: crypto.hex_decode("68656c6c6f") -> Result<String, Error>
fn crypto_hex_decode(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("hex_decode expects 1 argument, got {}", args.len()).into());
    }

    let hex_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("hex_decode: argument must be a string".to_string().into());
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
        return Err(format!("secure_compare expects 2 arguments, got {}", args.len()).into());
    }

    let str1 = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("secure_compare: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let str2 = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("secure_compare: second argument must be a string"
                .to_string()
                .into());
        }
    };

    // Simple constant-time comparison
    let bytes1 = str1.as_bytes();
    let bytes2 = str2.as_bytes();

    if bytes1.len() != bytes2.len() {
        Ok(Value::Boolean(false))
    } else {
        let mut result = 0u8;
        for i in 0..bytes1.len() {
            result |= bytes1[i] ^ bytes2[i];
        }
        Ok(Value::Boolean(result == 0))
    }
}

/// Encrypt data using AES-256-GCM (FIXED IMPLEMENTATION)
/// Usage: crypto.encrypt_aes(data, key) -> Result<String, Error>
/// Returns hex-encoded ciphertext with nonce prepended
fn crypto_encrypt_aes(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("encrypt_aes expects 2 arguments, got {}", args.len()).into());
    }

    let data = match &args[0] {
        Value::String(s) => s.as_ref().as_bytes(),
        _ => {
            return Err("encrypt_aes: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let key = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("encrypt_aes: second argument must be a string"
                .to_string()
                .into());
        }
    };

    // Decode key from hex
    let key_bytes = match hex::decode(key) {
        Ok(k) => k,
        Err(_) => {
            return Err("encrypt_aes: key must be valid hex string"
                .to_string()
                .into());
        }
    };

    if key_bytes.len() != 32 {
        return Err("encrypt_aes: key must be 32 bytes (64 hex characters)"
            .to_string()
            .into());
    }

    let cipher = match Aes256Gcm::new_from_slice(&key_bytes) {
        Ok(c) => c,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to create cipher: {}",
                e
            ))))));
        }
    };

    // Generate random nonce
    let nonce = Aes256Gcm::generate_nonce(&mut AeadOsRng);

    let ciphertext = match cipher.encrypt(&nonce, data) {
        Ok(ct) => ct,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Encryption failed: {}",
                e
            ))))));
        }
    };

    // Prepend nonce to ciphertext for storage
    let mut result = nonce.to_vec();
    result.extend_from_slice(&ciphertext);

    let hex_result = hex::encode(result);
    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex_result)))))
}

/// Decrypt data using AES-256-GCM
/// Usage: crypto.decrypt_aes(encrypted_data, key) -> Result<String, Error>
///        (encrypted_data is the output of encrypt_aes: hex of nonce || ciphertext)
/// Or:    crypto.decrypt_aes(ciphertext, key, nonce) -> Result<String, Error>
///        (ciphertext and nonce as separate hex strings)
fn crypto_decrypt_aes(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 && args.len() != 3 {
        return Err(format!("decrypt_aes expects 2 or 3 arguments, got {}", args.len()).into());
    }

    let encrypted_data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("decrypt_aes: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let key = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("decrypt_aes: second argument must be a string"
                .to_string()
                .into());
        }
    };

    // Decode inputs from hex
    let key_bytes = match hex::decode(key) {
        Ok(k) => k,
        Err(_) => {
            return Err("decrypt_aes: key must be valid hex string"
                .to_string()
                .into());
        }
    };

    let decoded = match hex::decode(encrypted_data) {
        Ok(c) => c,
        Err(_) => {
            return Err("decrypt_aes: encrypted_data must be valid hex string"
                .to_string()
                .into());
        }
    };

    // With 3 args the nonce is passed separately; with 2 args it is the
    // 12 bytes encrypt_aes prepends to the ciphertext.
    let (nonce_bytes, ciphertext_bytes) = if args.len() == 3 {
        let nonce = match &args[2] {
            Value::String(s) => s.as_ref(),
            _ => {
                return Err("decrypt_aes: third argument must be a string"
                    .to_string()
                    .into());
            }
        };
        let nonce_bytes = match hex::decode(nonce) {
            Ok(n) => n,
            Err(_) => {
                return Err("decrypt_aes: nonce must be valid hex string"
                    .to_string()
                    .into());
            }
        };
        (nonce_bytes, decoded)
    } else {
        if decoded.len() < 12 {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decrypt_aes: encrypted_data too short to contain a nonce".to_string(),
            )))));
        }
        let ciphertext = decoded[12..].to_vec();
        let mut nonce_bytes = decoded;
        nonce_bytes.truncate(12);
        (nonce_bytes, ciphertext)
    };

    if key_bytes.len() != 32 {
        return Err("decrypt_aes: key must be 32 bytes (64 hex characters)"
            .to_string()
            .into());
    }

    if nonce_bytes.len() != 12 {
        return Err("decrypt_aes: nonce must be 12 bytes (24 hex characters)"
            .to_string()
            .into());
    }

    let cipher = match Aes256Gcm::new_from_slice(&key_bytes) {
        Ok(c) => c,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to create cipher: {}",
                e
            ))))));
        }
    };

    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = match cipher.decrypt(nonce, ciphertext_bytes.as_ref()) {
        Ok(pt) => pt,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Decryption failed: {}",
                e
            ))))));
        }
    };

    let result = match String::from_utf8(plaintext) {
        Ok(s) => s,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Decrypted data is not valid UTF-8: {}",
                e
            ))))));
        }
    };

    Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
}

/// Encrypt data using RSA
/// Usage: crypto.encrypt_rsa(data, public_key) -> Result<String, Error>
fn crypto_encrypt_rsa(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("encrypt_rsa expects 2 arguments, got {}", args.len()).into());
    }

    let data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("encrypt_rsa: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let public_key_pem = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("encrypt_rsa: second argument must be a string"
                .to_string()
                .into());
        }
    };

    let public_key = match RsaPublicKey::from_public_key_pem(public_key_pem) {
        Ok(key) => key,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to parse public key: {}",
                e
            ))))));
        }
    };

    let encrypted = match public_key.encrypt(&mut RsaOsRng, Pkcs1v15Encrypt, data.as_bytes()) {
        Ok(enc) => enc,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "RSA encryption failed: {}",
                e
            ))))));
        }
    };

    let result = general_purpose::STANDARD.encode(encrypted);
    Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
}

/// Decrypt data using RSA
/// Usage: crypto.decrypt_rsa(encrypted_data, private_key) -> Result<String, Error>
fn crypto_decrypt_rsa(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("decrypt_rsa expects 2 arguments, got {}", args.len()).into());
    }

    let encrypted_data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("decrypt_rsa: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let private_key_pem = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("decrypt_rsa: second argument must be a string"
                .to_string()
                .into());
        }
    };

    let private_key = match RsaPrivateKey::from_pkcs8_pem(private_key_pem) {
        Ok(key) => key,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to parse private key: {}",
                e
            ))))));
        }
    };

    let encrypted_bytes = match general_purpose::STANDARD.decode(encrypted_data) {
        Ok(bytes) => bytes,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to decode base64: {}",
                e
            ))))));
        }
    };

    let decrypted = match private_key.decrypt(Pkcs1v15Encrypt, &encrypted_bytes) {
        Ok(dec) => dec,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "RSA decryption failed: {}",
                e
            ))))));
        }
    };

    let result = match String::from_utf8(decrypted) {
        Ok(s) => s,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Decrypted data is not valid UTF-8: {}",
                e
            ))))));
        }
    };

    Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
}

/// Derive a key from a password using Argon2 (FIXED IMPLEMENTATION)
/// Usage: crypto.derive_key(password, salt, key_length) -> Result<String, Error>
fn crypto_derive_key(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!("derive_key expects 3 arguments, got {}", args.len()).into());
    }

    let password = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("derive_key: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let salt = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("derive_key: second argument must be a string"
                .to_string()
                .into());
        }
    };

    let key_length = match &args[2] {
        Value::Integer(i) => {
            if *i < 0 {
                return Err("derive_key: key length must be non-negative"
                    .to_string()
                    .into());
            }
            *i as usize
        }
        _ => {
            return Err("derive_key: third argument must be an integer"
                .to_string()
                .into());
        }
    };

    if key_length > 64 {
        return Err("derive_key: maximum key length is 64 bytes"
            .to_string()
            .into());
    }

    // Use salt as raw bytes instead of parsing as base64
    let salt_bytes = salt.as_bytes();

    let argon2 = Argon2::default();
    let mut key = vec![0u8; key_length];

    match argon2.hash_password_into(password.as_bytes(), salt_bytes, &mut key) {
        Ok(_) => {
            let result = hex::encode(key);
            Ok(Value::Ok(Box::new(Value::String(Arc::new(result)))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Key derivation failed: {}",
            e
        )))))),
    }
}

/// Generate a new RSA key pair
/// Usage: crypto.generate_key_pair() -> Result<{private_key: String, public_key: String}, Error>
fn crypto_generate_key_pair(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("generate_key_pair expects 0 arguments, got {}", args.len()).into());
    }

    let private_key = match RsaPrivateKey::new(&mut RsaOsRng, 2048) {
        Ok(key) => key,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to generate private key: {}",
                e
            ))))));
        }
    };

    let public_key = RsaPublicKey::from(&private_key);

    let private_key_pem = match private_key.to_pkcs8_pem(LineEnding::LF) {
        Ok(pem) => pem,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to encode private key: {}",
                e
            ))))));
        }
    };

    let public_key_pem = match public_key.to_public_key_pem(LineEnding::LF) {
        Ok(pem) => pem,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Failed to encode public key: {}",
                e
            ))))));
        }
    };

    let mut key_pair = HashMap::new();
    key_pair.insert(
        "private_key".to_string(),
        Value::String(Arc::new(private_key_pem.to_string())),
    );
    key_pair.insert(
        "public_key".to_string(),
        Value::String(Arc::new(public_key_pem)),
    );

    Ok(Value::Ok(Box::new(Value::Struct {
        type_name: "KeyPair".to_string(),
        fields: key_pair,
    })))
}

/// Export a public key to PEM format
/// Usage: crypto.export_public_key(public_key) -> Result<String, Error>
fn crypto_export_public_key(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("export_public_key expects 1 argument, got {}", args.len()).into());
    }

    let public_key = match &args[0] {
        Value::Struct { fields, .. } => {
            if let Some(key) = fields.get("public_key") {
                match key {
                    Value::String(s) => s.as_ref(),
                    _ => {
                        return Err("export_public_key: public_key field must be a string"
                            .to_string()
                            .into());
                    }
                }
            } else {
                return Err("export_public_key: struct must have public_key field"
                    .to_string()
                    .into());
            }
        }
        _ => {
            return Err("export_public_key: argument must be a key pair struct"
                .to_string()
                .into());
        }
    };

    Ok(Value::String(Arc::new(public_key.to_string())))
}

/// Import a public key from PEM format
/// Usage: crypto.import_public_key(pem_string) -> Result<PublicKey, Error>
fn crypto_import_public_key(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("import_public_key expects 1 argument, got {}", args.len()).into());
    }

    let pem_string = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("import_public_key: argument must be a string"
                .to_string()
                .into());
        }
    };

    // Validate the PEM string by parsing it
    match RsaPublicKey::from_public_key_pem(pem_string) {
        Ok(_) => {
            let mut key_struct = HashMap::new();
            key_struct.insert(
                "pem".to_string(),
                Value::String(Arc::new(pem_string.to_string())),
            );

            Ok(Value::Ok(Box::new(Value::Struct {
                type_name: "PublicKey".to_string(),
                fields: key_struct,
            })))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to parse public key: {}",
            e
        )))))),
    }
}

/// Sign data using RSA private key (PKCS#1 v1.5 over SHA-256)
/// Usage: crypto.sign_data(data, private_key) -> Result<String, Error>
fn crypto_sign_data(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("sign_data expects 2 arguments, got {}", args.len()).into());
    }

    let data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("sign_data: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let private_key_pem = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("sign_data: second argument must be a string"
                .to_string()
                .into());
        }
    };

    let private_key = match RsaPrivateKey::from_pkcs8_pem(private_key_pem) {
        Ok(k) => k,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "sign_data: failed to parse private key: {}",
                e
            ))))));
        }
    };

    // RSA PKCS#1 v1.5 signature over SHA-256(data)
    let signing_key = RsaSigningKey::<Sha256>::new(private_key);
    let signature = signing_key.sign(data.as_bytes());

    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex::encode(
        signature.to_bytes(),
    ))))))
}

/// Verify a signature using RSA public key (PKCS#1 v1.5 over SHA-256)
/// Usage: crypto.verify_signature(data, signature, public_key) -> Result<Bool, Error>
fn crypto_verify_signature(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!("verify_signature expects 3 arguments, got {}", args.len()).into());
    }

    let data = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("verify_signature: first argument must be a string"
                .to_string()
                .into());
        }
    };

    let signature = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("verify_signature: second argument must be a string"
                .to_string()
                .into());
        }
    };

    let public_key_pem = match &args[2] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err("verify_signature: third argument must be a string"
                .to_string()
                .into());
        }
    };

    let public_key = match RsaPublicKey::from_public_key_pem(public_key_pem) {
        Ok(k) => k,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "verify_signature: failed to parse public key: {}",
                e
            ))))));
        }
    };

    let signature_bytes = match hex::decode(signature) {
        Ok(b) => b,
        Err(_) => {
            return Err("verify_signature: signature must be a valid hex string"
                .to_string()
                .into());
        }
    };

    let signature = match RsaSignature::try_from(signature_bytes.as_slice()) {
        Ok(s) => s,
        Err(_) => return Ok(Value::Ok(Box::new(Value::Boolean(false)))),
    };

    let verifying_key = RsaVerifyingKey::<Sha256>::new(public_key);
    let is_valid = verifying_key.verify(data.as_bytes(), &signature).is_ok();

    Ok(Value::Ok(Box::new(Value::Boolean(is_valid))))
}
