//! Negamax alpha-beta search over TURN PLANS with a transposition table.
//!
//! Enhancements:
//!   * Iterative deepening with soft deadline (checked every 2048 nodes).
//!   * Zobrist-keyed TT with Exact/LowerBound/UpperBound flags.
//!   * Principal Variation Search (PVS) with null-window re-searches.
//!   * Late Move Reduction (LMR) for quiet non-PV moves at depth ≥ 3.
//!   * Killer-move + history heuristics keyed on the turn-plan signature.
//!   * Quiescence on capture-only plans.
//!   * Mate distance encoding (so shorter mates are preferred).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::eval::{evaluate, piece_value};
use crate::movegen::{
    apply_turn, generate_capture_moves_from_board, generate_legal_turn_plans, in_check,
    required_source_boards,
};
use crate::moves::{Move, TurnPlan};
use crate::multiverse::Multiverse;
use crate::types::Color;
use crate::zobrist::zobrist;

pub const MATE: i32 = 1_000_000;

pub struct SearchResult {
    pub best: Option<TurnPlan>,
    pub score: i32,
    pub nodes: u64,
    pub depth: u32,
    pub tt_hits: u64,
    pub elapsed_ms: u128,
}

#[derive(Copy, Clone)]
enum TtFlag {
    Exact,
    LowerBound,
    UpperBound,
}

struct TtEntry {
    depth: u32,
    score: i32,
    flag: TtFlag,
    best_idx: Option<usize>,
}

const MAX_PLY: usize = 64;

/// Adjust a mate score for storage in the TT. Mate scores depend on ply-from-root,
/// which varies by how we reach a transposed position. We normalize to "mate in N
/// plies from this node" on store, and re-materialize ply-from-root on retrieve.
fn score_to_tt(score: i32, ply: usize) -> i32 {
    if score >= MATE - MAX_PLY as i32 {
        score + ply as i32
    } else if score <= -MATE + MAX_PLY as i32 {
        score - ply as i32
    } else {
        score
    }
}

fn score_from_tt(score: i32, ply: usize) -> i32 {
    if score >= MATE - MAX_PLY as i32 {
        score - ply as i32
    } else if score <= -MATE + MAX_PLY as i32 {
        score + ply as i32
    } else {
        score
    }
}

struct Ctx {
    deadline: Instant,
    nodes: u64,
    tt_hits: u64,
    tt: HashMap<u64, TtEntry>,
    aborted: bool,
    /// Killer plan signatures per ply-from-root (2 slots).
    killers: [[u64; 2]; MAX_PLY],
    /// History heuristic: plan-signature -> score.
    history: HashMap<u64, i32>,
}

impl Ctx {
    fn check_deadline(&mut self) -> bool {
        // Cheap poll: every ~256 nodes check the clock.
        if self.aborted {
            return true;
        }
        if self.nodes & 255 == 0 && Instant::now() >= self.deadline {
            self.aborted = true;
        }
        self.aborted
    }
}

fn mv_hash(mv: &Multiverse) -> u64 {
    zobrist(mv)
}

pub fn search(mv: &Multiverse, max_depth: u32) -> SearchResult {
    search_with_timeout(mv, max_depth, u128::MAX)
}

pub fn search_with_timeout(mv: &Multiverse, max_depth: u32, max_ms: u128) -> SearchResult {
    let start = Instant::now();
    let deadline = if max_ms == u128::MAX {
        Instant::now() + Duration::from_secs(3600 * 24 * 365)
    } else {
        start + Duration::from_millis(max_ms as u64)
    };
    let mut ctx = Ctx {
        deadline,
        nodes: 0,
        tt_hits: 0,
        tt: HashMap::new(),
        aborted: false,
        killers: [[0; 2]; MAX_PLY],
        history: HashMap::new(),
    };
    let mut best_result = SearchResult {
        best: None,
        score: 0,
        nodes: 0,
        depth: 0,
        tt_hits: 0,
        elapsed_ms: 0,
    };
    let mut prev_score: Option<i32> = None;
    for depth in 1..=max_depth {
        // Aspiration windows: start with a narrow window around the previous
        // iteration's score. On fail-low/high, widen and re-search. First-iter
        // or mate-adjacent scores use full window.
        let mut delta = 35;
        let (mut alpha, mut beta) = match prev_score {
            Some(s) if s.abs() < MATE / 2 => (s - delta, s + delta),
            _ => (-MATE - 1, MATE + 1),
        };
        let mut score;
        let mut best;
        let mut completed;
        loop {
            let res = negamax_root_window(mv, depth, &mut ctx, alpha, beta);
            score = res.0;
            best = res.1;
            completed = res.2;
            if !completed || ctx.aborted {
                break;
            }
            if score <= alpha {
                alpha = (score - delta).max(-MATE - 1);
                delta = delta.saturating_mul(2).min(1000);
                continue;
            }
            if score >= beta {
                beta = (score + delta).min(MATE + 1);
                delta = delta.saturating_mul(2).min(1000);
                continue;
            }
            break;
        }
        if completed {
            prev_score = Some(score);
            best_result = SearchResult {
                best,
                score,
                nodes: ctx.nodes,
                depth,
                tt_hits: ctx.tt_hits,
                elapsed_ms: start.elapsed().as_millis(),
            };
        }
        if ctx.aborted {
            break;
        }
        if Instant::now() >= ctx.deadline {
            break;
        }
    }
    best_result.elapsed_ms = start.elapsed().as_millis();
    best_result.nodes = ctx.nodes;
    best_result.tt_hits = ctx.tt_hits;
    best_result
}

