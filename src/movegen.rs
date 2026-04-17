//! Move generation and move application for the 4-axis multiverse.
//!
//! Axes: x (file), y (rank), t (time), l (timeline).
//! Piece movement rules generalize as follows:
//!   Rook:   step along exactly 1 axis (±1 per step, must be pure).
//!   Bishop: step along exactly 2 axes (±1 on two axes simultaneously).
//!   Queen:  step along ≥1 axes equally (union of R, B, and higher-dim analogues).
//!   King:   like Queen but exactly one step.
//!   Knight: (±2 on one axis) combined with (±1 on a different axis); jumps obstacles.
//!   Pawn:   forward one step on y OR on t (capture diagonally on (y,x), (y,t), (y,l));
//!           on its starting rank it may move 2 squares forward along y.
//!
//! The timeline step is in units of timeline INDEX; we only step to timelines that
//! currently exist in the multiverse.

use std::sync::OnceLock;

use crate::board::{in_bounds, Board, CastleSide};
use crate::moves::{Move, MoveKind, TurnPlan};
use crate::multiverse::{Multiverse, Timeline};
use crate::types::{BoardId, Color, Coord, Piece, PieceKind};

// --------------------------------------------------------------------------
// Direction tables (cached — `directions()` is deterministic and called in
// every ray walk, so regenerating a Vec each time is a hot-path allocation).
// --------------------------------------------------------------------------

static DIRS_K1: OnceLock<Vec<[i32; 4]>> = OnceLock::new();
static DIRS_K2: OnceLock<Vec<[i32; 4]>> = OnceLock::new();
static DIRS_K3: OnceLock<Vec<[i32; 4]>> = OnceLock::new();
static DIRS_K4: OnceLock<Vec<[i32; 4]>> = OnceLock::new();
static DIRS_ALL: OnceLock<Vec<[i32; 4]>> = OnceLock::new();
static KNIGHT_DIRS: OnceLock<Vec<[i32; 4]>> = OnceLock::new();

fn dirs_cached(k_min: u32, k_max: u32) -> &'static [[i32; 4]] {
    if k_min == k_max {
        match k_min {
            1 => DIRS_K1.get_or_init(|| directions(1, 1)).as_slice(),
            2 => DIRS_K2.get_or_init(|| directions(2, 2)).as_slice(),
            3 => DIRS_K3.get_or_init(|| directions(3, 3)).as_slice(),
            4 => DIRS_K4.get_or_init(|| directions(4, 4)).as_slice(),
            _ => unreachable!("direction k out of range"),
        }
    } else {
        assert!(k_min == 1 && k_max == 4, "unsupported direction range");
        DIRS_ALL.get_or_init(|| directions(1, 4)).as_slice()
    }
}

fn knight_dirs_cached() -> &'static [[i32; 4]] {
    KNIGHT_DIRS
        .get_or_init(|| {
            let mut out = Vec::with_capacity(48);
            for a2 in 0..4 {
                for a1 in 0..4 {
                    if a1 == a2 {
                        continue;
                    }
                    for &s2 in &[-1i32, 1] {
                        for &s1 in &[-1i32, 1] {
                            let mut d = [0i32; 4];
                            d[a2] = 2 * s2;
                            d[a1] = s1;
                            out.push(d);
                        }
                    }
                }
            }
            out
        })
        .as_slice()
}

// --------------------------------------------------------------------------
// Primitive helpers
// --------------------------------------------------------------------------

fn piece_at(mv: &Multiverse, c: Coord) -> Option<Piece> {
    mv.board(c.board()).and_then(|b| b.get(c.x, c.y))
}

fn board_exists(mv: &Multiverse, l: i32, t: i32) -> bool {
    mv.timelines
        .get(&l)
        .map(|tl| tl.get_board(t).is_some())
        .unwrap_or(false)
}

fn step_vec(c: Coord, d: [i32; 4]) -> Coord {
    Coord {
        l: c.l + d[3],
        t: c.t + d[2],
        x: c.x + d[0] as i8,
        y: c.y + d[1] as i8,
    }
}

fn reachable(mv: &Multiverse, c: Coord) -> bool {
    if !in_bounds(c.x, c.y) {
        return false;
    }
    board_exists(mv, c.l, c.t)
}

