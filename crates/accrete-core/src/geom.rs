//! Grid coordinates.
//!
//! Units: one tile is about 1 km x 1 km. Movement is 8-connected; a diagonal
//! step covers about 1.4 km, so it costs 14/10 of the tile's movement points
//! (see `world::movement`). Distances used by agents for estimates are octile
//! distances measured in tenths of a tile, matching that cost model.

use serde::{Deserialize, Serialize};

use crate::hash::{StableHash, StableHasher};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

impl Pos {
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    /// Chebyshev (king-move) distance in tiles.
    #[inline]
    pub fn chebyshev(self, o: Pos) -> i32 {
        (self.x - o.x).abs().max((self.y - o.y).abs())
    }

    /// Octile distance in tenths of a tile: 10 per straight step, 14 per diagonal.
    #[inline]
    pub fn octile10(self, o: Pos) -> i32 {
        let dx = (self.x - o.x).abs();
        let dy = (self.y - o.y).abs();
        let (lo, hi) = if dx < dy { (dx, dy) } else { (dy, dx) };
        10 * (hi - lo) + 14 * lo
    }

    /// Squared Euclidean distance in tiles².
    #[inline]
    pub fn dist2(self, o: Pos) -> i32 {
        let dx = self.x - o.x;
        let dy = self.y - o.y;
        dx * dx + dy * dy
    }
}

impl StableHash for Pos {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.i32(self.x);
        h.i32(self.y);
    }
}

/// The eight neighbour offsets in a fixed order: N, NE, E, SE, S, SW, W, NW.
/// The order is part of the determinism contract (it breaks ties).
pub const DIRS8: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

/// The four orthogonal neighbour offsets: N, E, S, W.
pub const DIRS4: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

#[inline]
pub fn is_diagonal(dx: i32, dy: i32) -> bool {
    dx != 0 && dy != 0
}

/// Map dimensions and index conversion. Tile indices are `u32`, row-major.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dims {
    pub width: u32,
    pub height: u32,
}

impl Dims {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    #[inline]
    pub fn len(self) -> usize {
        (self.width * self.height) as usize
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    #[inline]
    pub fn contains(self, p: Pos) -> bool {
        p.x >= 0 && p.y >= 0 && (p.x as u32) < self.width && (p.y as u32) < self.height
    }

    #[inline]
    pub fn idx(self, p: Pos) -> u32 {
        debug_assert!(self.contains(p), "{p:?} outside {self:?}");
        p.y as u32 * self.width + p.x as u32
    }

    #[inline]
    pub fn pos(self, i: u32) -> Pos {
        Pos::new((i % self.width) as i32, (i / self.width) as i32)
    }

    #[inline]
    pub fn is_border(self, p: Pos) -> bool {
        p.x == 0 || p.y == 0 || p.x as u32 == self.width - 1 || p.y as u32 == self.height - 1
    }

    /// In-bounds 8-neighbours of `p`, in `DIRS8` order.
    pub fn neighbors8(self, p: Pos) -> impl Iterator<Item = Pos> {
        DIRS8
            .iter()
            .map(move |&(dx, dy)| p.offset(dx, dy))
            .filter(move |&q| self.contains(q))
    }

    /// In-bounds 4-neighbours of `p`, in `DIRS4` order.
    pub fn neighbors4(self, p: Pos) -> impl Iterator<Item = Pos> {
        DIRS4
            .iter()
            .map(move |&(dx, dy)| p.offset(dx, dy))
            .filter(move |&q| self.contains(q))
    }
}

/// Integer Bresenham line from `a` to `b`, inclusive of both endpoints.
pub fn bresenham(a: Pos, b: Pos, mut f: impl FnMut(Pos) -> bool) {
    let dx = (b.x - a.x).abs();
    let dy = -(b.y - a.y).abs();
    let sx = if a.x < b.x { 1 } else { -1 };
    let sy = if a.y < b.y { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (a.x, a.y);
    loop {
        if !f(Pos::new(x, y)) {
            return;
        }
        if x == b.x && y == b.y {
            return;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_roundtrip() {
        let d = Dims::new(7, 5);
        for i in 0..d.len() as u32 {
            assert_eq!(d.idx(d.pos(i)), i);
        }
    }

    #[test]
    fn octile_distances() {
        let o = Pos::new(0, 0);
        assert_eq!(o.octile10(Pos::new(3, 0)), 30);
        assert_eq!(o.octile10(Pos::new(3, 3)), 42);
        assert_eq!(o.octile10(Pos::new(5, 2)), 58);
    }

    #[test]
    fn bresenham_endpoints_and_steps() {
        let mut pts = vec![];
        bresenham(Pos::new(0, 0), Pos::new(5, 2), |p| {
            pts.push(p);
            true
        });
        assert_eq!(pts.first(), Some(&Pos::new(0, 0)));
        assert_eq!(pts.last(), Some(&Pos::new(5, 2)));
        assert_eq!(pts.len(), 6); // Chebyshev distance + 1
    }
}
