use crate::types::{Color, Coord, PieceKind};

/// Special classification for moves. Standard moves have Kind::Normal.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MoveKind {
    Normal,
    /// Pawn double push; sets en-passant target on successor board.
    DoublePush,
    /// Capturing the en-passant target; removes the pawn on (to.x, from.y).
    EnPassant,
    /// King-side castling (intra-board).
    CastleKing,
    /// Queen-side castling (intra-board).
    CastleQueen,
}

/// A move in the multiverse: a piece goes from `from` to `to`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    pub from: Coord,
    pub to: Coord,
    pub promotion: Option<PieceKind>,
    pub capture: bool,
    pub mover: Color,
    pub kind: MoveKind,
}

impl Move {
    pub fn is_cross_board(&self) -> bool {
        self.from.board() != self.to.board()
    }
}

/// A complete turn: one or more moves making up a single player's move sequence.
/// All moves have `mover == global_to_move` at turn start. After all moves are
/// applied, the global turn flips.
pub type TurnPlan = Vec<Move>;
