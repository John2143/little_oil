//! Test-only helpers shared by in-crate tests. Compiled only under `cargo test`.
use std::path::{Path, PathBuf};

/// Every `.txt` file under `tests/example_items/`, recursively, sorted.
/// Directories named `broken` are skipped: known-unparseable pastes live there
/// so they do not fail the corpus.
pub(crate) fn example_item_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("example_items");
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().collect())
            .unwrap_or_default();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                if entry.file_name() == "broken" {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().is_some_and(|x| x == "txt") {
                out.push(path);
            }
        }
    }
    walk(&root, &mut out);
    out
}
