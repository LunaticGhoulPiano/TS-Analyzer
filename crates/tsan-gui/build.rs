use std::collections::BTreeMap;
use std::env;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = "../../packaging/platforms.toml";
    println!("cargo:rerun-if-changed={manifest}");
    let target = env::var("CARGO_CFG_TARGET_OS")?;
    let text = fs::read_to_string(manifest)?;
    let mut section = "";
    let mut fields = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
        } else if section == target
            && let Some((key, value)) = line.split_once('=')
        {
            fields.insert(key.trim(), value.trim().trim_matches('"'));
        }
    }
    let version = fields.get("version").ok_or("Missing platform version")?;
    if version.split('.').count() != 3 || !version.split('.').all(|p| p.parse::<u32>().is_ok()) {
        return Err("Invalid platform version".into());
    }
    for (key, name) in [
        ("version", "VERSION"),
        ("status", "STATUS"),
        ("minimum_os", "MINIMUM_OS"),
    ] {
        println!(
            "cargo:rustc-env=TSAN_PLATFORM_{name}={}",
            fields.get(key).ok_or("Missing platform field")?
        );
    }
    Ok(())
}
