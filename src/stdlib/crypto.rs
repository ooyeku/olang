use crate::ast::Value;
use aes_gcm::aead::{Aead, AeadCore};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::Argon2;
#[cfg(feature = "rsa-crypto")]
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
#[cfg(feature = "rsa-crypto")]
use rsa::Pkcs1v15Encrypt;
#[cfg(feature = "rsa-crypto")]
use rsa::pkcs1v15::{
    Signature as RsaSignature, SigningKey as RsaSigningKey, VerifyingKey as RsaVerifyingKey,
};
#[cfg(feature = "rsa-crypto")]
use rsa::rand_core::OsRng as RsaOsRng;
#[cfg(feature = "rsa-crypto")]
use rsa::signature::{SignatureEncoding, Signer, Verifier};
#[cfg(feature = "rsa-crypto")]
use rsa::{
    RsaPrivateKey, RsaPublicKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding},
};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
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
    let mut module = crate::ast::ValueMap::default();

    // Hash functions
    module.insert("md5".to_string(), create_builtin_function("md5", 1));
    module.insert("sha1".to_string(), create_builtin_function("sha1", 1));
    module.insert("sha256".to_string(), create_builtin_function("sha256", 1));
    module.insert("sha512".to_string(), create_builtin_function("sha512", 1));
    module.insert("xxh3".to_string(), create_builtin_function("xxh3", 1));
    module.insert("blake3".to_string(), create_builtin_function("blake3", 1));

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

    // Ed25519 (RFC 8032): keys and signatures travel as base64, the form
    // Sparkle's appcasts use; each also accepts Bytes.
    module.insert(
        "ed25519_keypair".to_string(),
        create_builtin_function("ed25519_keypair", 0),
    );
    module.insert(
        "ed25519_public_key".to_string(),
        create_builtin_function("ed25519_public_key", 1),
    );
    module.insert(
        "ed25519_sign".to_string(),
        create_builtin_function("ed25519_sign", 2),
    );
    module.insert(
        "ed25519_verify".to_string(),
        create_builtin_function("ed25519_verify", 3),
    );

    Value::Struct {
        type_name: "Module".into(),
        fields: std::sync::Arc::new(module).into(),
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
        // XXH3-64, an Int: fast and stable, not a cryptographic hash
        "xxh3" => match args.as_slice() {
            [v] => Ok(Value::Integer(crate::value_hash::hash_bytes(digest_input(
                v, "xxh3",
            )?))),
            _ => Err(format!("xxh3 expects 1 argument, got {}", args.len()).into()),
        },
        "blake3" => match args.as_slice() {
            [v] => Ok(Value::String(Arc::new(
                blake3::hash(digest_input(v, "blake3")?)
                    .to_hex()
                    .to_string(),
            ))),
            _ => Err(format!("blake3 expects 1 argument, got {}", args.len()).into()),
        },
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
        #[cfg(feature = "rsa-crypto")]
        "encrypt_rsa" => crypto_encrypt_rsa(args),
        #[cfg(feature = "rsa-crypto")]
        "decrypt_rsa" => crypto_decrypt_rsa(args),
        "derive_key" => crypto_derive_key(args),
        "ed25519_keypair" => crypto_ed25519_keypair(args),
        "ed25519_public_key" => crypto_ed25519_public_key(args),
        "ed25519_sign" => crypto_ed25519_sign(args),
        "ed25519_verify" => crypto_ed25519_verify(args),
        #[cfg(feature = "rsa-crypto")]
        "generate_key_pair" => crypto_generate_key_pair(args),
        #[cfg(feature = "rsa-crypto")]
        "export_public_key" => crypto_export_public_key(args),
        #[cfg(feature = "rsa-crypto")]
        "import_public_key" => crypto_import_public_key(args),
        #[cfg(feature = "rsa-crypto")]
        "sign_data" => crypto_sign_data(args),
        #[cfg(feature = "rsa-crypto")]
        "verify_signature" => crypto_verify_signature(args),
        // The browser profile omits the RSA suite (a third of a megabyte
        // of code a page never calls); the names still exist, so a
        // program that reaches one hears why.
        #[cfg(not(feature = "rsa-crypto"))]
        "encrypt_rsa" | "decrypt_rsa" | "generate_key_pair" | "export_public_key"
        | "import_public_key" | "sign_data" | "verify_signature" => Err(format!(
            "crypto.{}: RSA is not in this build (the browser runtime omits it)",
            name
        )
        .into()),
        _ => Err(format!("Unknown crypto function: {}", name).into()),
    }
}

/// The bytes to hash: a String's UTF-8, or a Bytes value's contents —
/// hashing is fundamentally over bytes, so both are accepted.
fn digest_input<'a>(v: &'a Value, fname: &str) -> Result<&'a [u8], String> {
    match v {
        Value::String(s) => Ok(s.as_ref().as_bytes()),
        other => crate::stdlib::bytes::bytes_of(other)
            .map_err(|_| format!("{fname}: argument must be a string or Bytes")),
    }
}

