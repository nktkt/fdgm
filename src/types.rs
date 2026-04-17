use std::fmt;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub fn flip(self) -> Self {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
    pub fn sign(self) -> i32 {
        match self {
            Color::White => 1,
            Color::Black => -1,
        }
    }
}

/// Piece species. Includes 5D Chess with Multiverse Time Travel variant pieces:
/// Unicorn (3-axis slider), Dragon (4-axis slider), Princess (Bishop + Knight),
/// Brawn (pawn that also captures along the t-axis).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PieceKind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
    Unicorn,
    Dragon,
    Princess,
    Brawn,
}

impl PieceKind {
    pub fn letter(self) -> char {
        match self {
            PieceKind::Pawn => 'P',
            PieceKind::Knight => 'N',
            PieceKind::Bishop => 'B',
            PieceKind::Rook => 'R',
            PieceKind::Queen => 'Q',
            PieceKind::King => 'K',
            PieceKind::Unicorn => 'U',
            PieceKind::Dragon => 'D',
            PieceKind::Princess => 'S',
            PieceKind::Brawn => 'W',
        }
    }
    pub fn from_letter(c: char) -> Option<Self> {
        Some(match c.to_ascii_uppercase() {
            'P' => PieceKind::Pawn,
            'N' => PieceKind::Knight,
            'B' => PieceKind::Bishop,
            'R' => PieceKind::Rook,
            'Q' => PieceKind::Queen,
            'K' => PieceKind::King,
            'U' => PieceKind::Unicorn,
            'D' => PieceKind::Dragon,
            'S' => PieceKind::Princess,
            'W' => PieceKind::Brawn,
            _ => return None,
        })
    }
    pub fn is_pawnlike(self) -> bool {
        matches!(self, PieceKind::Pawn | PieceKind::Brawn)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub kind: PieceKind,
    pub color: Color,
    pub has_moved: bool,
}

impl Piece {
    pub fn new(kind: PieceKind, color: Color) -> Self {
        Self { kind, color, has_moved: false }
    }
    pub fn glyph(&self) -> char {
        let c = self.kind.letter();
        if self.color == Color::White { c } else { c.to_ascii_lowercase() }
    }
}

/// Super-physical board identifier (timeline, time).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BoardId {
    pub l: i32,
    pub t: i32,
}

impl fmt::Display for BoardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(L{}T{})", self.l, self.t)
    }
}

/// A 4D coordinate: board + file/rank.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Coord {
    pub l: i32,
    pub t: i32,
    pub x: i8,
    pub y: i8,
}

impl Coord {
    pub fn new(l: i32, t: i32, x: i8, y: i8) -> Self {
        Self { l, t, x, y }
    }
    pub fn board(&self) -> BoardId {
        BoardId { l: self.l, t: self.t }
    }
}

impl fmt::Display for Coord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = (b'a' + self.x as u8) as char;
        let rank = self.y + 1;
        write!(f, "(L{}T{}){}{}", self.l, self.t, file, rank)
    }
}
