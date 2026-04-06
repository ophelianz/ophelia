/***************************************************
** This file is part of Ophelia.
** Copyright © 2026 Viktor Luna <viktor@hystericca.dev>
** Released under the GPL License, version 3 or later.
**
** If you found a weird little bug in here, tell the cat:
** viktor@hystericca.dev
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs behave plz, we're all trying our best )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

//! Byte-range chunking in struct-of-arrays (SoA) layout.
//!
//! `ChunkList` stores chunk boundaries and state as parallel vecs rather than
//! a vec of structs - cache-friendly for scanning operations across all chunks.
//! The `split` function and this layout are protocol-agnostic: any resumable
//! download protocol that supports byte ranges can use them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ChunkStatus {
    Pending,
    Downloading,
    Finished,
    Error,
}

pub struct ChunkList {
    pub starts: Vec<u64>,
    pub ends: Vec<u64>,
    pub downloaded: Vec<u64>,
    pub statuses: Vec<ChunkStatus>,
}

impl ChunkList {
    pub fn len(&self) -> usize {
        self.starts.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }
}

pub fn split(total_size: u64, num_chunks: usize) -> ChunkList {
    if total_size == 0 {
        return ChunkList {
            starts: vec![],
            ends: vec![],
            downloaded: vec![],
            statuses: vec![],
        };
    }
    assert!(num_chunks > 0, "num_chunks must be > 0");

    let base_size = total_size / num_chunks as u64;
    let remainder = total_size % num_chunks as u64;

    let mut starts = Vec::with_capacity(num_chunks);
    let mut ends = Vec::with_capacity(num_chunks);
    let mut downloaded = Vec::with_capacity(num_chunks);
    let mut statuses = Vec::with_capacity(num_chunks);

    for i in 0..num_chunks {
        let start = i as u64 * base_size;
        let end = if i == num_chunks - 1 {
            start + base_size + remainder
        } else {
            start + base_size
        };
        starts.push(start);
        ends.push(end);
        downloaded.push(0);
        statuses.push(ChunkStatus::Pending);
    }

    ChunkList {
        starts,
        ends,
        downloaded,
        statuses,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_returns_no_chunks() {
        let chunks = split(0, 4);
        assert!(chunks.is_empty());
    }

    #[test]
    #[should_panic]
    fn zero_chunks_panics() {
        split(100, 0);
    }

    #[test]
    fn single_chunk_covers_full_file() {
        let chunks = split(500, 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks.starts[0], 0);
        assert_eq!(chunks.ends[0], 500);
    }

    #[test]
    fn even_split_no_gaps() {
        let chunks = split(1000, 4);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks.starts[0], 0);
        for i in 1..chunks.len() {
            assert_eq!(chunks.starts[i], chunks.ends[i - 1]);
        }
        assert_eq!(*chunks.ends.last().unwrap(), 1000);
    }

    #[test]
    fn remainder_goes_to_last_chunk() {
        let chunks = split(1001, 4);
        assert_eq!(*chunks.ends.last().unwrap(), 1001);
        let last_size = chunks.ends[3] - chunks.starts[3];
        let first_size = chunks.ends[0] - chunks.starts[0];
        assert!(last_size > first_size);
    }

    #[test]
    fn all_chunks_start_pending() {
        let chunks = split(1000, 8);
        assert!(chunks.statuses.iter().all(|s| *s == ChunkStatus::Pending));
    }

    #[test]
    fn all_downloaded_start_at_zero() {
        let chunks = split(1000, 8);
        assert!(chunks.downloaded.iter().all(|&d| d == 0));
    }

    #[test]
    fn total_coverage_equals_file_size() {
        let chunks = split(999, 7);
        let total: u64 = chunks
            .starts
            .iter()
            .zip(chunks.ends.iter())
            .map(|(s, e)| e - s)
            .sum();
        assert_eq!(total, 999);
    }

    #[test]
    fn vecs_are_same_length() {
        let chunks = split(2048, 5);
        let n = chunks.len();
        assert_eq!(chunks.ends.len(), n);
        assert_eq!(chunks.downloaded.len(), n);
        assert_eq!(chunks.statuses.len(), n);
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn total_coverage_always_equals_file_size(
            total_size in 1u64..10_000_000,
            num_chunks in 1usize..256,
        ) {
            let chunks = split(total_size, num_chunks);
            let total: u64 = chunks.starts.iter()
                .zip(chunks.ends.iter())
                .map(|(s, e)| e - s)
                .sum();
            assert_eq!(total, total_size);
        }

        #[test]
        fn no_gaps_between_chunks(
            total_size in 1u64..10_000_000,
            num_chunks in 1usize..256,
        ) {
            let chunks = split(total_size, num_chunks);
            assert_eq!(chunks.starts[0], 0);
            for i in 1..chunks.len() {
                assert_eq!(chunks.starts[i], chunks.ends[i - 1]);
            }
            assert_eq!(*chunks.ends.last().unwrap(), total_size);
        }

        #[test]
        fn vecs_always_same_length(
            total_size in 1u64..10_000_000,
            num_chunks in 1usize..256,
        ) {
            let chunks = split(total_size, num_chunks);
            let n = chunks.len();
            assert_eq!(chunks.ends.len(), n);
            assert_eq!(chunks.downloaded.len(), n);
            assert_eq!(chunks.statuses.len(), n);
        }
    }
}
