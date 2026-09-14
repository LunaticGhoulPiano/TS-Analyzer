# Create a ts file to test

## 1. Create a synthetic ts by rust

Create `crates/tsan-core/examples/generate_test_data.rs` that will generate 16 valid 188-byte Null Packets:

  - Sync byte: `0x47`
  - PID: `0x1FFF`
  - Payload-only
  - 188 bytes per packet

## 2. Validate

### Generate test ts

```bash
# run
cargo run --locked -p tsan-core --example generate_test_data

# outputs
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.01s
     Running `target\debug\examples\generate_test_data.exe`
verified existing 3008-byte file at C:\Users\USER\Documents\GitHub\TS-Analyzer\dev_tests_data\inputs\synthetic\transport_detect_packet_size_188.ts
```

### Check ts

```bash
# run: check existance
ls -l dev_tests_data/inputs/synthetic/transport_detect_packet_size_188.ts
# outputs
-rw-r--r-- 1 USER 197121 3008 Sep 14 14:22 dev_tests_data/inputs/synthetic/transport_detect_packet_size_188.ts

# run: check size
wc -c dev_tests_data/inputs/synthetic/transport_detect_packet_size_188.ts
# outputs
3008 dev_tests_data/inputs/synthetic/transport_detect_packet_size_188.ts

# run: check first 2 packet headers
od -An -tx1 -N4 dev_tests_data/inputs/synthetic/transport_detect_packet_size_188.ts
od -An -tx1 -j188 -N4 dev_tests_data/inputs/synthetic/transport_detect_packet_size_188.ts
# outputs
47 1f ff 10
47 1f ff 11
```

### Calculate SHA-256

```bash
# run
sha256sum dev_tests_data/inputs/synthetic/transport_detect_packet_size_188.ts

#outputs
c4bff2003e1f0d2da05617b0812989a21b9b0bd3008657d6753f84dd94c83a94 *dev_tests_data/inputs/synthetic/transport_detect_packet_size_188.ts
```

## 3. Create transport expected template

### 1. Create template:

`dev_tests_data/expected/transport/expected.template.json`

```json
{
  "schema_version": 1,
  "case_id": "<case_id>",
  "packet_size_bytes": 0,
  "packet_count": 0,
  "sync_loss_event_count": 0,
  "pid_packet_counts": [
    {
      "pid": 0,
      "packet_count": 0
    }
  ]
}
```

This template is not read as a test case.

### 2. Create the actual expected json file in the template format

`dev_tests_data/expected/transport/transport_detect_packet_size_188.json`

```json
{
  "schema_version": 1,
  "case_id": "transport_detect_packet_size_188",
  "packet_size_bytes": 188,
  "packet_count": 16,
  "sync_loss_event_count": 0,
  "pid_packet_counts": [
    {
      "pid": 8191,
      "packet_count": 16
    }
  ]
}
```

## 4. Create input manifest

### 1. Create template

Create `dev_tests_data/inputs/manifest.template.toml` as the field template. Automated tests do not read this file.

### 2. Create actual manifest

Replace `dev_tests_data/inputs/manifest.toml` with the following:

```toml
manifest_version = 1

[[test_cases]]
case_id = "transport_detect_packet_size_188"
description = "Detect 188-byte packet size from a synchronized null-packet stream."
input_file = "synthetic/transport_detect_packet_size_188.ts"
input_sha256 = "c4bff2003e1f0d2da05617b0812989a21b9b0bd3008657d6753f84dd94c83a94"
input_size_bytes = 3008
packet_size_bytes = 188
transport = "file"
license = "Apache-2.0"
run_in_ci = true
standards = ["mpeg-ts"]

[[test_cases.expected]]
category = "transport"
file = "../expected/transport/transport_detect_packet_size_188.json"
schema_version = 1
basis = "generated_specification"
```

## 5. Add dependencies for test

Add these dependencies for `crates/tsan-core/Cargo.toml`:

```bash
# run
cargo add -p tsan-core --dev serde --features derive
cargo add -p tsan-core --dev toml
cargo add -p tsan-core --dev serde_json
cargo add -p tsan-core --dev sha2
```

Then test build dependencies:

```bash
cargo check -p tsan-core --tests --locked
```

## 6. Create dataset integrity test

Create `crates/tsan-core/tests/dataset_integrity.rs`.

It validates:

  - Manifest version and unique case IDs
  - Input and expected paths
  - Input file size and SHA-256
  - Packet size and required metadata
  - Expected JSON case ID and schema version
  - Dataset path containment

Run:

```bash
cargo test --locked -p tsan-core --test dataset_integrity
```

Output:

```text
cargo test: 1 passed (1 suite, 0.00s)
```

## 7. Validate workspace

```bash
cargo fmt --all -- --check
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo deny check
```

All commands passed.
