//! Position evaluation in centipawns, from White's perspective.
//!
//! Components (in order of weight):
//!   * Material + PST on every latest board, active timelines full weight / inactive
//!     at 30%.
//!   * 5D: timeline-count advantage (each extra timeline worth ~15 cp).
//!   * 5D: present-T advance — the side whose boards are at the present dominates;
//!     we reward material that sits on the present rather than behind it.
//!   * 5D: active-timeline count bonus (activity premium).
//!   * Classical: bishop pair, rook on 7th, passed-/doubled-/isolated-pawn, king
//!     shelter, tempo.
//!
//! Expensive pseudo-move mobility has been removed — at deep depth it dominated
//! cost. The PST already encodes most of the same signal.

use crate::board::Board;
use crate::multiverse::Multiverse;
use crate::types::{Color, PieceKind};

pub fn piece_value(kind: PieceKind) -> i32 {
    match kind {
        PieceKind::Pawn => 100,
        PieceKind::Brawn => 150,
        PieceKind::Knight => 320,
        PieceKind::Bishop => 330,
        PieceKind::Rook => 500,
        // Unicorn: 3-axis slider, less mobile on a single board than a bishop but
        // extremely powerful across timelines. Practice sets it near a rook.
        PieceKind::Unicorn => 500,
        // Dragon: 4-axis slider, rarely useful inside one board but dominant in 5D.
        PieceKind::Dragon => 550,
        // Princess: B + N compound.
        PieceKind::Princess => 650,
        PieceKind::Queen => 900,
        PieceKind::King => 20_000,
    }
}

// Piece-square tables (White perspective, y=0 is rank 1, y=7 is rank 8).
#[rustfmt::skip]
const PST_PAWN: [[i32; 8]; 8] = [
    [  0,  0,  0,  0,  0,  0,  0,  0],
    [  5, 10, 10,-20,-20, 10, 10,  5],
    [  5, -5,-10,  0,  0,-10, -5,  5],
    [  0,  0,  0, 20, 20,  0,  0,  0],
    [  5,  5, 10, 25, 25, 10,  5,  5],
    [ 10, 10, 20, 30, 30, 20, 10, 10],
    [ 50, 50, 50, 50, 50, 50, 50, 50],
    [  0,  0,  0,  0,  0,  0,  0,  0],
];

#[rustfmt::skip]
const PST_KNIGHT: [[i32; 8]; 8] = [
    [-50,-40,-30,-30,-30,-30,-40,-50],
    [-40,-20,  0,  5,  5,  0,-20,-40],
    [-30,  5, 10, 15, 15, 10,  5,-30],
    [-30,  0, 15, 20, 20, 15,  0,-30],
    [-30,  5, 15, 20, 20, 15,  5,-30],
    [-30,  0, 10, 15, 15, 10,  0,-30],
    [-40,-20,  0,  0,  0,  0,-20,-40],
    [-50,-40,-30,-30,-30,-30,-40,-50],
];

#[rustfmt::skip]
const PST_BISHOP: [[i32; 8]; 8] = [
    [-20,-10,-10,-10,-10,-10,-10,-20],
    [-10,  5,  0,  0,  0,  0,  5,-10],
    [-10, 10, 10, 10, 10, 10, 10,-10],
    [-10,  0, 10, 10, 10, 10,  0,-10],
    [-10,  5,  5, 10, 10,  5,  5,-10],
    [-10,  0,  5, 10, 10,  5,  0,-10],
    [-10,  0,  0,  0,  0,  0,  0,-10],
    [-20,-10,-10,-10,-10,-10,-10,-20],
];

#[rustfmt::skip]
const PST_ROOK: [[i32; 8]; 8] = [
    [  0,  0,  5, 10, 10,  5,  0,  0],
    [ -5,  0,  0,  0,  0,  0,  0, -5],
    [ -5,  0,  0,  0,  0,  0,  0, -5],
    [ -5,  0,  0,  0,  0,  0,  0, -5],
    [ -5,  0,  0,  0,  0,  0,  0, -5],
    [ -5,  0,  0,  0,  0,  0,  0, -5],
    [  5, 10, 10, 10, 10, 10, 10,  5],
    [  0,  0,  0,  0,  0,  0,  0,  0],
];

#[rustfmt::skip]
const PST_QUEEN: [[i32; 8]; 8] = [
    [-20,-10,-10, -5, -5,-10,-10,-20],
    [-10,  0,  5,  0,  0,  0,  0,-10],
    [-10,  5,  5,  5,  5,  5,  0,-10],
    [  0,  0,  5,  5,  5,  5,  0, -5],
    [ -5,  0,  5,  5,  5,  5,  0, -5],
    [-10,  0,  5,  5,  5,  5,  0,-10],
    [-10,  0,  0,  0,  0,  0,  0,-10],
    [-20,-10,-10, -5, -5,-10,-10,-20],
];

#[rustfmt::skip]
const PST_KING_MID: [[i32; 8]; 8] = [
    [ 20, 30, 10,  0,  0, 10, 30, 20],
    [ 20, 20,  0,  0,  0,  0, 20, 20],
    [-10,-20,-20,-20,-20,-20,-20,-10],
    [-20,-30,-30,-40,-40,-30,-30,-20],
    [-30,-40,-40,-50,-50,-40,-40,-30],
    [-30,-40,-40,-50,-50,-40,-40,-30],
    [-30,-40,-40,-50,-50,-40,-40,-30],
    [-30,-40,-40,-50,-50,-40,-40,-30],
];