fn move_sig(m: &Move) -> u64 {
    let mut s: u64 = 0x9E3779B97F4A7C15;
    s ^= (m.from.l as i64 as u64).wrapping_mul(0xBF58476D1CE4E5B9);
    s ^= (m.from.t as i64 as u64).wrapping_mul(0x94D049BB133111EB);
    s ^= ((m.from.x as u64) << 8) | (m.from.y as u64);
    s ^= (m.to.l as i64 as u64).wrapping_mul(0xD6E8FEB86659FD93);
    s ^= (m.to.t as i64 as u64).wrapping_mul(0xA5CB9243E6E0B9F1);
    s ^= ((m.to.x as u64) << 24) | ((m.to.y as u64) << 16);
    s ^= (m.kind as u64) << 40;
    if let Some(k) = m.promotion {
        s ^= (k as u64) << 48;
    }
    s = s.wrapping_mul(0x9E3779B97F4A7C15);
    s ^ (s >> 32)
}

fn plan_sig(plan: &TurnPlan) -> u64 {
    let mut s: u64 = plan.len() as u64;
    for m in plan {
        s = s.wrapping_mul(0x9E3779B97F4A7C15) ^ move_sig(m);
    }
    s
}

fn is_quiet(plan: &TurnPlan) -> bool {
    plan.iter()
        .all(|m| !m.capture && m.promotion.is_none())
}

fn mvv_lva(mv: &Multiverse, plan: &TurnPlan) -> i32 {
    let mut gain = 0i32;
    for m in plan {
        if m.capture {
            if let Some(p) = mv.board(m.to.board()).and_then(|b| b.get(m.to.x, m.to.y)) {
                gain += piece_value(p.kind);
            }
            if let Some(src) = mv.board(m.from.board()).and_then(|b| b.get(m.from.x, m.from.y)) {
                gain -= piece_value(src.kind) / 10;
            }
        }
        if let Some(k) = m.promotion {
            gain += piece_value(k) - 100;
        }
    }
    gain
}

fn score_plan(
    mv: &Multiverse,
    plan: &TurnPlan,
    ply: usize,
    ctx: &Ctx,
    pv_sig: Option<u64>,
) -> i32 {
    let sig = plan_sig(plan);
    if Some(sig) == pv_sig {
        return 10_000_000;
    }
    let cap = mvv_lva(mv, plan);
    if cap > 0 {
        return 1_000_000 + cap;
    }
    if ply < MAX_PLY {
        if ctx.killers[ply][0] == sig {
            return 900_000;
        }
        if ctx.killers[ply][1] == sig {
            return 800_000;
        }
    }
    ctx.history.get(&sig).copied().unwrap_or(0)
}

fn order_plans_scored(
    mv: &Multiverse,
    plans: &mut Vec<TurnPlan>,
    ply: usize,
    ctx: &Ctx,
    pv_sig: Option<u64>,
) {
    plans.sort_by_cached_key(|p| -score_plan(mv, p, ply, ctx, pv_sig));
}

fn record_killer(ctx: &mut Ctx, ply: usize, sig: u64) {
    if ply >= MAX_PLY {
        return;
    }
    if ctx.killers[ply][0] != sig {
        ctx.killers[ply][1] = ctx.killers[ply][0];
        ctx.killers[ply][0] = sig;
    }
}

fn bump_history(ctx: &mut Ctx, sig: u64, depth: u32) {
    let bonus = (depth as i32) * (depth as i32);
    let e = ctx.history.entry(sig).or_insert(0);
    *e = e.saturating_add(bonus).min(700_000);
}