/// All 4D direction vectors with entries in {-1, 0, 1}, non-zero, and having
/// `k` non-zero components where min_axes ≤ k ≤ max_axes.
fn directions(min_axes: u32, max_axes: u32) -> Vec<[i32; 4]> {
    let mut out = Vec::new();
    for bits in 0u32..16 {
        let k = bits.count_ones();
        if k < min_axes || k > max_axes {
            continue;
        }
        let axes: Vec<usize> = (0..4usize).filter(|&i| (bits >> i) & 1 == 1).collect();
        for sign_bits in 0u32..(1u32 << k) {
            let mut d = [0i32; 4];
            for (j, &a) in axes.iter().enumerate() {
                d[a] = if (sign_bits >> j) & 1 == 1 { 1 } else { -1 };
            }
            out.push(d);
        }
    }
    out
}

// --------------------------------------------------------------------------
// Pseudo-move generation
// --------------------------------------------------------------------------

/// Pseudo-legal moves for `color`: honors piece-movement rules and source-board
/// constraints, but NOT king-safety.
pub fn generate_pseudo_moves(mv: &Multiverse, color: Color) -> Vec<Move> {
    generate_pseudo_moves_filtered(mv, color, true)
}

/// As above but for the opponent when generating threats (ignores `to_move` filter).
pub fn generate_pseudo_moves_any_turn(mv: &Multiverse, color: Color) -> Vec<Move> {
    generate_pseudo_moves_filtered(mv, color, false)
}

fn generate_pseudo_moves_filtered(mv: &Multiverse, color: Color, enforce_to_move: bool) -> Vec<Move> {
    let mut moves = Vec::new();
    let source_boards: Vec<(BoardId, &Board)> = mv
        .timelines
        .iter()
        .filter_map(|(&l, tl)| {
            let t = tl.latest_t();
            let b = tl.get_board(t)?;
            if !mv.is_active(l) {
                return None;
            }
            if enforce_to_move && b.to_move != color {
                return None;
            }
            Some((BoardId { l, t }, b))
        })
        .collect();

    for (bid, board) in source_boards {
        for y in 0..8 {
            for x in 0..8 {
                let Some(piece) = board.get(x, y) else { continue };
                if piece.color != color {
                    continue;
                }
                let src = Coord::new(bid.l, bid.t, x, y);
                match piece.kind {
                    PieceKind::Pawn => gen_pawn(mv, board, &mut moves, src, piece),
                    PieceKind::Brawn => gen_brawn(mv, board, &mut moves, src, piece),
                    PieceKind::Knight => gen_knight(mv, &mut moves, src, piece),
                    PieceKind::Bishop => gen_slider(mv, &mut moves, src, piece, 2, 2),
                    PieceKind::Rook => gen_slider(mv, &mut moves, src, piece, 1, 1),
                    PieceKind::Queen => gen_slider(mv, &mut moves, src, piece, 1, 4),
                    PieceKind::Unicorn => gen_slider(mv, &mut moves, src, piece, 3, 3),
                    PieceKind::Dragon => gen_slider(mv, &mut moves, src, piece, 4, 4),
                    PieceKind::Princess => {
                        gen_slider(mv, &mut moves, src, piece, 2, 2);
                        gen_knight(mv, &mut moves, src, piece);
                    }
                    PieceKind::King => {
                        gen_king(mv, &mut moves, src, piece);
                        if enforce_to_move {
                            gen_castling(mv, board, &mut moves, src, piece);
                        }
                    }
                }
            }
        }
    }
    moves
}

fn gen_slider(
    mv: &Multiverse,
    out: &mut Vec<Move>,
    src: Coord,
    piece: Piece,
    min_axes: u32,
    max_axes: u32,
) {
    for &d in dirs_cached(min_axes, max_axes) {
        let mut cur = src;
        loop {
            cur = step_vec(cur, d);
            if !reachable(mv, cur) {
                break;
            }
            let target_piece = piece_at(mv, cur);
            match target_piece {
                Some(p) if p.color == piece.color => break,
                Some(_) => {
                    out.push(mk_simple(src, cur, piece, true));
                    break;
                }
                None => {
                    out.push(mk_simple(src, cur, piece, false));
                }
            }
        }
    }
}

fn gen_king(mv: &Multiverse, out: &mut Vec<Move>, src: Coord, piece: Piece) {
    for &d in dirs_cached(1, 4) {
        let cur = step_vec(src, d);
        if !reachable(mv, cur) {
            continue;
        }
        match piece_at(mv, cur) {
            Some(p) if p.color == piece.color => continue,
            Some(_) => out.push(mk_simple(src, cur, piece, true)),
            None => out.push(mk_simple(src, cur, piece, false)),
        }
    }
}

