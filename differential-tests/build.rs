use std::{fs, io};

fn dependency_block(manifest: &str) -> io::Result<String> {
    let block = manifest
        .split_once("[dependencies]")
        .and_then(|(_, tail)| tail.split_once("[dev-dependencies]"))
        .map(|(block, _)| block)
        .ok_or_else(|| io::Error::other("expected dependencies followed by dev-dependencies"))?;
    Ok(block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n"))
}

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=../Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.toml");
    let root = fs::read_to_string("../Cargo.toml")?;
    let harness = fs::read_to_string("Cargo.toml")?;
    if dependency_block(&root)? != dependency_block(&harness)? {
        return Err(io::Error::other(
            "sync the differential-tests production dependency block with ../Cargo.toml before testing",
        ));
    }
    println!("cargo:rustc-check-cfg=cfg(ntfs_mft_differential)");
    println!("cargo:rustc-cfg=ntfs_mft_differential");
    Ok(())
}
