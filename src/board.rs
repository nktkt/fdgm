use crate::types::{Color, Piece, PieceKind};

/// Castling rights on a single board. Indexed: [WK, WQ, BK, BQ].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CastlingRights(pub [bool; 4]);

impl CastlingRights {
    pub fn all() -> Self {
        Self([true, true, true, true])
    }
    pub fn none() -> Self {
        Self([false, false, false, false])
    }
    pub fn get(&self, side: CastleSide) -> bool {
        self.0[side as usize]
    }
    pub fn set(&mut self, side: CastleSide, v: bool) {
        self.0[side as usize] = v;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CastleSide {
    WhiteKing = 0,
    WhiteQueen = 1,
    BlackKing = 2,
    BlackQueen = 3,
}

/// A single 8×8 board snapshot at one (L, T).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Board {
    pub squares: [[Option<Piece>; 8]; 8], // squares[y][x]
    pub to_move: Color,
    pub castling: CastlingRights,
    /// If the previous move was a pawn double-push, this is the square the pawn
    /// skipped over (the en-passant target). Only valid ONE half-move.
    pub en_passant: Option<(i8, i8)>,
}

impl Board {
    pub fn empty(to_move: Color) -> Self {
        Self {
            squares: Default::default(),
            to_move,
            castling: CastlingRights::none(),
            en_passant: None,
        }
    }

    pub fn get(&self, x: i8, y: i8) -> Option<Piece> {
        if !in_bounds(x, y) {
            return None;
        }
        self.squares[y as usize][x as usize]
    }

    pub fn set(&mut self, x: i8, y: i8, p: Option<Piece>) {
        self.squares[y as usize][x as usize] = p;
    }

    /// Standard chess starting position with castling rights.
    pub fn starting_position() -> Self {
        let mut b = Self::empty(Color::White);
        b.castling = CastlingRights::all();
        use PieceKind::*;
        let back = [Rook, Knight, Bishop, Queen, King, Bishop, Knight, Rook];
        for x in 0..8 {
            b.set(x, 0, Some(Piece::new(back[x as usize], Color::White)));
            b.set(x, 1, Some(Piece::new(Pawn, Color::White)));
            b.set(x, 6, Some(Piece::new(Pawn, Color::Black)));
            b.set(x, 7, Some(Piece::new(back[x as usize], Color::Black)));
        }
        b
    }

    pub fn find_king(&self, color: Color) -> Option<(i8, i8)> {
        for y in 0..8 {
            for x in 0..8 {
                if let Some(p) = self.squares[y as usize][x as usize] {
                    if p.kind == PieceKind::King && p.color == color {
                        return Some((x, y));
                    }
                }
            }
        }
        None
    }

    pub fn ascii(&self) -> String {
        let mut s = String::new();
        for y in (0..8).rev() {
            s.push_str(&format!("{} ", y + 1));
            for x in 0..8 {
                let c = self.get(x, y).map(|p| p.glyph()).unwrap_or('.');
                s.push(c);
                s.push(' ');
            }
            s.push('\n');
        }
        s.push_str("  a b c d e f g h\n");
        s
    }
}

pub fn in_bounds(x: i8, y: i8) -> bool {
    (0..8).contains(&x) && (0..8).contains(&y)
}
