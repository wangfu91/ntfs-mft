use super::*;
use crate::NtfsFileReference;
use crate::NtfsName;
use std::path::PathBuf;
fn fid(n: u64, seq: u16) -> NtfsFileReference {
    NtfsFileReference::try_from_parts(n, seq).unwrap()
}
fn index() -> MftPathIndex {
    let mut index = MftPathIndex::new();
    for (reference, parent, name) in [
        (fid(5, 1), fid(5, 1), "."),
        (fid(10, 1), fid(5, 1), "docs"),
        (fid(20, 1), fid(10, 1), "note.txt"),
    ] {
        index
            .insert_reference(reference, parent, &NtfsName::from(name))
            .unwrap();
    }
    index
}
#[test]
fn exact_paths_require_matching_root_and_parent_generations() {
    let mut index = index();
    assert_eq!(
        index.resolve_strict(fid(20, 1)).unwrap(),
        PathBuf::from("docs").join("note.txt")
    );
    assert_eq!(index.resolve_strict(fid(5, 1)).unwrap(), PathBuf::new());
    index
        .insert_reference(fid(10, 2), fid(5, 1), &NtfsName::from("replacement"))
        .unwrap();
    assert!(matches!(
        index.resolve_strict(fid(20, 1)),
        Err(MftPathError::SequenceMismatch { .. })
    ));
    let fallback = index.resolve_best_effort(fid(20, 1));
    assert_eq!(fallback.quality, MftPathQuality::RecordFallback);
    assert!(fallback.used_record_fallback);
    assert_eq!(fallback.path, PathBuf::from("replacement").join("note.txt"));
    index
        .insert_reference(fid(5, 2), fid(5, 2), &NtfsName::from("."))
        .unwrap();
    assert!(matches!(
        index.resolve_strict(fid(10, 2)),
        Err(MftPathError::SequenceMismatch { .. })
    ));
}
#[test]
fn missing_parents_cycles_depth_and_invalid_names_are_explicit() {
    let mut index = index();
    assert!(matches!(
        index.resolve_strict(fid(30, 1)),
        Err(MftPathError::MissingReference(_))
    ));
    index
        .insert_reference(fid(10, 1), fid(30, 1), &NtfsName::from("docs"))
        .unwrap();
    let partial = index.resolve_best_effort(fid(20, 1));
    assert_eq!(partial.quality, MftPathQuality::Partial);
    assert_eq!(partial.path, PathBuf::from("docs").join("note.txt"));
    index
        .insert_reference(fid(10, 1), fid(20, 1), &NtfsName::from("docs"))
        .unwrap();
    assert!(matches!(
        index.resolve_strict(fid(20, 1)),
        Err(MftPathError::Cycle(_))
    ));
    for n in 100..400 {
        index
            .insert_reference(fid(n, 1), fid(n + 1, 1), &NtfsName::from("x"))
            .unwrap();
    }
    assert!(matches!(
        index.resolve_strict(fid(100, 1)),
        Err(MftPathError::DepthLimit)
    ));
    for name in ["..", ".", "/abs", "a/b", "a\\b"] {
        assert!(
            index
                .insert_reference(fid(30, 1), fid(5, 1), &NtfsName::from(name))
                .is_err()
        );
    }
}
