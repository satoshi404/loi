use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn cargo_manifest_declares_edition_2024_msrv() {
    let manifest =
        fs::read_to_string(repo_root().join("Cargo.toml")).expect("Cargo.toml should be readable");
    let declared = manifest
        .lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("rust-version")?;
            let value = value.trim().strip_prefix('=')?;
            let value = value.trim().trim_matches(['"', '\'']);
            Some(value.to_string())
        })
        .expect("Cargo.toml must declare rust-version");

    let parts: Vec<u32> = declared
        .split('.')
        .map(|part| part.parse::<u32>().expect("rust-version must be numeric"))
        .collect();
    assert_eq!(parts.len(), 2, "rust-version must be MAJOR.MINOR");
    assert!(
        (parts[0], parts[1]) >= (1, 85),
        "edition 2024 requires rust-version >= 1.85, found {declared}"
    );
}