/// Compute MD5 hash of input
/// Usage: crypto.md5("hello") -> Result<String, Error>
fn crypto_md5(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("md5 expects 1 argument, got {}", args.len()).into());
    }

    let input = digest_input(&args[0], "md5")?;

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

    let input = digest_input(&args[0], "sha1")?;

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

    let input = digest_input(&args[0], "sha256")?;

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

    let input = digest_input(&args[0], "sha512")?;

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
    fill_random(&mut bytes);

    Ok(crate::stdlib::bytes::to_value(bytes))
}

/// Fill `bytes` from the operating system's generator — or, inside a
/// simulated world (`sim`), from the running task's seeded stream, so a
/// world's `random_bytes` reproduces with its seed.
fn fill_random(bytes: &mut [u8]) {
    if let Some(stream) = crate::sim::task_rng() {
        let mut rng = stream.lock().unwrap_or_else(|e| e.into_inner());
        rng.fill_bytes(bytes);
        return;
    }
    rand::rng().fill_bytes(bytes);
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
    fill_random(&mut bytes);
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
/// A key, nonce or ciphertext that is not what AES takes: input a program
/// read (a key from its configuration), so an `Err` it can handle, not a
/// raise (stdlib.md rule 2; OL-385). A wrong argument count or type still
/// raises: that is the call's own mistake.
fn aes_bad_input(why: &str) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(why.to_string()))))
}

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
            return Ok(aes_bad_input("encrypt_aes: key must be valid hex string"));
        }
    };

    if key_bytes.len() != 32 {
        return Ok(aes_bad_input(
            "encrypt_aes: key must be 32 bytes (64 hex characters)",
        ));
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
            return Ok(aes_bad_input("decrypt_aes: key must be valid hex string"));
        }
    };

    let decoded = match hex::decode(encrypted_data) {
        Ok(c) => c,
        Err(_) => {
            return Ok(aes_bad_input(
                "decrypt_aes: encrypted_data must be valid hex string",
            ));
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
                return Ok(aes_bad_input("decrypt_aes: nonce must be valid hex string"));
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
        return Ok(aes_bad_input(
            "decrypt_aes: key must be 32 bytes (64 hex characters)",
        ));
    }

    if nonce_bytes.len() != 12 {
        return Ok(aes_bad_input(
            "decrypt_aes: nonce must be 12 bytes (24 hex characters)",
        ));
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
#[cfg(feature = "rsa-crypto")]
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
#[cfg(feature = "rsa-crypto")]
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
#[cfg(feature = "rsa-crypto")]
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

    let mut key_pair = crate::ast::ValueMap::default();
    key_pair.insert(
        "private_key".to_string(),
        Value::String(Arc::new(private_key_pem.to_string())),
    );
    key_pair.insert(
        "public_key".to_string(),
        Value::String(Arc::new(public_key_pem)),
    );

    Ok(Value::Ok(Box::new(Value::Struct {
        type_name: "KeyPair".into(),
        fields: std::sync::Arc::new(key_pair).into(),
    })))
}

/// Export a public key to PEM format
/// Usage: crypto.export_public_key(public_key) -> Result<String, Error>
#[cfg(feature = "rsa-crypto")]
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
#[cfg(feature = "rsa-crypto")]
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
            let mut key_struct = crate::ast::ValueMap::default();
            key_struct.insert(
                "pem".to_string(),
                Value::String(Arc::new(pem_string.to_string())),
            );

            Ok(Value::Ok(Box::new(Value::Struct {
                type_name: "PublicKey".into(),
                fields: std::sync::Arc::new(key_struct).into(),
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
#[cfg(feature = "rsa-crypto")]
fn crypto_sign_data(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("sign_data expects 2 arguments, got {}", args.len()).into());
    }

    let data = digest_input(&args[0], "sign_data")?;

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
    let signature = signing_key.sign(data);

    Ok(Value::Ok(Box::new(Value::String(Arc::new(hex::encode(
        signature.to_bytes(),
    ))))))
}

/// Verify a signature using RSA public key (PKCS#1 v1.5 over SHA-256)
/// Usage: crypto.verify_signature(data, signature, public_key) -> Result<Bool, Error>
#[cfg(feature = "rsa-crypto")]
fn crypto_verify_signature(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!("verify_signature expects 3 arguments, got {}", args.len()).into());
    }

    let data = digest_input(&args[0], "verify_signature")?;

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
    let is_valid = verifying_key.verify(data, &signature).is_ok();

    Ok(Value::Ok(Box::new(Value::Boolean(is_valid))))
}

// ── Ed25519 ──────────────────────────────────────────────────────────

fn crypto_err(message: String) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(message))))
}

fn b64(bytes: &[u8]) -> Value {
    use base64::Engine as _;
    Value::String(Arc::new(
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))
}