fn gen_castling(mv: &Multiverse, board: &Board, out: &mut Vec<Move>, src: Coord, piece: Piece) {
    // Castling only on the source board itself (intra-board), and only from the
    // standard king squares: e1 (x=4, y=0) for White, e8 (x=4, y=7) for Black.
    let (home_y, king_side, queen_side) = match piece.color {
        Color::White => (0i8, CastleSide::WhiteKing, CastleSide::WhiteQueen),
        Color::Black => (7i8, CastleSide::BlackKing, CastleSide::BlackQueen),
    };
    if src.x != 4 || src.y != home_y {
        return;
    }
    // In-check kings cannot castle.
    if in_check(mv, piece.color) {
        return;
    }
    // King-side: squares f1/g1 empty, right has K-side right, rook on h1, king doesn't pass through attacked squares.
    if board.castling.get(king_side)
        && board.get(5, home_y).is_none()
        && board.get(6, home_y).is_none()
        && matches!(board.get(7, home_y), Some(p) if p.kind == PieceKind::Rook && p.color == piece.color)
        && !square_attacked_on_board(mv, src.board(), 5, home_y, piece.color.flip())
        && !square_attacked_on_board(mv, src.board(), 6, home_y, piece.color.flip())
    {
        let mut m = mk_simple(src, Coord::new(src.l, src.t, 6, home_y), piece, false);
        m.kind = MoveKind::CastleKing;
        out.push(m);
    }
    // Queen-side: b1/c1/d1 empty, Q-side right, rook on a1, king doesn't pass through attacked squares.
    if board.castling.get(queen_side)
        && board.get(1, home_y).is_none()
        && board.get(2, home_y).is_none()
        && board.get(3, home_y).is_none()
        && matches!(board.get(0, home_y), Some(p) if p.kind == PieceKind::Rook && p.color == piece.color)
        && !square_attacked_on_board(mv, src.board(), 2, home_y, piece.color.flip())
        && !square_attacked_on_board(mv, src.board(), 3, home_y, piece.color.flip())
    {
        let mut m = mk_simple(src, Coord::new(src.l, src.t, 2, home_y), piece, false);
        m.kind = MoveKind::CastleQueen;
        out.push(m);
    }
}

fn gen_knight(mv: &Multiverse, out: &mut Vec<Move>, src: Coord, piece: Piece) {
    for &d in knight_dirs_cached() {
        let cur = step_vec(src, d);
        if !reachable(mv, cur) {
            continue;
        }
        match piece_at(mv, cur) {
            Some(p) if p.color == piece.color => continue,
            Some(_) => out.push(mk_simple(src, cur, piece, true)),
            None => out.push(mk_simple(src, cur, piece, false)),
        }
    }
}

fn gen_pawn(mv: &Multiverse, board: &Board, out: &mut Vec<Move>, src: Coord, piece: Piece) {
    let fwd = piece.color.sign();
    // Forward one on y or on t.
    let forward_pushes: [[i32; 4]; 2] = [[0, fwd, 0, 0], [0, 0, fwd, 0]];
    for d in forward_pushes {
        let cur = step_vec(src, d);
        if reachable(mv, cur) && piece_at(mv, cur).is_none() {
            push_with_promo(out, src, cur, piece, false, MoveKind::Normal);
            // Double push on y only from the starting rank.
            if !piece.has_moved && d == [0, fwd, 0, 0] {
                let cur2 = step_vec(cur, d);
                if reachable(mv, cur2) && piece_at(mv, cur2).is_none() {
                    let mut m = mk_simple(src, cur2, piece, false);
                    m.kind = MoveKind::DoublePush;
                    out.push(m);
                }
            }
        }
    }
    // Captures: diagonals containing forward-y.
    let capture_deltas: [[i32; 4]; 6] = [
        [1, fwd, 0, 0],
        [-1, fwd, 0, 0],
        [0, fwd, 1, 0],
        [0, fwd, -1, 0],
        [0, fwd, 0, 1],
        [0, fwd, 0, -1],
    ];
    for d in capture_deltas {
        let cur = step_vec(src, d);
        if !reachable(mv, cur) {
            continue;
        }
        if let Some(p) = piece_at(mv, cur) {
            if p.color != piece.color {
                push_with_promo(out, src, cur, piece, true, MoveKind::Normal);
            }
        }
    }
    // En-passant (intra-board only — the `to` coord is constructed with the
    // pawn's own (l, t) so the capture is always on the same board).
    if let Some((ex, ey)) = board.en_passant {
        let left = (src.x - 1, src.y + fwd as i8);
        let right = (src.x + 1, src.y + fwd as i8);
        if (left == (ex, ey) || right == (ex, ey)) && in_bounds(ex, ey) {
            let to = Coord::new(src.l, src.t, ex, ey);
            let mut m = mk_simple(src, to, piece, true);
            m.kind = MoveKind::EnPassant;
            out.push(m);
        }
    }
}

