//! The program image: a parsed `Program` as bytes.
//!
//! `encode` serializes a program (postcard, the compact serde format)
//! behind a small header — the magic `olb1` and the olang version that
//! wrote it. `decode` reads one back, refusing an image another version
//! produced rather than guessing at its layout.
//!
//! The image exists for the browser: the web SDK's `serve` encodes the
//! client bundle once at boot and hands the wasm runtime the image
//! instead of the source, so the boot skips the parser. `meta.encode`
//! exposes the same encoding to programs.

use crate::ast::Program;

const MAGIC: &[u8; 4] = b"olb1";

/// Encode a program as an image this runtime version can load.
pub fn encode(program: &Program) -> Result<Vec<u8>, String> {
    let body =
        postcard::to_stdvec(program).map_err(|e| format!("could not encode the program: {}", e))?;
    let version = crate::version::VERSION.as_bytes();
    let mut out = Vec::with_capacity(5 + version.len() + body.len());
    out.extend_from_slice(MAGIC);
    out.push(version.len() as u8);
    out.extend_from_slice(version);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Why an image could not be loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    /// The bytes do not begin with the image header.
    NotAnImage,
    /// Another olang version wrote the image.
    VersionMismatch { image: String, runtime: String },
    /// The header is right but the body does not decode.
    Corrupt(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::NotAnImage => write!(f, "not an olang program image"),
            DecodeError::VersionMismatch { image, runtime } => write!(
                f,
                "program image written by olang {} cannot be loaded by olang {}",
                image, runtime
            ),
            DecodeError::Corrupt(e) => write!(f, "program image is corrupt: {}", e),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode an image written by this runtime version.
pub fn decode(bytes: &[u8]) -> Result<Program, DecodeError> {
    if bytes.len() < 5 || &bytes[..4] != MAGIC {
        return Err(DecodeError::NotAnImage);
    }
    let version_len = bytes[4] as usize;
    let rest = &bytes[5..];
    if rest.len() < version_len {
        return Err(DecodeError::NotAnImage);
    }
    let image_version = String::from_utf8_lossy(&rest[..version_len]).into_owned();
    if image_version != crate::version::VERSION {
        return Err(DecodeError::VersionMismatch {
            image: image_version,
            runtime: crate::version::VERSION.to_string(),
        });
    }
    postcard::from_bytes(&rest[version_len..]).map_err(|e| DecodeError::Corrupt(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Program {
        crate::parser::Parser::new().parse(source).expect("parses")
    }

    #[test]
    fn a_program_round_trips_through_its_image() {
        let program = parse(
            "fn area(w, h) = w * h\n\
             let xs = [1, 2, 3] |> map((x) => x * 2)\n\
             match xs { [a, b, c] => println(`${a} then ${b + c}`), _ => () }\n\
             println(to_string(area(3, 4)))",
        );
        let image = encode(&program).unwrap();
        assert_eq!(&image[..4], b"olb1");
        assert_eq!(decode(&image).unwrap(), program);
    }

    #[test]
    fn macros_are_expanded_before_encoding() {
        let program = parse("meta fn twice(e) = `${e} * 2`\nprintln(@twice(21))");
        let image = encode(&program).unwrap();
        let back = decode(&image).unwrap();
        assert_eq!(back, program);
        assert!(!format!("{:?}", back).contains("MetaFnDecl"));
    }

    #[test]
    fn another_version_is_refused_by_name() {
        let program = parse("println(1)");
        let mut image = encode(&program).unwrap();
        // Rewrite the version field to something no runtime is.
        let len = image[4] as usize;
        image.splice(5..5 + len, b"0.0.1".iter().copied());
        image[4] = 5;
        match decode(&image) {
            Err(DecodeError::VersionMismatch { image, runtime }) => {
                assert_eq!(image, "0.0.1");
                assert_eq!(runtime, crate::version::VERSION);
            }
            other => panic!("expected a version mismatch, got {:?}", other),
        }
    }

    #[test]
    fn source_text_and_truncated_images_are_not_images() {
        assert_eq!(decode(b"println(1)"), Err(DecodeError::NotAnImage));
        assert_eq!(decode(b"olb"), Err(DecodeError::NotAnImage));
        let program = parse("println(1)");
        let image = encode(&program).unwrap();
        let cut = &image[..image.len() - 3];
        assert!(matches!(decode(cut), Err(DecodeError::Corrupt(_))));
    }
}
