//! 5D Chess with Multiverse Time Travel — rule engine and search-based engine ("GM").
//!
//! Coordinate system: (L, T, x, y)
//!   L: timeline index. Original = 0; white-created = +1, +2, …; black-created = -1, -2, …
//!   T: half-move time index on a given timeline. T increases by 1 per successor board.
//!   (x, y): file/rank on the 8×8 physical board (x = file 0..=7, y = rank 0..=7).
//!
//! Each board is a snapshot with `to_move` recorded explicitly (branches may start on
//! either color). A move creates successor board(s). If the destination is the LATEST
//! board on its timeline, it progresses that timeline. Otherwise a new timeline branches.
//!
//! Simplification vs. the real game:
//!   * Each player's "turn" is exactly ONE move (on one source board). The real game
//!     requires moving on ALL present-aligned boards of your color. We model the
//!     reduced game but preserve multiverse, branching, and cross-board moves.
//!   * No en-passant, no 50-move rule, no threefold repetition.
//!   * Castling omitted (rare in multi-board play).
//!   * Promotion: auto-promote pawns to queens on last rank.

pub mod types;
pub mod board;
pub mod multiverse;
pub mod moves;
pub mod movegen;
pub mod eval;
pub mod search;
pub mod notation;
pub mod zobrist;