/// Brawn: like a Pawn but with capture-diagonals on every 2-axis pair involving
/// the forward direction (y or t). Also able to push forward along t on its first
/// move (no double-push on y beyond the pawn's first move).
fn gen_brawn(mv: &Multiverse, board: &Board, out: &mut Vec<Move>, src: Coord, piece: Piece) {
    let fwd = piece.color.sign();
    // Quiet pushes: one step forward on y OR t.
    for d in [[0, fwd, 0, 0], [0, 0, fwd, 0]] {
        let cur = step_vec(src, d);
        if reachable(mv, cur) && piece_at(mv, cur).is_none() {
            push_with_promo(out, src, cur, piece, false, MoveKind::Normal);
            if !piece.has_moved {
                let cur2 = step_vec(cur, d);
                if reachable(mv, cur2) && piece_at(mv, cur2).is_none() {
                    let mut m = mk_simple(src, cur2, piece, false);
                    m.kind = MoveKind::DoublePush;
                    out.push(m);
                }
            }
        }
    }
    // Capture diagonals: every 2-axis combination that includes a forward axis (y or t).
    let forward_axes: [usize; 2] = [1, 2]; // y=1, t=2
    for &fa in &forward_axes {
        for other in 0..4usize {
            if other == fa {
                continue;
            }
            for &so in &[-1i32, 1] {
                let mut d = [0i32; 4];
                d[fa] = fwd;
                d[other] = so;
                let cur = step_vec(src, d);
                if !reachable(mv, cur) {
                    continue;
                }
                if let Some(p) = piece_at(mv, cur) {
                    if p.color != piece.color {
                        push_with_promo(out, src, cur, piece, true, MoveKind::Normal);
                    }
                }
            }
        }
    }
    // En-passant (intra-board only): same as pawn.
    if let Some((ex, ey)) = board.en_passant {
        let left = (src.x - 1, src.y + fwd as i8);
        let right = (src.x + 1, src.y + fwd as i8);
        if (left == (ex, ey) || right == (ex, ey)) && in_bounds(ex, ey) {
            let to = Coord::new(src.l, src.t, ex, ey);
            let mut m = mk_simple(src, to, piece, true);
            m.kind = MoveKind::EnPassant;
            out.push(m);
        }
    }
}

fn push_with_promo(
    out: &mut Vec<Move>,
    src: Coord,
    dst: Coord,
    piece: Piece,
    cap: bool,
    kind: MoveKind,
) {
    let last_rank = if piece.color == Color::White { 7 } else { 0 };
    if dst.y == last_rank && piece.kind.is_pawnlike() {
        for promo in [PieceKind::Queen, PieceKind::Rook, PieceKind::Bishop, PieceKind::Knight] {
            let mut m = mk_simple(src, dst, piece, cap);
            m.promotion = Some(promo);
            m.kind = kind;
            out.push(m);
        }
    } else {
        let mut m = mk_simple(src, dst, piece, cap);
        m.kind = kind;
        out.push(m);
    }
}

fn mk_simple(from: Coord, to: Coord, piece: Piece, capture: bool) -> Move {
    Move {
        from,
        to,
        promotion: None,
        capture,
        mover: piece.color,
        kind: MoveKind::Normal,
    }
}

// --------------------------------------------------------------------------
// Attack detection (for castling and check)
// --------------------------------------------------------------------------

fn square_attacked_on_board(mv: &Multiverse, bid: BoardId, x: i8, y: i8, by: Color) -> bool {
    is_attacked(mv, Coord::new(bid.l, bid.t, x, y), by)
}

// --------------------------------------------------------------------------
// Move application
// --------------------------------------------------------------------------

