// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! The Unicode blocks: named, contiguous ranges of code points.
//!
//! The table is read from `data/Blocks.txt`, the Unicode Character Database file, which is
//! distributed under the Unicode License v3.

use std::ops::RangeInclusive;
use std::sync::OnceLock;

/// A named range of code points, inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnicodeBlock {
    /// The block's name, such as `Basic Latin`.
    pub name: &'static str,
    /// The first code point.
    pub first: u32,
    /// The last code point.
    pub last: u32,
}

impl UnicodeBlock {
    /// The most code points shown on a page, so that a block such as CJK Unified Ideographs
    /// is read and laid out in pieces.
    pub const PAGE_SIZE: u32 = 500;

    /// Every block, in code point order.
    pub fn all() -> &'static [Self] {
        static BLOCKS: OnceLock<Vec<UnicodeBlock>> = OnceLock::new();
        BLOCKS.get_or_init(|| Self::parse(include_str!("../data/Blocks.txt")))
    }

    /// The block containing `code`, if any.
    pub fn containing(code: u32) -> Option<&'static Self> {
        let blocks = Self::all();
        let idx = blocks.partition_point(|b| b.last < code);
        blocks.get(idx).filter(|b| b.first <= code)
    }

    /// The block called `name`, or `None` for an empty or unknown name.
    pub fn named(name: &str) -> Option<&'static Self> {
        Self::all().iter().find(|b| b.name == name)
    }

    /// The number of pages of [`Self::PAGE_SIZE`] code points needed to show the block.
    pub const fn page_count(&self) -> u32 {
        self.len().div_ceil(Self::PAGE_SIZE)
    }

    /// The code points of page `page`, counted from zero, which holds at most
    /// [`Self::PAGE_SIZE`] of them.
    pub fn page_codes(&self, page: u32) -> RangeInclusive<u32> {
        let first = self.first + page.min(self.page_count() - 1) * Self::PAGE_SIZE;
        first..=self.last.min(first + Self::PAGE_SIZE - 1)
    }

    /// The number of code points in the block, assigned or not.
    pub const fn len(&self) -> u32 {
        self.last - self.first + 1
    }

    /// Returns `true` if the block has no code points, which never happens.
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads lines of the form `0000..007F; Basic Latin`, ignoring comments.
    fn parse(text: &'static str) -> Vec<Self> {
        text.lines()
            .filter_map(|line| {
                let (range, name) = line.split_once(';')?;
                let (first, last) = range.trim().split_once("..")?;
                Some(Self {
                    name: name.trim(),
                    first: u32::from_str_radix(first, 16).ok()?,
                    last: u32::from_str_radix(last, 16).ok()?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_complete() {
        let blocks = UnicodeBlock::all();
        assert!(blocks.len() > 300, "{}", blocks.len());
        assert!(blocks.windows(2).all(|w| w[0].last < w[1].first));
        assert_eq!(blocks[0].name, "Basic Latin");
        assert_eq!((blocks[0].first, blocks[0].last), (0, 0x7F));
    }

    #[test]
    fn finds_the_block_of_a_code_point() {
        assert_eq!(
            UnicodeBlock::containing('A' as u32).unwrap().name,
            "Basic Latin"
        );
        assert_eq!(
            UnicodeBlock::containing('é' as u32).unwrap().name,
            "Latin-1 Supplement"
        );
        assert_eq!(
            UnicodeBlock::containing('日' as u32).unwrap().name,
            "CJK Unified Ideographs"
        );
        assert!(UnicodeBlock::containing(0x10_FFFF).is_some());
        assert!(UnicodeBlock::containing(0x11_0000).is_none());
    }

    #[test]
    fn large_blocks_are_split_into_pages() {
        let cjk = UnicodeBlock::named("CJK Unified Ideographs").unwrap();
        assert_eq!(cjk.page_count(), 42);
        assert_eq!(cjk.page_codes(0), 0x4E00..=0x4E00 + 499);
        assert_eq!(*cjk.page_codes(41).end(), cjk.last);
        assert_eq!(cjk.page_codes(99), cjk.page_codes(41));
        let latin = UnicodeBlock::named("Basic Latin").unwrap();
        assert_eq!(latin.page_count(), 1);
        assert_eq!(latin.page_codes(0), 0..=0x7F);
        assert!(UnicodeBlock::named("").is_none());
    }

    #[test]
    fn block_length_counts_both_ends() {
        let latin = UnicodeBlock::containing(0x41).unwrap();
        assert_eq!(latin.len(), 128);
        assert!(!latin.is_empty());
    }
}
