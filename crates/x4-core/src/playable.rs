//! How much of a file can be played from its start while it downloads.

use x4_rpc::{FileRecord, Pieces};

/// The bytes at the start of a file that are already downloaded without a gap.
///
/// The file occupies `file_length` bytes from `file_offset` in the torrent's data, which is cut
/// into pieces of `piece_size` bytes. Counting starts at the piece holding the file's first byte
/// and goes on while pieces are present, so the result is the length of the prefix a player can
/// read. It is 0 when the first piece is missing, when `file_length` or `piece_size` is 0 (a
/// magnet link without metadata has no piece size yet) and never more than `file_length`.
#[must_use]
pub fn playable_prefix(
    pieces: &Pieces,
    piece_size: u64,
    file_offset: u64,
    file_length: u64,
) -> u64 {
    if piece_size == 0 || file_length == 0 {
        return 0;
    }
    let first_piece = file_offset / piece_size;
    // Counting stops at the file's own last piece, not at the end of the torrent. The saturating
    // arithmetic keeps absurd inputs from wrapping; real torrents never get near the limits.
    let last_piece = file_offset.saturating_add(file_length - 1) / piece_size;

    let mut present_end = file_offset;
    for piece in first_piece..=last_piece {
        // A piece number beyond `u32` cannot exist in the bitfield.
        let Ok(index) = u32::try_from(piece) else {
            break;
        };
        if !pieces.has(index) {
            break;
        }
        present_end = piece.saturating_add(1).saturating_mul(piece_size);
    }
    present_end.saturating_sub(file_offset).min(file_length)
}

/// The offset of every file's first byte in the torrent's data, in the order of `files`.
#[must_use]
pub fn file_offsets(files: &[FileRecord]) -> Vec<u64> {
    files
        .iter()
        .scan(0_u64, |offset, file| {
            let start = *offset;
            *offset = start.saturating_add(file.length);
            Some(start)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitfield with exactly the pieces in `present`.
    fn pieces(present: &[u32]) -> Pieces {
        let bytes = present
            .iter()
            .max()
            .map_or(0, |&last| usize::try_from(last / 8).unwrap() + 1);
        let mut bitfield = vec![0_u8; bytes];
        for &index in present {
            bitfield[usize::try_from(index / 8).unwrap()] |= 0x80 >> (index % 8);
        }
        Pieces::from_bytes(bitfield)
    }

    fn file(length: u64) -> FileRecord {
        FileRecord {
            name: "file".to_owned(),
            length,
            bytes_completed: 0,
            begin_piece: 0,
            end_piece: 0,
        }
    }

    #[test]
    fn playable_complete_file() {
        assert_eq!(playable_prefix(&pieces(&[0, 1, 2]), 4, 0, 10), 10);
        assert_eq!(playable_prefix(&pieces(&[0, 1, 2, 3, 4]), 4, 0, 20), 20);
    }

    #[test]
    fn playable_missing_first_piece() {
        assert_eq!(playable_prefix(&pieces(&[1, 2, 3]), 4, 0, 20), 0);
    }

    #[test]
    fn playable_gap_after_three() {
        assert_eq!(playable_prefix(&pieces(&[0, 1, 2]), 4, 0, 20), 12);
    }

    #[test]
    fn playable_stops_at_the_first_gap_even_if_later_pieces_are_present() {
        assert_eq!(playable_prefix(&pieces(&[0, 1, 3, 4]), 4, 0, 20), 8);
    }

    #[test]
    fn playable_file_starting_mid_piece() {
        assert_eq!(playable_prefix(&pieces(&[1, 2]), 4, 6, 10), 6);
    }

    #[test]
    fn playable_last_piece_partial() {
        assert_eq!(playable_prefix(&pieces(&[0, 1, 2]), 4, 0, 10), 10);
    }

    #[test]
    fn playable_magnet_no_metadata() {
        assert_eq!(playable_prefix(&Pieces::from_bytes(Vec::new()), 0, 0, 0), 0);
        assert_eq!(
            playable_prefix(&Pieces::from_bytes(Vec::new()), 0, 0, 10),
            0
        );
    }

    #[test]
    fn playable_empty_file_or_zero_piece_size_is_zero() {
        let all = pieces(&[0, 1, 2, 3]);

        assert_eq!(playable_prefix(&all, 4, 8, 0), 0);
        assert_eq!(playable_prefix(&all, 0, 8, 4), 0);
    }

    #[test]
    fn playable_file_in_the_middle_of_a_multi_file_torrent() {
        let files = [file(6), file(10), file(4)];
        let offsets = file_offsets(&files);
        assert_eq!(offsets, [0, 6, 16]);
        let playable = |present: &[u32], index: usize| {
            playable_prefix(&pieces(present), 4, offsets[index], files[index].length)
        };

        // The second file spans bytes 6 to 15, so pieces 1 to 3; piece 0 and piece 4 are not its.
        assert_eq!(playable(&[1, 2, 3], 1), 10);
        assert_eq!(playable(&[1, 2], 1), 6);
        assert_eq!(playable(&[0, 2, 3, 4], 1), 0);
        // Neighbours are judged by their own pieces.
        assert_eq!(playable(&[0, 1], 0), 6);
        assert_eq!(playable(&[0, 1, 2, 3], 2), 0);
        assert_eq!(playable(&[4], 2), 4);
    }

    #[test]
    fn playable_pieces_past_the_bitfield_are_absent() {
        assert_eq!(playable_prefix(&pieces(&[0]), 4, 0, 20), 4);
    }

    #[test]
    fn playable_piece_index_beyond_u32_counts_as_absent() {
        let all = Pieces::from_bytes(vec![0xff; 16]);

        assert_eq!(playable_prefix(&all, 1, 5_000_000_000, 10), 0);
    }

    #[test]
    fn playable_never_overflows() {
        let all = Pieces::from_bytes(vec![0xff; 16]);

        // The end of the file and the end of a piece both exceed `u64`.
        assert_eq!(playable_prefix(&all, u64::MAX, 0, u64::MAX), u64::MAX);
        assert_eq!(playable_prefix(&all, 1 << 63, 0, u64::MAX), u64::MAX);
        assert_eq!(playable_prefix(&all, u64::MAX, u64::MAX, 5), 0);
        assert_eq!(playable_prefix(&all, 1, u64::MAX, u64::MAX), 0);
        assert_eq!(playable_prefix(&all, 1, 0, u64::MAX), 128);
    }

    #[test]
    fn file_offsets_are_cumulative() {
        assert_eq!(file_offsets(&[file(5), file(0), file(7)]), [0, 5, 5]);
    }

    #[test]
    fn file_offsets_of_no_files_are_empty() {
        assert_eq!(file_offsets(&[]), Vec::<u64>::new());
    }

    #[test]
    fn file_offsets_saturate_instead_of_overflowing() {
        assert_eq!(
            file_offsets(&[file(u64::MAX), file(5), file(1)]),
            [0, u64::MAX, u64::MAX]
        );
    }
}