/// Apply a single move to the multiverse WITHOUT flipping the global turn.
/// Used as the building block for multi-move turns.
pub fn apply_move_partial(mv: &Multiverse, m: Move) -> Multiverse {
    let mut out = mv.clone();

    let src_board = out.board(m.from.board()).expect("source board missing").clone();
    let moving_piece = src_board.get(m.from.x, m.from.y).expect("no piece at source");
    let mut piece_moved = moving_piece;
    piece_moved.has_moved = true;
    if let Some(promo) = m.promotion {
        piece_moved.kind = promo;
    }

    match m.kind {
        MoveKind::CastleKing | MoveKind::CastleQueen => {
            let mut succ = src_board.clone();
            let home_y = m.from.y;
            let (rook_from_x, rook_to_x, king_to_x) = if m.kind == MoveKind::CastleKing {
                (7, 5, 6)
            } else {
                (0, 3, 2)
            };
            let rook = succ.get(rook_from_x, home_y).unwrap();
            let mut moved_rook = rook;
            moved_rook.has_moved = true;
            succ.set(m.from.x, home_y, None);
            succ.set(rook_from_x, home_y, None);
            succ.set(king_to_x, home_y, Some(piece_moved));
            succ.set(rook_to_x, home_y, Some(moved_rook));
            // Revoke both castling rights for this color.
            let (ks, qs) = match moving_piece.color {
                Color::White => (CastleSide::WhiteKing, CastleSide::WhiteQueen),
                Color::Black => (CastleSide::BlackKing, CastleSide::BlackQueen),
            };
            succ.castling.set(ks, false);
            succ.castling.set(qs, false);
            succ.en_passant = None;
            succ.to_move = moving_piece.color.flip();
            out.timelines.get_mut(&m.from.l).unwrap().push_board(succ);
            return out;
        }
        _ => {}
    }

    if !m.is_cross_board() {
        // Intra-board move: create ONE successor on source timeline.
        let mut succ = src_board.clone();
        succ.set(m.from.x, m.from.y, None);
        succ.set(m.to.x, m.to.y, Some(piece_moved));
        // En-passant capture: also remove the pawn on (to.x, from.y).
        if m.kind == MoveKind::EnPassant {
            succ.set(m.to.x, m.from.y, None);
        }
        // En-passant target on successor.
        succ.en_passant = if m.kind == MoveKind::DoublePush {
            Some((m.to.x, (m.from.y + m.to.y) / 2))
        } else {
            None
        };
        update_castling_rights(&mut succ.castling, m.from.x, m.from.y, moving_piece);
        update_castling_rights_on_capture(&mut succ.castling, m.to.x, m.to.y);
        succ.to_move = moving_piece.color.flip();
        out.timelines.get_mut(&m.from.l).unwrap().push_board(succ);
    } else {
        // Cross-board move: source progresses with piece removed.
        let mut src_succ = src_board.clone();
        src_succ.set(m.from.x, m.from.y, None);
        src_succ.en_passant = None;
        update_castling_rights(&mut src_succ.castling, m.from.x, m.from.y, moving_piece);
        src_succ.to_move = moving_piece.color.flip();
        out.timelines.get_mut(&m.from.l).unwrap().push_board(src_succ);

        let dst_board = out.board(m.to.board()).expect("dest board missing").clone();
        let mut dst_succ = dst_board.clone();
        dst_succ.set(m.to.x, m.to.y, Some(piece_moved));
        dst_succ.en_passant = None;
        update_castling_rights_on_capture(&mut dst_succ.castling, m.to.x, m.to.y);
        dst_succ.to_move = moving_piece.color.flip();

        let dst_is_latest = out.is_latest_on_timeline(m.to.board());
        if dst_is_latest {
            out.timelines.get_mut(&m.to.l).unwrap().push_board(dst_succ);
        } else {
            let new_l = out.new_timeline_index(m.mover);
            out.timelines.insert(
                new_l,
                Timeline {
                    t_start: m.to.t + 1,
                    boards: std::sync::Arc::new(vec![dst_succ]),
                    creator: Some(m.mover),
                },
            );
        }
    }

    out
}

fn update_castling_rights(cr: &mut crate::board::CastlingRights, fx: i8, fy: i8, p: Piece) {
    match (p.kind, p.color) {
        (PieceKind::King, Color::White) => {
            cr.set(CastleSide::WhiteKing, false);
            cr.set(CastleSide::WhiteQueen, false);
        }
        (PieceKind::King, Color::Black) => {
            cr.set(CastleSide::BlackKing, false);
            cr.set(CastleSide::BlackQueen, false);
        }
        (PieceKind::Rook, Color::White) if fy == 0 && fx == 0 => {
            cr.set(CastleSide::WhiteQueen, false);
        }
        (PieceKind::Rook, Color::White) if fy == 0 && fx == 7 => {
            cr.set(CastleSide::WhiteKing, false);
        }
        (PieceKind::Rook, Color::Black) if fy == 7 && fx == 0 => {
            cr.set(CastleSide::BlackQueen, false);
        }
        (PieceKind::Rook, Color::Black) if fy == 7 && fx == 7 => {
            cr.set(CastleSide::BlackKing, false);
        }
        _ => {}
    }
}

