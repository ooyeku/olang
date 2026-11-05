//! Ed25519 in `crypto` (OL-262): key pairs, signing and strict
//! verification over String or Bytes, the RFC 8032 test vectors, and the
//! RSA pair taking Bytes too.

use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn run(source: &str) -> String {
    let program = Parser::new().parse(source).expect("parses");
    match Interpreter::new().eval_program(program) {
        Ok(v) => match &v {
            olang::ast::Value::String(s) => s.to_string(),
            other => other.to_string(),
        },
        Err(e) => panic!("{source}\n{e}"),
    }
}

// RFC 8032 section 7.1, tests 1 and 2.
const SK1: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PK1: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const SIG1: &str = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
const SK2: &str = "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb";
const PK2: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";
const SIG2: &str = "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00";

#[test]
fn the_rfc_8032_vectors_sign_and_verify() {
    // keys given in hex; the signature comes back base64
    let b64 = |hex: &str| {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(hex::decode(hex).unwrap())
    };
    assert_eq!(
        run(&format!("unwrap(crypto.ed25519_sign(\"\", \"{SK1}\"))")),
        b64(SIG1)
    );
    assert_eq!(
        run(&format!("unwrap(crypto.ed25519_public_key(\"{SK1}\"))")),
        b64(PK1)
    );
    assert_eq!(
        run(&format!("unwrap(crypto.ed25519_verify(\"\", \"{SIG1}\", \"{PK1}\"))")),
        "true"
    );
    // test 2 signs the one byte 0x72, given as Bytes
    assert_eq!(
        run(&format!(
            "unwrap(crypto.ed25519_sign(bytes.from_list([114]), \"{SK2}\"))"
        )),
        b64(SIG2)
    );
    assert_eq!(
        run(&format!(
            "unwrap(crypto.ed25519_verify(bytes.from_list([114]), \"{}\", \"{}\"))",
            b64(SIG2),
            b64(PK2)
        )),
        "true"
    );
    // the wrong key or the wrong message
    assert_eq!(
        run(&format!("unwrap(crypto.ed25519_verify(\"\", \"{SIG1}\", \"{PK2}\"))")),
        "false"
    );
    assert_eq!(
        run(&format!("unwrap(crypto.ed25519_verify(\"x\", \"{SIG1}\", \"{PK1}\"))")),
        "false"
    );
}

#[test]
fn a_new_pair_signs_and_verifies_strings_and_bytes() {
    let out = run(r#"
let keys = unwrap(crypto.ed25519_keypair())
let sig = unwrap(crypto.ed25519_sign("release 1.2.0", keys.private_key))
let raw = bytes.from_string("release 1.2.0")
[
  unwrap(crypto.ed25519_verify("release 1.2.0", sig, keys.public_key)),
  unwrap(crypto.ed25519_verify(raw, sig, keys.public_key)),
  unwrap(crypto.ed25519_verify("release 1.2.1", sig, keys.public_key)),
  unwrap(crypto.ed25519_verify("release 1.2.0", "c2hvcnQ=", keys.public_key)),
  unwrap(crypto.ed25519_public_key(keys.private_key)) == keys.public_key,
  len(sig), len(keys.public_key),
  is_err(crypto.ed25519_verify("m", sig, "not a key")),
  is_err(crypto.ed25519_sign("m", "short")),
]
"#);
    assert_eq!(
        out,
        "[true, true, false, false, true, 88, 44, true, true]"
    );
}

#[test]
fn rsa_signatures_take_bytes() {
    let out = run(r#"
let keys = unwrap(crypto.generate_key_pair())
let data = bytes.from_list([0, 159, 255, 7])
let sig = unwrap(crypto.sign_data(data, keys.private_key))
[unwrap(crypto.verify_signature(data, sig, keys.public_key)),
 unwrap(crypto.verify_signature(bytes.from_list([0, 159, 255, 8]), sig, keys.public_key))]
"#);
    assert_eq!(out, "[true, false]");
}