fn negamax_root_window(
    mv: &Multiverse,
    depth: u32,
    ctx: &mut Ctx,
    alpha_init: i32,
    beta: i32,
) -> (i32, Option<TurnPlan>, bool) {
    if Instant::now() >= ctx.deadline {
        ctx.aborted = true;
        return (0, None, false);
    }
    let mut plans = generate_legal_turn_plans(mv);
    if plans.is_empty() {
        let score = if in_check(mv, mv.global_to_move) {
            -MATE
        } else {
            0
        };
        return (score, None, true);
    }
    let key = mv_hash(mv);
    let pv_sig = ctx
        .tt
        .get(&key)
        .and_then(|e| e.best_idx)
        .and_then(|i| plans.get(i))
        .map(plan_sig);
    order_plans_scored(mv, &mut plans, 0, ctx, pv_sig);

    let mut alpha = alpha_init;
    let mut best_plan = plans[0].clone();
    let mut best_score = -MATE - 1;
    let mut best_idx = 0usize;
    for (i, plan) in plans.iter().enumerate() {
        if Instant::now() >= ctx.deadline {
            ctx.aborted = true;
            return (best_score, Some(best_plan), false);
        }
        let child = apply_turn(mv, plan);
        let score = if i == 0 {
            -negamax(&child, depth - 1, -beta, -alpha, ctx, 1)
        } else {
            let s = -negamax(&child, depth - 1, -alpha - 1, -alpha, ctx, 1);
            if s > alpha && s < beta && !ctx.aborted {
                -negamax(&child, depth - 1, -beta, -alpha, ctx, 1)
            } else {
                s
            }
        };
        if ctx.aborted {
            return (best_score, Some(best_plan), false);
        }
        if score > best_score {
            best_score = score;
            best_plan = plan.clone();
            best_idx = i;
        }
        if score > alpha {
            alpha = score;
        }
        if alpha >= beta {
            break;
        }
    }
    let flag = if best_score <= alpha_init {
        TtFlag::UpperBound
    } else if best_score >= beta {
        TtFlag::LowerBound
    } else {
        TtFlag::Exact
    };
    ctx.tt.insert(
        key,
        TtEntry {
            depth,
            score: best_score,
            flag,
            best_idx: Some(best_idx),
        },
    );
    (best_score, Some(best_plan), true)
}

fn negamax(
    mv: &Multiverse,
    depth: u32,
    mut alpha: i32,
    mut beta: i32,
    ctx: &mut Ctx,
    ply: usize,
) -> i32 {
    ctx.nodes += 1;
    if ctx.check_deadline() {
        return sign(mv.global_to_move) * evaluate(mv);
    }
    if mv.is_50move_draw() || mv.is_threefold() || mv.is_insufficient_material() {
        return 0;
    }

    let key = mv_hash(mv);
    let mut tt_best: Option<usize> = None;
    if let Some(entry) = ctx.tt.get(&key) {
        if entry.depth >= depth {
            ctx.tt_hits += 1;
            let s = score_from_tt(entry.score, ply);
            match entry.flag {
                TtFlag::Exact => return s,
                TtFlag::LowerBound => alpha = alpha.max(s),
                TtFlag::UpperBound => beta = beta.min(s),
            }
            if alpha >= beta {
                return s;
            }
        }
        tt_best = entry.best_idx;
    }

    let in_chk = in_check(mv, mv.global_to_move);
    // Check extension: when the side to move is in check, extend by 1 ply so
    // forced continuations don't get truncated at the horizon. This must fire
    // at depth==0 too — otherwise checkmate at the horizon is missed (quiescence
    // stand-pats instead of detecting mate).
    let depth = if in_chk && ply < MAX_PLY - 1 { depth + 1 } else { depth };

    if depth == 0 {
        return quiescence(mv, alpha, beta, ctx, 0);
    }

    let mut plans = generate_legal_turn_plans(mv);
    if plans.is_empty() {
        return if in_chk {
            -MATE + ply as i32
        } else {
            0
        };
    }
    let pv_sig = tt_best.and_then(|i| plans.get(i)).map(plan_sig);
    order_plans_scored(mv, &mut plans, ply, ctx, pv_sig);
    let orig_alpha = alpha;
    let mut best = -MATE - 1;
    let mut best_idx = 0usize;
    let mut best_sig = 0u64;
    for (i, plan) in plans.iter().enumerate() {
        let child = apply_turn(mv, plan);
        let quiet = is_quiet(plan);
        let gives_check = in_check(&child, child.global_to_move);
        // Late Move Reduction: shave a ply off quiet late moves when not in check / not a check.
        let reduce = if depth >= 3 && i >= 3 && quiet && !in_chk && !gives_check {
            1
        } else {
            0
        };
        let nd = depth - 1 - reduce;
        let score = if i == 0 {
            -negamax(&child, depth - 1, -beta, -alpha, ctx, ply + 1)
        } else {
            let mut s = -negamax(&child, nd, -alpha - 1, -alpha, ctx, ply + 1);
            if s > alpha && reduce > 0 && !ctx.aborted {
                s = -negamax(&child, depth - 1, -alpha - 1, -alpha, ctx, ply + 1);
            }
            if s > alpha && s < beta && !ctx.aborted {
                s = -negamax(&child, depth - 1, -beta, -alpha, ctx, ply + 1);
            }
            s
        };
        if ctx.aborted {
            return best.max(alpha);
        }
        if score > best {
            best = score;
            best_idx = i;
            best_sig = plan_sig(plan);
        }
        if score > alpha {
            alpha = score;
        }
        if alpha >= beta {
            if quiet {
                record_killer(ctx, ply, plan_sig(plan));
                bump_history(ctx, plan_sig(plan), depth);
            }
            break;
        }
    }

    let flag = if best <= orig_alpha {
        TtFlag::UpperBound
    } else if best >= beta {
        TtFlag::LowerBound
    } else {
        TtFlag::Exact
    };
    ctx.tt.insert(
        key,
        TtEntry {
            depth,
            score: score_to_tt(best, ply),
            flag,
            best_idx: Some(best_idx),
        },
    );
    let _ = best_sig;
    best
}