fn update_castling_rights_on_capture(cr: &mut crate::board::CastlingRights, cx: i8, cy: i8) {
    // If we capture a rook on a starting-corner, revoke the matching side's rights.
    match (cx, cy) {
        (0, 0) => cr.set(CastleSide::WhiteQueen, false),
        (7, 0) => cr.set(CastleSide::WhiteKing, false),
        (0, 7) => cr.set(CastleSide::BlackQueen, false),
        (7, 7) => cr.set(CastleSide::BlackKing, false),
        _ => {}
    }
}

/// Apply a move and flip the global turn (legacy single-move API).
pub fn apply_move(mv: &Multiverse, m: Move) -> Multiverse {
    let mut out = apply_move_partial(mv, m);
    out.global_to_move = out.global_to_move.flip();
    out
}

/// Apply an entire turn (sequence of moves for a single player), then flip the global turn.
/// Search-only wrapper: updates halfmove / fullmove counters but does NOT push into
/// `position_history` (that's an O(timelines*64) Zobrist cost we want to avoid on
/// every child node).  For real game-play call `apply_turn_with_history`.
pub fn apply_turn(mv: &Multiverse, plan: &TurnPlan) -> Multiverse {
    let mut cur = mv.clone();
    let mut any_progress = false;
    for m in plan {
        let is_pawn = cur
            .board(m.from.board())
            .and_then(|b| b.get(m.from.x, m.from.y))
            .map(|p| p.kind == crate::types::PieceKind::Pawn)
            .unwrap_or(false);
        if m.capture || is_pawn {
            any_progress = true;
        }
        cur = apply_move_partial(&cur, *m);
    }
    let prev_side = cur.global_to_move;
    cur.global_to_move = cur.global_to_move.flip();
    if any_progress {
        cur.halfmove_clock = 0;
    } else {
        cur.halfmove_clock = cur.halfmove_clock.saturating_add(1);
    }
    if prev_side == Color::Black {
        cur.fullmove_number = cur.fullmove_number.saturating_add(1);
    }
    cur
}

/// Game-play wrapper: same as `apply_turn` plus Zobrist-into-history push so that
/// threefold repetition can be detected across real turns.
pub fn apply_turn_with_history(mv: &Multiverse, plan: &TurnPlan) -> Multiverse {
    let mut cur = apply_turn(mv, plan);
    let h = crate::zobrist::zobrist(&cur);
    cur.position_history.push(h);
    cur
}

// --------------------------------------------------------------------------
// Check detection and turn plan generation
// --------------------------------------------------------------------------

/// Is `color`'s king currently in check on any active board?
/// Uses direct attack tests (rays + knight/pawn patterns) — much faster than
/// generating all enemy pseudo-moves.
pub fn in_check(mv: &Multiverse, color: Color) -> bool {
    for (&l, tl) in &mv.timelines {
        if !mv.is_active(l) {
            continue;
        }
        let t = tl.latest_t();
        let Some(b) = tl.get_board(t) else { continue };
        let Some((x, y)) = b.find_king(color) else { continue };
        if is_attacked(mv, Coord::new(l, t, x, y), color.flip()) {
            return true;
        }
    }
    false
}

