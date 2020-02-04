/*
 * meli - text_processing mod.
 *
 * Copyright 2017-2020 Manos Pitsidianakis
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

pub mod grapheme_clusters;
pub mod line_break;
mod tables;
mod types;
pub use types::Reflow;
pub mod wcwidth;
pub use grapheme_clusters::*;
pub use line_break::*;
pub use wcwidth::*;

pub trait Truncate {
    fn truncate_at_boundary(self, new_len: usize);
}

impl Truncate for &mut String {
    fn truncate_at_boundary(self, new_len: usize) {
        if new_len >= self.len() {
            return;
        }

        extern crate unicode_segmentation;
        use unicode_segmentation::UnicodeSegmentation;
        if let Some((last, _)) = UnicodeSegmentation::grapheme_indices(self.as_str(), true)
            .take(new_len)
            .last()
        {
            String::truncate(self, last);
        }
    }
}

pub trait GlobMatch {
    fn matches_glob(&self, s: &str) -> bool;
    fn is_glob(&self) -> bool;
}

impl GlobMatch for str {
    fn matches_glob(&self, s: &str) -> bool {
        let parts = s.split("*");

        let mut ptr = 0;
        let mut part_no = 0;

        for p in parts {
            if p.is_empty() {
                /* asterisk is at beginning and/or end of glob */
                /* eg "*".split("*") gives ["", ""] */
                part_no += 1;
                continue;
            }

            if ptr >= self.len() {
                return false;
            }
            if part_no > 0 {
                while !&self[ptr..].starts_with(p) {
                    ptr += 1;
                    if ptr >= self.len() {
                        return false;
                    }
                }
            }
            if !&self[ptr..].starts_with(p) {
                return false;
            }
            ptr += p.len();
            part_no += 1;
        }
        true
    }

    fn is_glob(&self) -> bool {
        self.contains('*')
    }
}

#[test]
fn test_globmatch() {
    assert!("INBOX".matches_glob("INBOX"));
    assert!("INBOX/Sent".matches_glob("INBOX/*"));
    assert!(!"INBOX/Sent".matches_glob("*/Drafts"));
    assert!("INBOX/Sent".matches_glob("*/Sent"));
    assert!("INBOX/Archives/2047".matches_glob("*"));
}