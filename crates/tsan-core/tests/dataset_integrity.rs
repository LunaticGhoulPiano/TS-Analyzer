use std::collections::HashSet;
use std::error::Error;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const SUPPORTED_MANIFEST_VERSION: u32 = 1;
const HASH_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
struct Manifest {
    manifest_version: u32,
    test_cases: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
struct TestCase {
    case_id: String,
    description: String,
    input_file: String,
    input_sha256: String,
    input_size_bytes: u64,
    packet_size_bytes: u16,
    transport: String,
    license: String,
    run_in_ci: bool,
    #[serde(default)]
    standards: Vec<String>,
    expected: Vec<ExpectedReference>,
}

#[derive(Debug, Deserialize)]
struct ExpectedReference {
    category: String,
    file: String,
    schema_version: u32,
    basis: String,
}

#[derive(Debug, Deserialize)]
struct ExpectedDocument {
    schema_version: u32,
    case_id: String,
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn workspace_root() -> io::Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            invalid_data("tsan-core is not located under the workspace crates directory")
        })
}

fn read_text(path: &Path) -> io::Result<String> {
    fs::read_to_string(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to read UTF-8 text file {}: {error}", path.display()),
        )
    })
}

fn resolve_file(base: &Path, relative: &str, allowed_root: &Path) -> io::Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        return Err(invalid_data(format!(
            "absolute paths are not allowed in the dataset manifest: {relative}"
        )));
    }

    let canonical_root = allowed_root.canonicalize()?;
    let candidate = base.join(relative_path);
    let canonical_candidate = candidate.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to resolve dataset file {}: {error}",
                candidate.display()
            ),
        )
    })?;

    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(invalid_data(format!(
            "dataset path escapes its allowed root: {relative}"
        )));
    }

    if !canonical_candidate.is_file() {
        return Err(invalid_data(format!(
            "dataset path is not a file: {}",
            canonical_candidate.display()
        )));
    }

    Ok(canonical_candidate)
}

fn is_valid_case_id(case_id: &str) -> bool {
    !case_id.is_empty()
        && !case_id.starts_with('_')
        && !case_id.ends_with('_')
        && !case_id.contains("__")
        && case_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];

    loop {
        let read_size = file.read(&mut buffer)?;
        if read_size == 0 {
            break;
        }
        hasher.update(&buffer[..read_size]);
    }

    let digest = hasher.finalize();
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}")
            .map_err(|error| invalid_data(format!("failed to format SHA-256: {error}")))?;
    }

    Ok(output)
}

fn validate_case_metadata(test_case: &TestCase) -> io::Result<()> {
    if !is_valid_case_id(&test_case.case_id) {
        return Err(invalid_data(format!(
            "invalid case_id: {}",
            test_case.case_id
        )));
    }
    if test_case.description.trim().is_empty() {
        return Err(invalid_data(format!(
            "description is empty for case {}",
            test_case.case_id
        )));
    }
    if !matches!(test_case.packet_size_bytes, 188 | 192 | 204) {
        return Err(invalid_data(format!(
            "unsupported packet_size_bytes for case {}: {}",
            test_case.case_id, test_case.packet_size_bytes
        )));
    }
    if !matches!(test_case.transport.as_str(), "file" | "udp" | "rtp") {
        return Err(invalid_data(format!(
            "unsupported transport for case {}: {}",
            test_case.case_id, test_case.transport
        )));
    }
    if test_case.license.trim().is_empty() {
        return Err(invalid_data(format!(
            "license is empty for case {}",
            test_case.case_id
        )));
    }
    if test_case.input_file.contains('\\') {
        return Err(invalid_data(format!(
            "input_file must use forward slashes for case {}",
            test_case.case_id
        )));
    }
    if test_case.run_in_ci && test_case.input_file.starts_with("local/") {
        return Err(invalid_data(format!(
            "local input cannot be required by CI for case {}",
            test_case.case_id
        )));
    }
    if test_case
        .standards
        .iter()
        .any(|standard| standard.trim().is_empty())
    {
        return Err(invalid_data(format!(
            "standards contains an empty value for case {}",
            test_case.case_id
        )));
    }
    if test_case.expected.is_empty() {
        return Err(invalid_data(format!(
            "no expected file is declared for case {}",
            test_case.case_id
        )));
    }

    Ok(())
}