/// Direct attack test: is `target` attacked by any piece of color `by` anywhere
/// in the multiverse?
fn is_attacked(mv: &Multiverse, target: Coord, by: Color) -> bool {
    // Knight attacks: any knight at target ± knight-delta (4-axis generalized)?
    for &d in knight_dirs_cached() {
        let c = step_vec(target, d);
        if let Some(p) = piece_at(mv, c) {
            if p.color == by
                && mv.is_active(c.l)
                && matches!(p.kind, PieceKind::Knight | PieceKind::Princess)
            {
                return true;
            }
        }
    }
    // Pawn attacks: a pawn of color `by` at target - d attacks via +d.
    // Pawn attacks along (x, y) and (t, y) diagonals.
    // Brawn extends this to every 2-axis diagonal containing the forward axis (y or t).
    let fwd = by.sign();
    let pawn_deltas = [
        [-1, fwd, 0, 0],
        [1, fwd, 0, 0],
        [0, fwd, 1, 0],
        [0, fwd, -1, 0],
        [0, fwd, 0, 1],
        [0, fwd, 0, -1],
    ];
    for d in pawn_deltas {
        let atk = Coord {
            x: target.x - d[0] as i8,
            y: target.y - d[1] as i8,
            t: target.t - d[2],
            l: target.l - d[3],
        };
        if let Some(p) = piece_at(mv, atk) {
            if p.color == by && mv.is_active(atk.l)
                && matches!(p.kind, PieceKind::Pawn | PieceKind::Brawn)
            {
                return true;
            }
        }
    }
    // Brawn additional attacks: the (t, l) and (y, l) diagonals are covered above via (0,fwd,0,±1) (y-l) and
    // (0,0,fwd,±1)? Actually only diagonals that include the forward axis (y or t) are pawn-forward
    // diagonals. Brawn uses both y-forward AND t-forward capture diagonals:
    //   (fwd,y) × {x,t,l}, and (fwd,t) × {x,y,l}.
    // The above already covers (fwd-y) × {x,t,l}. Add the (fwd-t) × {x,y,l} set for Brawn only:
    let brawn_t_deltas = [
        [1, 0, fwd, 0],
        [-1, 0, fwd, 0],
        [0, 1, fwd, 0],
        [0, -1, fwd, 0],
        [0, 0, fwd, 1],
        [0, 0, fwd, -1],
    ];
    for d in brawn_t_deltas {
        let atk = Coord {
            x: target.x - d[0] as i8,
            y: target.y - d[1] as i8,
            t: target.t - d[2],
            l: target.l - d[3],
        };
        if let Some(p) = piece_at(mv, atk) {
            if p.color == by && p.kind == PieceKind::Brawn && mv.is_active(atk.l) {
                return true;
            }
        }
    }
    // King attacks: any enemy king at target ± 1-step in any axes?
    for &d in dirs_cached(1, 4) {
        let c = step_vec(target, d);
        if let Some(p) = piece_at(mv, c) {
            if p.color == by && p.kind == PieceKind::King && mv.is_active(c.l) {
                return true;
            }
        }
    }
    // Generalized slider attacks, per k-axis direction set:
    //   k=1: Rook, Queen
    //   k=2: Bishop, Queen, Princess
    //   k=3: Unicorn, Queen
    //   k=4: Dragon, Queen
    for k in 1u32..=4 {
        let threats: &[PieceKind] = match k {
            1 => &[PieceKind::Rook, PieceKind::Queen],
            2 => &[PieceKind::Bishop, PieceKind::Queen, PieceKind::Princess],
            3 => &[PieceKind::Unicorn, PieceKind::Queen],
            _ => &[PieceKind::Dragon, PieceKind::Queen],
        };
        for &d in dirs_cached(k, k) {
            let mut cur = target;
            loop {
                cur = step_vec(cur, d);
                if !reachable(mv, cur) {
                    break;
                }
                match piece_at(mv, cur) {
                    None => continue,
                    Some(p) => {
                        if p.color == by && mv.is_active(cur.l) && threats.contains(&p.kind) {
                            return true;
                        }
                        break;
                    }
                }
            }
        }
    }
    false
}

/// Boards where the player `color` MUST make a move this turn.
/// Rule: active timelines whose latest board has `to_move == color` AND
/// whose latest T equals the present (min latest T over active timelines).
pub fn required_source_boards(mv: &Multiverse, color: Color) -> Vec<BoardId> {
    let p = mv.present_t();
    let mut out: Vec<BoardId> = mv
        .timelines
        .iter()
        .filter_map(|(&l, tl)| {
            if !mv.is_active(l) {
                return None;
            }
            let t = tl.latest_t();
            if t != p {
                return None;
            }
            let b = tl.get_board(t)?;
            if b.to_move == color {
                Some(BoardId { l, t })
            } else {
                None
            }
        })
        .collect();
    out.sort_by_key(|b| (b.l, b.t));
    out
}

/// Hard cap on the number of turn plans we enumerate for a single position.
/// 5D positions with many required source boards can produce a cartesian
/// explosion; truncating at this count prevents pathological search hangs.
pub const MAX_PLANS_PER_POSITION: usize = 2048;