fn pst(kind: PieceKind, color: Color, x: i8, y: i8) -> i32 {
    let (x, y) = (x as usize, y as usize);
    let y_table = if color == Color::White { y } else { 7 - y };
    match kind {
        PieceKind::Pawn => PST_PAWN[y_table][x],
        PieceKind::Brawn => PST_PAWN[y_table][x] + 10,
        PieceKind::Knight => PST_KNIGHT[y_table][x],
        PieceKind::Bishop => PST_BISHOP[y_table][x],
        PieceKind::Rook => PST_ROOK[y_table][x],
        PieceKind::Queen => PST_QUEEN[y_table][x],
        // Variant sliders: reuse Queen PST (they all benefit from central control).
        PieceKind::Unicorn | PieceKind::Dragon => PST_QUEEN[y_table][x],
        PieceKind::Princess => PST_BISHOP[y_table][x] + PST_KNIGHT[y_table][x] / 2,
        PieceKind::King => PST_KING_MID[y_table][x],
    }
}

fn eval_board_for(board: &Board, color: Color) -> i32 {
    let mut score = 0i32;
    let mut bishop_count = 0;
    let mut pawn_files = [0u8; 8];
    for y in 0..8i8 {
        for x in 0..8i8 {
            if let Some(p) = board.get(x, y) {
                if p.color == color {
                    score += piece_value(p.kind);
                    score += pst(p.kind, color, x, y);
                    match p.kind {
                        PieceKind::Bishop => bishop_count += 1,
                        PieceKind::Pawn | PieceKind::Brawn => pawn_files[x as usize] += 1,
                        PieceKind::Rook => {
                            let seventh = if color == Color::White { 6 } else { 1 };
                            if y == seventh {
                                score += 20;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if bishop_count >= 2 {
        score += 30;
    }
    for x in 0..8 {
        if pawn_files[x] >= 2 {
            score -= 15 * (pawn_files[x] as i32 - 1);
        }
        if pawn_files[x] > 0 {
            let left = if x > 0 { pawn_files[x - 1] } else { 0 };
            let right = if x < 7 { pawn_files[x + 1] } else { 0 };
            if left == 0 && right == 0 {
                score -= 12;
            }
        }
    }
    // Passed pawn: a pawn whose file and neighbor files have no enemy pawn ahead.
    for y in 0..8i8 {
        for x in 0..8i8 {
            if let Some(p) = board.get(x, y) {
                if p.color == color
                    && matches!(p.kind, PieceKind::Pawn | PieceKind::Brawn)
                    && is_passed_pawn(board, x, y, color)
                {
                    let rank_from_start = if color == Color::White { y } else { 7 - y };
                    score += 10 + 5 * rank_from_start as i32;
                }
            }
        }
    }
    score
}

fn is_passed_pawn(board: &Board, x: i8, y: i8, color: Color) -> bool {
    let step = if color == Color::White { 1 } else { -1 };
    let target_end = if color == Color::White { 7 } else { 0 };
    let mut yy = y + step;
    while yy != target_end + step {
        for dx in [-1i8, 0, 1] {
            let xx = x + dx;
            if !(0..8).contains(&xx) {
                continue;
            }
            if let Some(p) = board.get(xx, yy) {
                if p.color != color && matches!(p.kind, PieceKind::Pawn | PieceKind::Brawn) {
                    return false;
                }
            }
        }
        yy += step;
    }
    true
}

fn king_shelter(board: &Board, color: Color) -> i32 {
    let Some((kx, ky)) = board.find_king(color) else {
        return -5000;
    };
    let pawn_y = match color {
        Color::White => ky + 1,
        Color::Black => ky - 1,
    };
    let mut shelter = 0;
    for dx in [-1i8, 0, 1] {
        let px = kx + dx;
        if !(0..8).contains(&px) || !(0..8).contains(&pawn_y) {
            continue;
        }
        if let Some(p) = board.get(px, pawn_y) {
            if p.kind == PieceKind::Pawn && p.color == color {
                shelter += 8;
            }
        }
    }
    shelter
}

pub fn evaluate(mv: &Multiverse) -> i32 {
    let mut score = 0i32;
    let present = mv.present_t();
    for (&l, tl) in &mv.timelines {
        let active = mv.is_active(l);
        let mult = if active { 100 } else { 30 };
        let Some(board) = tl.get_board(tl.latest_t()) else { continue };
        let w = eval_board_for(board, Color::White) * mult / 100;
        let b = eval_board_for(board, Color::Black) * mult / 100;
        score += w - b;
        if active {
            score += king_shelter(board, Color::White);
            score -= king_shelter(board, Color::Black);
        }
        // Present-advance: boards at the present contribute more fully; those
        // at the present matter most for the immediate branching frontier.
        if active && tl.latest_t() == present {
            let frontier_bonus = if board.to_move == Color::White { -5 } else { 5 };
            score += frontier_bonus;
        }
    }
    // Timeline count advantage.
    let (wt, bt) = mv.timeline_counts();
    score += (wt - bt) * 20;

    // Active-timeline count (separate from just-created count — may overlap).
    let mut w_active = 0i32;
    let mut b_active = 0i32;
    for &l in mv.timelines.keys() {
        if mv.is_active(l) {
            if l > 0 {
                w_active += 1;
            } else if l < 0 {
                b_active += 1;
            }
        }
    }
    score += (w_active - b_active) * 8;

    // Tempo.
    score += match mv.global_to_move {
        Color::White => 10,
        Color::Black => -10,
    };
    score
}