fn validate_expected_reference(
    test_case: &TestCase,
    expected_reference: &ExpectedReference,
    inputs_root: &Path,
    expected_root: &Path,
) -> Result<(), Box<dyn Error>> {
    if !matches!(
        expected_reference.category.as_str(),
        "transport" | "analyzer" | "tables" | "clocks" | "tr101290" | "compliance"
    ) {
        return Err(invalid_data(format!(
            "unsupported expected category for case {}: {}",
            test_case.case_id, expected_reference.category
        ))
        .into());
    }
    if !matches!(
        expected_reference.basis.as_str(),
        "generated_specification" | "manual_validation"
    ) {
        return Err(invalid_data(format!(
            "unsupported expected basis for case {}: {}",
            test_case.case_id, expected_reference.basis
        ))
        .into());
    }

    let category_root = expected_root.join(&expected_reference.category);
    let expected_path = resolve_file(inputs_root, &expected_reference.file, &category_root)?;
    let expected_text = read_text(&expected_path)?;
    let expected_document: ExpectedDocument =
        serde_json::from_str(&expected_text).map_err(|error| {
            invalid_data(format!(
                "failed to parse expected JSON {}: {error}",
                expected_path.display()
            ))
        })?;

    if expected_document.case_id != test_case.case_id {
        return Err(invalid_data(format!(
            "case_id mismatch in {}: expected {}, found {}",
            expected_path.display(),
            test_case.case_id,
            expected_document.case_id
        ))
        .into());
    }
    if expected_document.schema_version != expected_reference.schema_version {
        return Err(invalid_data(format!(
            "schema_version mismatch in {}: manifest {}, document {}",
            expected_path.display(),
            expected_reference.schema_version,
            expected_document.schema_version
        ))
        .into());
    }

    Ok(())
}

#[test]
fn manifest_references_valid_test_data() -> Result<(), Box<dyn Error>> {
    let workspace_root = workspace_root()?;
    let inputs_root = workspace_root.join("dev_tests_data").join("inputs");
    let expected_root = workspace_root.join("dev_tests_data").join("expected");
    let manifest_path = inputs_root.join("manifest.toml");
    let manifest_text = read_text(&manifest_path)?;
    let manifest: Manifest = toml::from_str(&manifest_text).map_err(|error| {
        invalid_data(format!(
            "failed to parse manifest {}: {error}",
            manifest_path.display()
        ))
    })?;

    if manifest.manifest_version != SUPPORTED_MANIFEST_VERSION {
        return Err(invalid_data(format!(
            "unsupported manifest_version: {}",
            manifest.manifest_version
        ))
        .into());
    }
    if manifest.test_cases.is_empty() {
        return Err(invalid_data("manifest does not contain any test cases").into());
    }

    let mut case_ids = HashSet::new();
    for test_case in &manifest.test_cases {
        validate_case_metadata(test_case)?;
        if !case_ids.insert(test_case.case_id.as_str()) {
            return Err(invalid_data(format!(
                "duplicate case_id in manifest: {}",
                test_case.case_id
            ))
            .into());
        }
        if !is_valid_sha256(&test_case.input_sha256) {
            return Err(invalid_data(format!(
                "invalid input_sha256 for case {}",
                test_case.case_id
            ))
            .into());
        }

        let input_path = resolve_file(&inputs_root, &test_case.input_file, &inputs_root)?;
        let actual_size = input_path.metadata()?.len();
        if actual_size != test_case.input_size_bytes {
            return Err(invalid_data(format!(
                "input size mismatch for case {}: manifest {}, file {}",
                test_case.case_id, test_case.input_size_bytes, actual_size
            ))
            .into());
        }

        let actual_sha256 = sha256_file(&input_path)?;
        if actual_sha256 != test_case.input_sha256 {
            return Err(invalid_data(format!(
                "input SHA-256 mismatch for case {}: manifest {}, file {}",
                test_case.case_id, test_case.input_sha256, actual_sha256
            ))
            .into());
        }

        for expected_reference in &test_case.expected {
            validate_expected_reference(
                test_case,
                expected_reference,
                &inputs_root,
                &expected_root,
            )?;
        }
    }

    Ok(())
}
