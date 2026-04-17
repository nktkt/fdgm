//! Collision-safe position hashing (Zobrist-style with on-demand splitmix keys).
//!
//! Because (L, T) is unbounded we derive keys at hash time via splitmix64 rather than
//! storing a giant table. XOR-accumulated across all active timelines' latest boards.

use crate::multiverse::Multiverse;
use crate::types::{Color, PieceKind};

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

#[inline]
fn mix(tag: u64, a: i64, b: i64, c: i64, d: i64) -> u64 {
    let seed = tag
        ^ (a as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ (b as u64).wrapping_mul(0xC6BC279692B5C323)
        ^ (c as u64).wrapping_mul(0xD6E8FEB86659FD93)
        ^ (d as u64).wrapping_mul(0xA5CB9243E6E0B9F1);
    splitmix64(seed)
}

fn piece_key(l: i32, t: i32, x: i8, y: i8, color: Color, kind: PieceKind) -> u64 {
    let tag = 0x5050_5050_5050_5050u64
        ^ ((color as u64) << 48)
        ^ ((kind as u64) << 56);
    mix(tag, l as i64, t as i64, x as i64, y as i64)
}

fn castle_key(l: i32, t: i32, side: usize) -> u64 {
    mix(0xCACACACA_CACACACAu64, l as i64, t as i64, side as i64, 0)
}

fn ep_key(l: i32, t: i32, x: i8, y: i8) -> u64 {
    mix(0xEEEE_EEEE_EEEE_EEEEu64, l as i64, t as i64, x as i64, y as i64)
}

fn tl_head_key(l: i32, t: i32, to_move: Color) -> u64 {
    mix(
        0xABCD_1234_5678_9ABCu64 ^ ((to_move as u64) << 60),
        l as i64,
        t as i64,
        0,
        0,
    )
}

fn active_key(l: i32) -> u64 {
    mix(0xA11AA11A_A11AA11Au64, l as i64, 0, 0, 0)
}

fn global_side_key(c: Color) -> u64 {
    match c {
        Color::White => 0x5A5A5A5A5A5A5A5Au64,
        Color::Black => 0xA5A5A5A5A5A5A5A5u64,
    }
}

pub fn zobrist(mv: &Multiverse) -> u64 {
    let mut h: u64 = 0;
    for (&l, tl) in &mv.timelines {
        let t = tl.latest_t();
        let Some(b) = tl.get_board(t) else { continue };
        h ^= tl_head_key(l, t, b.to_move);
        // Timeline origin (t_start) separates otherwise-identical heads spawned
        // at different times.
        h ^= mix(0x7711_0077_7711_0077u64, l as i64, tl.t_start as i64, 0, 0);
        if mv.is_active(l) {
            h ^= active_key(l);
        }
        for y in 0..8i8 {
            for x in 0..8i8 {
                if let Some(p) = b.get(x, y) {
                    h ^= piece_key(l, t, x, y, p.color, p.kind);
                }
            }
        }
        for i in 0..4 {
            if b.castling.0[i] {
                h ^= castle_key(l, t, i);
            }
        }
        if let Some((ex, ey)) = b.en_passant {
            h ^= ep_key(l, t, ex, ey);
        }
    }
    h ^= global_side_key(mv.global_to_move);
    h ^= splitmix64(mv.halfmove_clock as u64 ^ 0xF0F0F0F0F0F0F0F0);
    // Future-timeline allocators: different `next_*_l` means different branching
    // potential — if two search trees merge by content their future is different.
    h ^= mix(
        0x4E58_4E58_4E58_4E58u64,
        mv.next_white_l as i64,
        mv.next_black_l as i64,
        0,
        0,
    );
    h
}
