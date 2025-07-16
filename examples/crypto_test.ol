// examples/crypto_test.ol
// Test crypto module functionality

fn test_crypto() = {
    println("=== Testing Crypto Module ===")
    
    // Test hash functions
    let md5_result = crypto.md5("hello")
    println("MD5 hash test: " + to_string(md5_result))
    
    let sha256_result = crypto.sha256("hello")
    println("SHA256 hash test: " + to_string(sha256_result))
    
    // Test password hashing
    let password_hash = crypto.hash_password("mypassword")
    println("Password hash test: " + to_string(password_hash))
    
    // Test random generation
    let random_hex = crypto.random_hex(16)
    println("Random hex test: " + to_string(random_hex))
    
    // Test hex encoding/decoding
    let encoded = crypto.hex_encode("hello")
    println("Hex encode test: " + to_string(encoded))
    
    // Test secure comparison
    let comparison = crypto.secure_compare("secret1", "secret1")
    println("Secure compare test: " + to_string(comparison))
    
    println("=== Crypto Module Tests Complete ===")
}

fn main() = test_crypto()

// Execute test
main() 