/// Generate all fully-legal turn plans for the side to move.
/// A legal plan: (a) makes at least one move from each required source board,
/// (b) leaves no own king in check at turn end, (c) respects piece-movement rules.
///
/// Branching control: we enumerate moves from the FIRST required board, recurse,
/// and require that subsequent required boards still exist after each move. To
/// bound branching we cap plan length at `MAX_TURN_LENGTH` and cap total plans
/// at `MAX_PLANS_PER_POSITION`.
pub fn generate_legal_turn_plans(mv: &Multiverse) -> Vec<TurnPlan> {
    const MAX_TURN_LENGTH: usize = 6;
    let color = mv.global_to_move;
    let mut out: Vec<TurnPlan> = Vec::new();
    let mut plan: TurnPlan = Vec::new();
    let required_start = required_source_boards(mv, color);
    dfs_plans(mv, color, &required_start, 0, &mut plan, &mut out, MAX_TURN_LENGTH);
    out
}

fn dfs_plans(
    mv: &Multiverse,
    color: Color,
    required_original: &[BoardId],
    idx: usize,
    plan: &mut TurnPlan,
    out: &mut Vec<TurnPlan>,
    budget: usize,
) {
    if out.len() >= MAX_PLANS_PER_POSITION {
        return;
    }
    if idx >= required_original.len() {
        if !in_check(mv, color) {
            out.push(plan.clone());
        }
        return;
    }
    if plan.len() >= budget {
        return;
    }
    let target = required_original[idx];
    let moves = generate_pseudo_moves_from_board(mv, target, color);
    if moves.is_empty() {
        return;
    }
    for m in moves {
        if out.len() >= MAX_PLANS_PER_POSITION {
            return;
        }
        let next = apply_move_partial(mv, m);
        plan.push(m);
        dfs_plans(&next, color, required_original, idx + 1, plan, out, budget);
        plan.pop();
    }
}

/// Generate pseudo-legal moves originating from a single board, for `color`.
fn generate_pseudo_moves_from_board(mv: &Multiverse, bid: BoardId, color: Color) -> Vec<Move> {
    let Some(board) = mv.board(bid) else { return Vec::new() };
    if board.to_move != color {
        return Vec::new();
    }
    let mut moves = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            let Some(piece) = board.get(x, y) else { continue };
            if piece.color != color {
                continue;
            }
            let src = Coord::new(bid.l, bid.t, x, y);
            match piece.kind {
                PieceKind::Pawn => gen_pawn(mv, board, &mut moves, src, piece),
                PieceKind::Brawn => gen_brawn(mv, board, &mut moves, src, piece),
                PieceKind::Knight => gen_knight(mv, &mut moves, src, piece),
                PieceKind::Bishop => gen_slider(mv, &mut moves, src, piece, 2, 2),
                PieceKind::Rook => gen_slider(mv, &mut moves, src, piece, 1, 1),
                PieceKind::Queen => gen_slider(mv, &mut moves, src, piece, 1, 4),
                PieceKind::Unicorn => gen_slider(mv, &mut moves, src, piece, 3, 3),
                PieceKind::Dragon => gen_slider(mv, &mut moves, src, piece, 4, 4),
                PieceKind::Princess => {
                    gen_slider(mv, &mut moves, src, piece, 2, 2);
                    gen_knight(mv, &mut moves, src, piece);
                }
                PieceKind::King => {
                    gen_king(mv, &mut moves, src, piece);
                    gen_castling(mv, board, &mut moves, src, piece);
                }
            }
        }
    }
    moves
}

/// Generate pseudo-legal capture moves originating from a single board. Used by
/// quiescence search, which needs to enumerate noisy moves cheaply without
/// walking the full turn-plan DFS.
pub fn generate_capture_moves_from_board(
    mv: &Multiverse,
    bid: BoardId,
    color: Color,
) -> Vec<Move> {
    generate_pseudo_moves_from_board(mv, bid, color)
        .into_iter()
        .filter(|m| m.capture || m.promotion.is_some())
        .collect()
}

/// Convenience: generate all LEGAL single-move plans (for simple tests / CLI).
pub fn generate_legal_moves(mv: &Multiverse, color: Color) -> Vec<Move> {
    // Legal "move" in isolation = a move that's part of some legal turn plan.
    // For testing convenience, return the first move of each turn plan.
    let mut set: std::collections::HashSet<Move> = std::collections::HashSet::new();
    for plan in generate_legal_turn_plans(mv) {
        if let Some(first) = plan.first() {
            set.insert(*first);
        }
    }
    let _ = color;
    set.into_iter().collect()
}
