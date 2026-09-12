//! The program image: a parsed `Program` as bytes.
//!
//! `encode` serializes a program (postcard, the compact serde format)
//! behind a small header — the magic `olb2`, the format number, and the olang version that
//! wrote it. `decode` reads one back, refusing an image another version
//! produced rather than guessing at its layout.
//!
//! The image exists for the browser: the web SDK's `serve` encodes the
//! client bundle once at boot and hands the wasm runtime the image
//! instead of the source, so the boot skips the parser. `meta.encode`
//! exposes the same encoding to programs.

use crate::ast::Program;

const MAGIC: &[u8; 4] = b"olb2";
/// The image format this runtime writes and reads. Bumped when the
/// layout behind the header changes; a runtime that meets another
/// number says so and the page falls back to the source. With the
/// runtime embedded in the binary that serves the image, the two are
/// always the same build — which is why the guard is cheap to keep.
pub const FORMAT: u32 = 2;

/// What an image carries besides the program: the names of the
/// functions the writer knows are hot — a view, its actions — which the
/// loading runtime compiles at declaration instead of at first call, so
/// the first render never runs on the tree-walker (the browser's
/// counterpart of the native warm profile, `ovm::warm`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Image {
    pub program: Program,
    #[serde(default)]
    pub hot: Vec<String>,
}

/// Encode a program as an image this runtime version can load.
pub fn encode(program: &Program) -> Result<Vec<u8>, String> {
    encode_with(program, &[])
}

/// `encode`, with the hot-function hints the image carries.
pub fn encode_with(program: &Program, hot: &[String]) -> Result<Vec<u8>, String> {
    let image = Image {
        program: program.clone(),
        hot: hot.to_vec(),
    };
    let body =
        postcard::to_stdvec(&image).map_err(|e| format!("could not encode the program: {}", e))?;
    let version = crate::version::VERSION.as_bytes();
    let mut out = Vec::with_capacity(9 + version.len() + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT.to_le_bytes());
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
    /// The image is an olang image, but of a format this runtime does
    /// not read (an older or newer layout).
    FormatMismatch { image: u32, runtime: u32 },
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
            DecodeError::FormatMismatch { image, runtime } => write!(
                f,
                "program image format {} cannot be loaded by a runtime that reads format {} — the image and the runtime come from different olang builds; the page falls back to the source",
                image, runtime
            ),
            DecodeError::Corrupt(e) => write!(f, "program image is corrupt: {}", e),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode an image written by this runtime version, program only.
pub fn decode(bytes: &[u8]) -> Result<Program, DecodeError> {
    decode_image(bytes).map(|image| image.program)
}

/// Decode an image written by this runtime version, hints included.
pub fn decode_image(bytes: &[u8]) -> Result<Image, DecodeError> {
    // Format 1 (`olb1`, no format number) is recognized as an image so
    // the report names the mismatch rather than "not an image".
    if bytes.len() >= 4 && &bytes[..4] == b"olb1" {
        return Err(DecodeError::FormatMismatch {
            image: 1,
            runtime: FORMAT,
        });
    }
    if bytes.len() < 9 || &bytes[..4] != MAGIC {
        return Err(DecodeError::NotAnImage);
    }
    let format = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if format != FORMAT {
        return Err(DecodeError::FormatMismatch {
            image: format,
            runtime: FORMAT,
        });
    }
    let version_len = bytes[8] as usize;
    let rest = &bytes[9..];
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
        assert_eq!(&image[..4], b"olb2");
        assert_eq!(
            u32::from_le_bytes([image[4], image[5], image[6], image[7]]),
            FORMAT
        );
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
    fn another_format_is_refused_by_number() {
        let program = parse("println(1)");
        let mut image = encode(&program).unwrap();
        image[4..8].copy_from_slice(&(FORMAT + 1).to_le_bytes());
        match decode(&image) {
            Err(DecodeError::FormatMismatch { image, runtime }) => {
                assert_eq!(image, FORMAT + 1);
                assert_eq!(runtime, FORMAT);
            }
            other => panic!("expected a format mismatch, got {:?}", other),
        }
        // A format-1 image (`olb1`, no number) is named as such, not
        // dismissed as "not an image".
        let mut old = encode(&program).unwrap();
        old[..4].copy_from_slice(b"olb1");
        assert!(matches!(
            decode(&old),
            Err(DecodeError::FormatMismatch { image: 1, .. })
        ));
        let text = decode(&old).unwrap_err().to_string();
        assert!(text.contains("format 1"), "{text}");
    }

    #[test]
    fn another_version_is_refused_by_name() {
        let program = parse("println(1)");
        let mut image = encode(&program).unwrap();
        // Rewrite the version field to something no runtime is.
        let len = image[8] as usize;
        image.splice(9..9 + len, b"0.0.1".iter().copied());
        image[8] = 5;
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

    #[test]
    fn hot_hints_ride_in_the_image_and_a_plain_image_has_none() {
        let program = parse("fn view(s) = s\nfn update(s, a) = s\n");
        let plain = encode(&program).unwrap();
        let hinted = encode_with(&program, &["view".to_string(), "update".to_string()]).unwrap();
        assert!(hinted.len() > plain.len());
        let image = decode_image(&hinted).unwrap();
        assert_eq!(image.hot, vec!["view".to_string(), "update".to_string()]);
        assert_eq!(image.program, program);
        assert!(decode_image(&plain).unwrap().hot.is_empty());
        assert_eq!(decode(&hinted).unwrap(), program);
    }
}
