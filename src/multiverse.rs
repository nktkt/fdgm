use std::collections::BTreeMap;
use std::sync::Arc;

use crate::board::Board;
use crate::types::{BoardId, Color};

/// A single timeline: a sequence of board snapshots at times `t_start..t_start+boards.len()`.
///
/// `boards` is `Arc<Vec<Board>>` for structural sharing: cloning a Multiverse is
/// the engine's hottest operation (one per pseudo-move during DFS) and we don't
/// want to memcpy every historical board on each clone. Mutation goes through
/// `Arc::make_mut` which copy-on-writes only the modified timeline.
#[derive(Clone, Debug)]
pub struct Timeline {
    pub t_start: i32,
    pub boards: Arc<Vec<Board>>,
    /// Color that CREATED the timeline (for sign convention). None for the root (L=0).
    pub creator: Option<Color>,
}

impl Timeline {
    pub fn latest_t(&self) -> i32 {
        self.t_start + self.boards.len() as i32 - 1
    }
    pub fn get_board(&self, t: i32) -> Option<&Board> {
        let i = (t - self.t_start) as isize;
        if i < 0 || i as usize >= self.boards.len() {
            None
        } else {
            Some(&self.boards[i as usize])
        }
    }
    pub fn get_board_mut(&mut self, t: i32) -> Option<&mut Board> {
        let i = (t - self.t_start) as isize;
        if i < 0 || i as usize >= self.boards.len() {
            None
        } else {
            Some(&mut Arc::make_mut(&mut self.boards)[i as usize])
        }
    }
    pub fn push_board(&mut self, b: Board) {
        Arc::make_mut(&mut self.boards).push(b);
    }
}

/// Full multiverse state.
#[derive(Clone, Debug)]
pub struct Multiverse {
    pub timelines: BTreeMap<i32, Timeline>,
    pub next_white_l: i32, // next positive L to assign (starts at 1)
    pub next_black_l: i32, // next negative L to assign (starts at -1)
    pub global_to_move: Color,
    /// Half-move counter since last capture or pawn move (any active board).
    /// Used for the 50-move rule.
    pub halfmove_clock: u32,
    /// Full-move counter (increments after Black's turn).
    pub fullmove_number: u32,
    /// History of position hashes (for three-fold repetition).
    pub position_history: Vec<u64>,
}

impl Multiverse {
    /// Create multiverse from standard starting position on timeline 0.
    pub fn standard_start() -> Self {
        let mut timelines = BTreeMap::new();
        timelines.insert(
            0,
            Timeline {
                t_start: 0,
                boards: Arc::new(vec![Board::starting_position()]),
                creator: None,
            },
        );
        Self {
            timelines,
            next_white_l: 1,
            next_black_l: -1,
            global_to_move: Color::White,
            halfmove_clock: 0,
            fullmove_number: 1,
            position_history: Vec::new(),
        }
    }

    pub fn board(&self, id: BoardId) -> Option<&Board> {
        self.timelines.get(&id.l).and_then(|tl| tl.get_board(id.t))
    }

    pub fn is_latest_on_timeline(&self, id: BoardId) -> bool {
        match self.timelines.get(&id.l) {
            Some(tl) => tl.latest_t() == id.t,
            None => false,
        }
    }

    /// Number of white-created and black-created timelines that currently exist.
    pub fn timeline_counts(&self) -> (i32, i32) {
        let mut w = 0;
        let mut b = 0;
        for (&l, _) in &self.timelines {
            if l > 0 {
                w += 1;
            } else if l < 0 {
                b += 1;
            }
        }
        (w, b)
    }

    /// A timeline L is ACTIVE iff |L| <= min(white_count, black_count) + 1.
    /// The +1 slack is why creating a new timeline on your side can deactivate the
    /// opponent's far-out timeline.
    pub fn is_active(&self, l: i32) -> bool {
        let (w, b) = self.timeline_counts();
        let limit = w.min(b) + 1;
        l.abs() <= limit
    }

    /// The "present" T: the minimum latest-T among all active timelines whose
    /// latest board has the global side-to-move's color to move. This is the
    /// frontier the current player must advance.
    pub fn present_t(&self) -> i32 {
        let side = self.global_to_move;
        let it = self
            .timelines
            .iter()
            .filter(|(l, _)| self.is_active(**l))
            .filter_map(|(_, tl)| {
                let t = tl.latest_t();
                let b = tl.get_board(t)?;
                if b.to_move == side { Some(t) } else { None }
            });
        let mut best: Option<i32> = None;
        for t in it {
            best = Some(best.map_or(t, |b| b.min(t)));
        }
        // If no active board has side-to-move, fall back to min over all active
        // timelines so downstream code still has a defined present.
        best.unwrap_or_else(|| {
            self.timelines
                .iter()
                .filter(|(l, _)| self.is_active(**l))
                .map(|(_, tl)| tl.latest_t())
                .min()
                .unwrap_or(0)
        })
    }

    /// Allocate a fresh timeline index for `color`.
    pub fn new_timeline_index(&mut self, color: Color) -> i32 {
        match color {
            Color::White => {
                let l = self.next_white_l;
                self.next_white_l += 1;
                l
            }
            Color::Black => {
                let l = self.next_black_l;
                self.next_black_l -= 1;
                l
            }
        }
    }

    /// Draw by 50-move rule (100 plies without pawn move or capture).
    pub fn is_50move_draw(&self) -> bool {
        self.halfmove_clock >= 100
    }

    /// Draw by threefold repetition of the current position (Zobrist-based).
    pub fn is_threefold(&self) -> bool {
        if self.position_history.is_empty() {
            return false;
        }
        let cur = *self.position_history.last().unwrap();
        self.position_history.iter().filter(|&&h| h == cur).count() >= 3
    }

    /// Insufficient material: both sides have only K, or K vs K+minor on a single
    /// timeline. We apply the classic chess test to every active board and require
    /// that *every* board is drawn by insufficient material.
    pub fn is_insufficient_material(&self) -> bool {
        use crate::types::PieceKind;
        if self.timelines.is_empty() {
            return false;
        }
        for (&l, tl) in &self.timelines {
            if !self.is_active(l) {
                continue;
            }
            let t = tl.latest_t();
            let Some(b) = tl.get_board(t) else { continue };
            let mut white_minor = 0;
            let mut black_minor = 0;
            let mut anything_else = false;
            for y in 0..8i8 {
                for x in 0..8i8 {
                    if let Some(p) = b.get(x, y) {
                        match p.kind {
                            PieceKind::King => {}
                            PieceKind::Knight | PieceKind::Bishop => {
                                if p.color == Color::White {
                                    white_minor += 1;
                                } else {
                                    black_minor += 1;
                                }
                            }
                            _ => anything_else = true,
                        }
                    }
                }
            }
            if anything_else || white_minor > 1 || black_minor > 1 {
                return false;
            }
        }
        true
    }

    pub fn ascii(&self) -> String {
        let mut s = String::new();
        for (l, tl) in &self.timelines {
            s.push_str(&format!(
                "== Timeline L={} (t={}..{}) active={} ==\n",
                l,
                tl.t_start,
                tl.latest_t(),
                self.is_active(*l)
            ));
            let latest = tl.latest_t();
            if let Some(b) = tl.get_board(latest) {
                s.push_str(&format!("  latest T={}, to_move={:?}\n", latest, b.to_move));
                s.push_str(&b.ascii());
            }
        }
        s.push_str(&format!(
            "Global to move: {:?}    Present T = {}\n",
            self.global_to_move,
            self.present_t()
        ));
        s
    }
}
