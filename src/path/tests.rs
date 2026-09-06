use super::InMemoryDirTree;
use crate::Fid;

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn in_memory_tree_resolve_four_deep_path() {
    // Layout:
    //   5 (root)  -> "Users" (10) -> "alice" (20) -> "docs" (30) -> "todo.txt" (40)
    let mut tree = InMemoryDirTree::default();
    tree.insert(10, 5, &utf16("Users"));
    tree.insert(20, 10, &utf16("alice"));
    tree.insert(30, 20, &utf16("docs"));
    tree.insert(40, 30, &utf16("todo.txt"));

    let p = tree.resolve(Fid::new(40)).expect("resolved");
    // Without drive prefix, components join with the platform separator.
    assert_eq!(
        p.to_string_lossy().replace('/', "\\"),
        "Users\\alice\\docs\\todo.txt"
    );

    let p = tree
        .resolve_with_drive_letter(Fid::new(40), 'c')
        .expect("with drive");
    assert_eq!(p.to_string_lossy(), "C:\\Users\\alice\\docs\\todo.txt");
}

#[test]
fn in_memory_tree_cycle_detection() {
    let mut tree = InMemoryDirTree::default();
    // 10 -> 11 -> 10 (cycle)
    tree.insert(10, 11, &utf16("a"));
    tree.insert(11, 10, &utf16("b"));
    assert!(tree.resolve(Fid::new(10)).is_none());
}

#[test]
fn in_memory_tree_missing_fid_returns_none() {
    let tree = InMemoryDirTree::default();
    assert!(tree.resolve(Fid::new(0xDEADBEEF)).is_none());
}

#[test]
fn in_memory_tree_fid_with_sequence_bits_is_masked() {
    let mut tree = InMemoryDirTree::default();
    tree.insert(10, 5, &utf16("hello"));
    // Lookup with the high 16 bits set (sequence number) must
    // still resolve to the same record.
    let fid = (0x0123u64 << 48) | 10;
    assert!(tree.resolve(Fid::new(fid)).is_some());
}

#[test]
fn builder_in_memory_tree_empty_tree() {
    let tree = InMemoryDirTree::default();
    assert!(tree.is_empty());
    assert!(tree.resolve(Fid::new(0xDEADBEEF)).is_none());
}
