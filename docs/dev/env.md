# Development Environment
## Step 1. Rust
### 1. Ensure Environment
```bash
PS C:\Users\USER\Documents\GitHub\TS-Analyzer> rustc --version
rustc 1.97.1 (8bab26f4f 2026-07-14)
PS C:\Users\USER\Documents\GitHub\TS-Analyzer> cargo --version                               
cargo 1.97.1 (c980f4866 2026-06-30)
PS C:\Users\USER\Documents\GitHub\TS-Analyzer> rustup show active-toolchain
stable-x86_64-pc-windows-msvc (default)
PS C:\Users\USER\Documents\GitHub\TS-Analyzer> rustup component list --installed
cargo-x86_64-pc-windows-msvc
clippy-x86_64-pc-windows-msvc
rust-docs-x86_64-pc-windows-msvc
rust-std-x86_64-pc-windows-msvc
rustc-x86_64-pc-windows-msvc
rustfmt-x86_64-pc-windows-msvc
PS C:\Users\USER\Documents\GitHub\TS-Analyzer> rustup target list --installed   
x86_64-pc-windows-msvc
```

### 2. Create toolchain at root
Create `rust-toolchain.toml` at root:
```toml
[toolchain]
channel = "1.97.1"
profile = "minimal"
components = ["rustfmt", "clippy"]
targets = ["x86_64-pc-windows-msvc"]
```

### 3. Create 8 crates
#### Core
```bash
cargo new --lib --vcs none crates/tsan-core
cargo new --lib --vcs none crates/tsan-input
cargo new --lib --vcs none crates/tsan-analyzer
cargo new --lib --vcs none crates/tsan-runtime
cargo new --lib --vcs none crates/tsan-recorder
cargo new --lib --vcs none crates/tsan-player
```
#### App
```bash
cargo new --bin --vcs none crates/tsan-cli
cargo new --bin --vcs none crates/tsan-gui
```

### 4. Create cargo at root
Create `Cargo.toml` at root:
```toml
[workspace]
members = [
    "crates/tsan-core",
    "crates/tsan-input",
    "crates/tsan-analyzer",
    "crates/tsan-runtime",
    "crates/tsan-recorder",
    "crates/tsan-player",
    "crates/tsan-cli",
    "crates/tsan-gui",
]
resolver = "3"

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.97"
license = "Apache-2.0"
publish = false

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
```

### 5. Edit Cargo.toml of all crates
Edit all `Cargo.toml` of crates in the following format (for instance, tsan-core):
```toml
[package]
name = "tsan-core" # maintain crate's original name
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
```

### 6. Create lock at root
```bash
cargo generate-lockfile
```

### 7. Validate workspace and MSVC linker
```bash
# check cargo output all crates
cargo metadata --no-deps --format-version 1

# full check
cargo fmt --all -- --check
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# run tsan-cli
cargo run --locked -p tsan-cli # output "Hello, world!" means all items pass
```

### 8. Install and initialize cargo-deny
#### Initialize cargo-deny
```bash
# install
cargo install --locked cargo-deny

# check
cargo deny --version
cargo deny help

# init (generate deny.toml)
cargo deny init
```
#### remove comment in deny.toml, for this project is "Apache-2.0"
```toml
allow = [
    #"MIT",
    #"Apache-2.0",
    #"Apache-2.0 WITH LLVM-exception",
]
```

#### check deny anytime after a new crate is added
```bash
cargo deny check
```
#### view worktree
```bash
cargo tree --workspace
```