const QS_MAX_PLY: u32 = 4;
/// Quiescence explores at most this many top-MVV-LVA captures per ply.
const QS_MOVE_LIMIT: usize = 8;

/// Quiescence search: follows capture sequences until the position is quiet.
///
/// In 5D chess a "turn" may be a cartesian product of moves across required
/// source boards, and `generate_legal_turn_plans` is exponential in that count.
/// Using it inside quiescence caused search leaves to explode by 3+ orders of
/// magnitude. Quiescence instead enumerates single-board capture moves and
/// treats each capture as a one-move pseudo-turn — only safe when exactly one
/// source board is required (otherwise we'd skip legality). For multi-required
/// positions quiescence simply stands pat, which is the conservative choice.
fn quiescence(mv: &Multiverse, mut alpha: i32, beta: i32, ctx: &mut Ctx, qply: u32) -> i32 {
    ctx.nodes += 1;
    if ctx.check_deadline() {
        return sign(mv.global_to_move) * evaluate(mv);
    }
    let stand = sign(mv.global_to_move) * evaluate(mv);
    if qply >= QS_MAX_PLY {
        return stand;
    }
    if stand >= beta {
        return beta;
    }
    if stand > alpha {
        alpha = stand;
    }

    let req = required_source_boards(mv, mv.global_to_move);
    if req.len() != 1 {
        // Multi-board required turns — plan space is too large for quiescence.
        return alpha;
    }
    let bid = req[0];
    let mut caps = generate_capture_moves_from_board(mv, bid, mv.global_to_move);
    if caps.is_empty() {
        return alpha;
    }
    // MVV-LVA ordering on single-move captures.
    caps.sort_by_cached_key(|m| {
        let victim = mv.board(m.to.board()).and_then(|b| b.get(m.to.x, m.to.y));
        let attacker = mv.board(m.from.board()).and_then(|b| b.get(m.from.x, m.from.y));
        let v = victim.map(|p| piece_value(p.kind)).unwrap_or(0);
        let a = attacker.map(|p| piece_value(p.kind)).unwrap_or(0);
        -(v * 10 - a)
    });
    caps.truncate(QS_MOVE_LIMIT);

    for m in caps {
        let plan: TurnPlan = vec![m];
        let child = apply_turn(mv, &plan);
        // Legality: our king must not remain in check after the capture.
        if in_check(&child, mv.global_to_move) {
            continue;
        }
        let score = -quiescence(&child, -beta, -alpha, ctx, qply + 1);
        if ctx.aborted {
            return alpha;
        }
        if score >= beta {
            return beta;
        }
        if score > alpha {
            alpha = score;
        }
    }
    alpha
}

fn sign(c: Color) -> i32 {
    match c {
        Color::White => 1,
        Color::Black => -1,
    }
}
