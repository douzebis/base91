// SPDX-FileCopyrightText: 2026 Frederic Ruget <fred@atlant.is> (GitHub: @douzebis)
//
// SPDX-License-Identifier: MIT

//! A reader that hands out its data in chosen pieces, like a pipe or a
//! socket does (spec 0009 §2; spec 0006, bug 6).

use std::io::Read;

/// Serves `data` in consecutive reads of the given lengths (a read never
/// returns more than the next piece, even if the caller's buffer is
/// larger). After the last piece, reads return `Ok(0)`.
pub struct PieceReader {
    data: Vec<u8>,
    lengths: Vec<usize>,
    next_piece: usize,
    pos: usize,
}

impl PieceReader {
    /// `lengths` must add up to `data.len()`.
    pub fn new(data: &[u8], lengths: &[usize]) -> Self {
        assert_eq!(lengths.iter().sum::<usize>(), data.len());
        Self {
            data: data.to_vec(),
            lengths: lengths.to_vec(),
            next_piece: 0,
            pos: 0,
        }
    }
}

impl Read for PieceReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(&piece) = self.lengths.get(self.next_piece) else {
            return Ok(0);
        };
        let n = piece.min(buf.len());
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        if n == piece {
            self.next_piece += 1;
        } else {
            // The caller's buffer was smaller: the rest of this piece comes
            // with the next read.
            self.lengths[self.next_piece] -= n;
        }
        Ok(n)
    }
}