/// A key or signature of exactly `N` bytes: Bytes as they are, or a
/// String in standard base64 (with or without padding) or in hex.
fn fixed_bytes<const N: usize>(v: &Value, fname: &str, what: &str) -> Result<[u8; N], String> {
    use base64::Engine as _;
    let raw: Vec<u8> = match v {
        Value::String(s) => {
            let t = s.trim();
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(t)
                .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(t))
                .ok()
                .filter(|b| b.len() == N)
                .or_else(|| hex::decode(t).ok().filter(|b| b.len() == N));
            match decoded {
                Some(b) => b,
                None => {
                    return Err(format!(
                        "{fname}: the {what} must be {N} bytes, as Bytes or a base64 or hex string"
                    ));
                }
            }
        }
        other => crate::stdlib::bytes::bytes_of(other)
            .map_err(|_| format!("{fname}: the {what} must be Bytes or a base64 string"))?
            .to_vec(),
    };
    raw.try_into()
        .map_err(|b: Vec<u8>| format!("{fname}: the {what} must be {N} bytes, not {}", b.len()))
}

/// A new Ed25519 key pair: `Ok({private_key, public_key})`, each base64
/// (the private key is the 32-byte seed, as Sparkle's tools write it).
/// Usage: crypto.ed25519_keypair()
fn crypto_ed25519_keypair(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("ed25519_keypair expects 0 arguments, got {}", args.len()).into());
    }
    let mut seed = [0u8; 32];
    fill_random(&mut seed);
    let key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let mut pair = crate::ast::ValueMap::default();
    pair.insert("private_key".to_string(), b64(&seed));
    pair.insert(
        "public_key".to_string(),
        b64(key.verifying_key().as_bytes()),
    );
    seed.fill(0);
    Ok(Value::Ok(Box::new(Value::Struct {
        type_name: "KeyPair".into(),
        fields: Arc::new(pair).into(),
    })))
}

/// The public key of an Ed25519 private key, base64.
/// Usage: crypto.ed25519_public_key(private_key) -> Result<String, Error>
fn crypto_ed25519_public_key(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("ed25519_public_key expects 1 argument, got {}", args.len()).into());
    }
    let seed = match fixed_bytes::<32>(&args[0], "ed25519_public_key", "private key") {
        Ok(s) => s,
        Err(e) => return Ok(crypto_err(e)),
    };
    let key = ed25519_dalek::SigningKey::from_bytes(&seed);
    Ok(Value::Ok(Box::new(b64(key.verifying_key().as_bytes()))))
}

/// Sign `data` (a String's UTF-8, or Bytes) with an Ed25519 private key:
/// the 64-byte signature, base64.
/// Usage: crypto.ed25519_sign(data, private_key) -> Result<String, Error>
fn crypto_ed25519_sign(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("ed25519_sign expects 2 arguments, got {}", args.len()).into());
    }
    let data = digest_input(&args[0], "ed25519_sign")?;
    let seed = match fixed_bytes::<32>(&args[1], "ed25519_sign", "private key") {
        Ok(s) => s,
        Err(e) => return Ok(crypto_err(e)),
    };
    use ed25519_dalek::Signer as _;
    let key = ed25519_dalek::SigningKey::from_bytes(&seed);
    Ok(Value::Ok(Box::new(b64(&key.sign(data).to_bytes()))))
}

/// Whether `signature` is an Ed25519 signature of `data` by the holder of
/// `public_key`: `Ok(true)` or `Ok(false)`; `Err` only for a public key
/// that is no key. Verification is strict (RFC 8032 with the weak-key and
/// malleability checks), so a signature has one valid form.
/// Usage: crypto.ed25519_verify(data, signature, public_key) -> Result<Bool, Error>
fn crypto_ed25519_verify(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!("ed25519_verify expects 3 arguments, got {}", args.len()).into());
    }
    let data = digest_input(&args[0], "ed25519_verify")?;
    let public = match fixed_bytes::<32>(&args[2], "ed25519_verify", "public key") {
        Ok(p) => p,
        Err(e) => return Ok(crypto_err(e)),
    };
    let key = match ed25519_dalek::VerifyingKey::from_bytes(&public) {
        Ok(k) => k,
        Err(_) => {
            return Ok(crypto_err(
                "ed25519_verify: the public key is not a valid Ed25519 point".to_string(),
            ));
        }
    };
    let Ok(sig) = fixed_bytes::<64>(&args[1], "ed25519_verify", "signature") else {
        return Ok(Value::Ok(Box::new(Value::Boolean(false))));
    };
    let sig = ed25519_dalek::Signature::from_bytes(&sig);
    Ok(Value::Ok(Box::new(Value::Boolean(
        key.verify_strict(data, &sig).is_ok(),
    ))))
}

/// Verify an Ed25519 signature from Rust (the code-layer boot, OL-445):
/// raw public key, signature and message.
pub fn ed25519_verify_raw(public: &[u8; 32], signature: &[u8; 64], data: &[u8]) -> bool {
    match ed25519_dalek::VerifyingKey::from_bytes(public) {
        Ok(k) => k
            .verify_strict(data, &ed25519_dalek::Signature::from_bytes(signature))
            .is_ok(),
        Err(_) => false,
    }
}
