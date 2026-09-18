fn main() {
    println!("cargo:rustc-check-cfg=cfg(ntfs_mft_differential)");
    println!("cargo:rustc-cfg=ntfs_mft_differential");
}
