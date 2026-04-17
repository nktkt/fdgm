//! Minimal 5D chess notation I/O.
//!
//! Single move: `(L0T0)e2 -> (L0T0)e4` (arrow for quiet, `x` for capture).
//! Turn plan:   moves joined by `; ` (semi-colon + space).
//! User input:  the player can submit multiple moves separated by `;` — the CLI
//!              parses each into a Move.

use crate::moves::{Move, MoveKind, TurnPlan};
use crate::multiverse::Multiverse;
use crate::types::{Color, Coord, PieceKind};

pub fn fmt_coord(c: Coord) -> String {
    let file = (b'a' + c.x as u8) as char;
    format!("(L{}T{}){}{}", c.l, c.t, file, c.y + 1)
}

pub fn fmt_move(m: &Move) -> String {
    let base = match m.kind {
        MoveKind::CastleKing => "O-O".to_string(),
        MoveKind::CastleQueen => "O-O-O".to_string(),
        _ => {
            let sep = if m.capture { "x" } else { "->" };
            let promo = match m.promotion {
                Some(k) => format!("={}", k.letter()),
                None => String::new(),
            };
            format!("{} {} {}{}", fmt_coord(m.from), sep, fmt_coord(m.to), promo)
        }
    };
    base
}

pub fn fmt_turn(plan: &TurnPlan) -> String {
    plan.iter().map(fmt_move).collect::<Vec<_>>().join("; ")
}

/// Parse a coord like `(L0T0)e2`.
pub fn parse_coord(s: &str) -> Option<Coord> {
    let s = s.trim();
    let s = s.strip_prefix('(')?;
    let (prefix, rest) = s.split_once(')')?;
    let prefix = prefix.replace(' ', "");
    let l_start = prefix.find('L')?;
    let t_idx = prefix.find('T')?;
    let l_str = &prefix[l_start + 1..t_idx];
    let t_str = &prefix[t_idx + 1..];
    let l: i32 = l_str.parse().ok()?;
    let t: i32 = t_str.parse().ok()?;
    let rest = rest.trim();
    let bytes = rest.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let file = bytes[0];
    let rank = bytes[1];
    if !(b'a'..=b'h').contains(&file) || !(b'1'..=b'8').contains(&rank) {
        return None;
    }
    Some(Coord {
        l,
        t,
        x: (file - b'a') as i8,
        y: (rank - b'1') as i8,
    })
}

pub fn parse_user_move(mv: &Multiverse, s: &str, color: Color) -> Option<Move> {
    let parts: Vec<&str> = s
        .split_whitespace()
        .filter(|p| !matches!(*p, "->" | "x"))
        .collect();
    if parts.len() != 2 {
        return None;
    }
    let from = parse_coord(parts[0])?;
    let to = parse_coord(parts[1])?;
    let src_board = mv.board(from.board())?;
    let piece = src_board.get(from.x, from.y)?;
    if piece.color != color {
        return None;
    }
    let dst_piece = mv.board(to.board()).and_then(|b| b.get(to.x, to.y));
    // Castling: king moves 2 squares on its home rank.
    let home_y = if color == Color::White { 0 } else { 7 };
    let kind = if piece.kind == PieceKind::King
        && from.x == 4
        && from.y == home_y
        && from.board() == to.board()
        && to.y == home_y
        && (to.x - from.x).abs() == 2
    {
        if to.x == 6 {
            MoveKind::CastleKing
        } else {
            MoveKind::CastleQueen
        }
    } else {
        MoveKind::Normal
    };
    Some(Move {
        from,
        to,
        promotion: None,
        capture: dst_piece.is_some(),
        mover: color,
        kind,
    })
}

/// Parse a multi-move user turn, separated by `;`.
pub fn parse_user_turn(mv: &Multiverse, s: &str, color: Color) -> Option<TurnPlan> {
    let mut plan = Vec::new();
    for part in s.split(';') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let m = parse_user_move(mv, trimmed, color)?;
        plan.push(m);
    }
    if plan.is_empty() {
        None
    } else {
        Some(plan)
    }
}
