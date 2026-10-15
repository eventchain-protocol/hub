//! The reconciliation rule: what startup finds when it puts the head sidecar
//! beside the AOF. A pure decision over three numbers and a tail, so that
//! its four outcomes can be checked without a file.

/// What the sidecar and the file say together.
#[derive(Debug, PartialEq, Eq)]
pub enum Reconcile {
    /// The file is exactly the committed prefix.
    Consistent,
    /// The file is shorter than the committed prefix: bytes a receipt vouched for are gone.
    FileShorter,
    /// Bytes beyond the committed prefix do not end in a line terminator: a write was cut.
    TornTail,
    /// Whole lines beyond the committed prefix: the crash window between append and sidecar.
    Replay,
}

/// `tail` is every byte of the file beyond `committed_length`, empty when the file is no longer.
pub fn reconcile(committed_length: u64, file_length: u64, tail: &[u8]) -> Reconcile {
    if file_length < committed_length {
        Reconcile::FileShorter
    } else if file_length == committed_length {
        Reconcile::Consistent
    } else if tail.last() != Some(&b'\n') {
        Reconcile::TornTail
    } else {
        Reconcile::Replay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_outcomes() {
        let cases: [(u64, u64, &[u8], Reconcile); 5] = [
            (10, 10, b"", Reconcile::Consistent),
            (0, 0, b"", Reconcile::Consistent),
            (10, 4, b"", Reconcile::FileShorter),
            (10, 14, b"{..}", Reconcile::TornTail),
            (10, 15, b"{..}\n", Reconcile::Replay),
        ];
        for (committed, file, tail, want) in cases {
            assert_eq!(reconcile(committed, file, tail), want, "committed {committed}, file {file}, tail {tail:?}");
        }
    }
}
