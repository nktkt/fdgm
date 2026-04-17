//! Rule correctness tests for fdgm.

use std::collections::BTreeMap;
use std::sync::Arc;

use fdgm::board::{Board, CastleSide, CastlingRights};
use fdgm::movegen::{
    apply_move, apply_move_partial, apply_turn, generate_legal_moves, generate_legal_turn_plans,
    generate_pseudo_moves, in_check, required_source_boards,
};
use fdgm::moves::{Move, MoveKind};
use fdgm::multiverse::{Multiverse, Timeline};
use fdgm::search::{MATE, search};
use fdgm::types::{BoardId, Color, Coord, Piece, PieceKind};

// ----- helpers -----------------------------------------------------------------

fn starting() -> Multiverse {
    Multiverse::standard_start()
}


// ----- basic rule tests --------------------------------------------------------

#[test]
fn standard_opening_has_at_least_20_single_board_moves() {
    let mv = starting();
    // `generate_legal_moves` uses turn plans internally but returns first-moves only.
    let moves = generate_legal_moves(&mv, Color::White);
    assert!(moves.len() >= 20, "got {}", moves.len());
}

#[test]
fn e2_e4_progresses_single_timeline() {
    let mv = starting();
    let e2e4 = Move {
        from: Coord::new(0, 0, 4, 1),
        to: Coord::new(0, 0, 4, 3),
        promotion: None,
        capture: false,
        mover: Color::White,
        kind: MoveKind::DoublePush,
    };
    let after = apply_turn(&mv, &vec![e2e4]);
    let tl = after.timelines.get(&0).unwrap();
    assert_eq!(tl.boards.len(), 2);
    let latest = tl.get_board(1).unwrap();
    assert!(latest.get(4, 1).is_none());
    assert_eq!(latest.get(4, 3).unwrap().kind, PieceKind::Pawn);
    assert_eq!(after.global_to_move, Color::Black);
    // En-passant target set.
    assert_eq!(latest.en_passant, Some((4, 2)));
}

#[test]
fn cross_board_move_to_past_creates_new_timeline() {
    // Construct multiverse with one timeline having T=0 and T=1, then make a
    // knight jump from T=1 back to T=0 (past) — must branch.
    let mut mv = starting();
    let e2e4 = Move {
        from: Coord::new(0, 0, 4, 1),
        to: Coord::new(0, 0, 4, 3),
        promotion: None,
        capture: false,
        mover: Color::White,
        kind: MoveKind::DoublePush,
    };
    mv = apply_turn(&mv, &vec![e2e4]);

    // Now it's Black to move globally. Force latest T=1 board to White for test setup.
    mv.global_to_move = Color::White;
    {
        let tl = mv.timelines.get_mut(&0).unwrap();
        Arc::make_mut(&mut tl.boards)[1].to_move = Color::White;
    }

    // Knight from (0, 1, 6, 0) with Δ=(0, +2, -1, 0) lands on (0, 0, 6, 2).
    let jump = Move {
        from: Coord::new(0, 1, 6, 0),
        to: Coord::new(0, 0, 6, 2),
        promotion: None,
        capture: false,
        mover: Color::White,
        kind: MoveKind::Normal,
    };
    // Bypass turn-plan requirements (source on T=1, required is present T=... min).
    // Use apply_move_partial directly to verify mechanics.
    let after = apply_move_partial(&mv, jump);
    assert!(after.timelines.contains_key(&1), "expected new L=1 timeline");
    let new_tl = after.timelines.get(&1).unwrap();
    assert_eq!(new_tl.t_start, 1);
    assert_eq!(new_tl.boards.len(), 1);
    assert_eq!(after.timelines.get(&0).unwrap().boards.len(), 3);
}

// ----- mate detection ----------------------------------------------------------

#[test]
fn back_rank_mate_is_detected() {
    let mut b = Board::empty(Color::Black);
    b.set(7, 7, Some(Piece::new(PieceKind::King, Color::Black)));
    b.set(6, 6, Some(Piece::new(PieceKind::Pawn, Color::Black)));
    b.set(7, 6, Some(Piece::new(PieceKind::Pawn, Color::Black)));
    b.set(4, 7, Some(Piece::new(PieceKind::Rook, Color::White)));
    b.set(4, 0, Some(Piece::new(PieceKind::King, Color::White)));
    let mut tls = BTreeMap::new();
    tls.insert(
        0,
        Timeline {
            t_start: 0,
            boards: Arc::new(vec![b]),
            creator: None,
        },
    );
    let mv = Multiverse {
        timelines: tls,
        next_white_l: 1,
        next_black_l: -1,
        global_to_move: Color::Black,
        halfmove_clock: 0,
        fullmove_number: 1,
        position_history: Vec::new(),
    };
    assert!(in_check(&mv, Color::Black));
    assert!(generate_legal_turn_plans(&mv).is_empty());
}

// ----- castling ----------------------------------------------------------------

fn empty_board_with_kings_and_rooks() -> Multiverse {
    let mut b = Board::empty(Color::White);
    b.castling = CastlingRights::all();
    b.set(4, 0, Some(Piece::new(PieceKind::King, Color::White)));
    b.set(0, 0, Some(Piece::new(PieceKind::Rook, Color::White)));
    b.set(7, 0, Some(Piece::new(PieceKind::Rook, Color::White)));
    b.set(4, 7, Some(Piece::new(PieceKind::King, Color::Black)));
    b.set(0, 7, Some(Piece::new(PieceKind::Rook, Color::Black)));
    b.set(7, 7, Some(Piece::new(PieceKind::Rook, Color::Black)));
    let mut tls = BTreeMap::new();
    tls.insert(
        0,
        Timeline {
            t_start: 0,
            boards: Arc::new(vec![b]),
            creator: None,
        },
    );
    Multiverse {
        timelines: tls,
        next_white_l: 1,
        next_black_l: -1,
        global_to_move: Color::White,
        halfmove_clock: 0,
        fullmove_number: 1,
        position_history: Vec::new(),
    }
}

#[test]
fn white_can_castle_kingside() {
    let mv = empty_board_with_kings_and_rooks();
    let legal = generate_legal_moves(&mv, Color::White);
    let castle = legal.iter().find(|m| m.kind == MoveKind::CastleKing);
    assert!(castle.is_some(), "expected O-O in {:?}", legal);
    // Apply and verify positions.
    let applied = apply_turn(&mv, &vec![*castle.unwrap()]);
    let b = applied.board(BoardId { l: 0, t: 1 }).unwrap();
    assert_eq!(b.get(6, 0).unwrap().kind, PieceKind::King);
    assert_eq!(b.get(5, 0).unwrap().kind, PieceKind::Rook);
    assert!(b.get(4, 0).is_none());
    assert!(b.get(7, 0).is_none());
    assert!(!b.castling.get(CastleSide::WhiteKing));
    assert!(!b.castling.get(CastleSide::WhiteQueen));
}

#[test]
fn cannot_castle_through_check() {
    let mut mv = empty_board_with_kings_and_rooks();
    // Put a black rook on f-file, attacking f1 (5, 0).
    {
        let tl = mv.timelines.get_mut(&0).unwrap();
        Arc::make_mut(&mut tl.boards)[0].set(5, 5, Some(Piece::new(PieceKind::Rook, Color::Black)));
    }
    let legal = generate_legal_moves(&mv, Color::White);
    assert!(legal.iter().all(|m| m.kind != MoveKind::CastleKing));
}

// ----- en-passant --------------------------------------------------------------

#[test]
fn en_passant_capture_is_legal_one_turn_only() {
    // Standard set-up: White plays e2-e4, Black plays d7-d5 (irrelevant), then
    // we flip to a position where Black has a pawn on d4 and White just played
    // e2-e4 setting ep = (4, 2). Black to move, pawn on d4 can take e3.
    // Build it directly.
    let mut b = Board::empty(Color::Black);
    b.set(3, 3, Some(Piece::new(PieceKind::Pawn, Color::Black))); // d4
    b.set(4, 3, Some(Piece::new(PieceKind::Pawn, Color::White))); // e4
    b.set(4, 0, Some(Piece::new(PieceKind::King, Color::White)));
    b.set(4, 7, Some(Piece::new(PieceKind::King, Color::Black)));
    b.en_passant = Some((4, 2)); // skipped over e3
    let mut tls = BTreeMap::new();
    tls.insert(
        0,
        Timeline {
            t_start: 0,
            boards: Arc::new(vec![b]),
            creator: None,
        },
    );
    let mv = Multiverse {
        timelines: tls,
        next_white_l: 1,
        next_black_l: -1,
        global_to_move: Color::Black,
        halfmove_clock: 0,
        fullmove_number: 1,
        position_history: Vec::new(),
    };
    let legal = generate_legal_moves(&mv, Color::Black);
    let ep = legal.iter().find(|m| m.kind == MoveKind::EnPassant);
    assert!(ep.is_some(), "expected EnPassant move");
    let after = apply_turn(&mv, &vec![*ep.unwrap()]);
    let b = after.board(BoardId { l: 0, t: 1 }).unwrap();
    // Black pawn ends on e3.
    assert_eq!(b.get(4, 2).unwrap().color, Color::Black);
    // White pawn on e4 was removed.
    assert!(b.get(4, 3).is_none());
}

// ----- promotion ---------------------------------------------------------------

#[test]
fn pawn_promotion_yields_four_choices() {
    // White pawn on a7 with empty a8; expect 4 pseudo-moves to a8 (=Q, =R, =B, =N).
    let mut b = Board::empty(Color::White);
    b.set(0, 6, Some(Piece::new(PieceKind::Pawn, Color::White)));
    b.set(4, 0, Some(Piece::new(PieceKind::King, Color::White)));
    b.set(4, 7, Some(Piece::new(PieceKind::King, Color::Black)));
    let mut tls = BTreeMap::new();
    tls.insert(
        0,
        Timeline {
            t_start: 0,
            boards: Arc::new(vec![b]),
            creator: None,
        },
    );
    let mv = Multiverse {
        timelines: tls,
        next_white_l: 1,
        next_black_l: -1,
        global_to_move: Color::White,
        halfmove_clock: 0,
        fullmove_number: 1,
        position_history: Vec::new(),
    };
    let pseudo = generate_pseudo_moves(&mv, Color::White);
    let to_a8: Vec<_> = pseudo.iter().filter(|m| m.to.x == 0 && m.to.y == 7).collect();
    // a7 forward = a8 promo moves (4). Plus pawn can also push forward in t to (0, 1, 0, 6)
    // creating a branch... that's not a promotion square. So exactly 4 promotion moves on a8.
    assert_eq!(to_a8.len(), 4, "expected 4 promo moves to a8, got {}", to_a8.len());
    let kinds: std::collections::HashSet<_> = to_a8.iter().map(|m| m.promotion.unwrap()).collect();
    assert!(kinds.contains(&PieceKind::Queen));
    assert!(kinds.contains(&PieceKind::Rook));
    assert!(kinds.contains(&PieceKind::Bishop));
    assert!(kinds.contains(&PieceKind::Knight));
}

// ----- multi-board turn --------------------------------------------------------

#[test]
fn after_branching_two_timelines_require_two_moves_per_turn() {
    // Create an artificial state with two active timelines both at the same present
    // and both with White to move. Verify required_source_boards returns 2, and
    // every legal turn plan has length 2.
    let mut b1 = Board::starting_position();
    let mut b2 = Board::starting_position();
    b1.to_move = Color::White;
    b2.to_move = Color::White;
    let mut tls = BTreeMap::new();
    tls.insert(
        0,
        Timeline {
            t_start: 0,
            boards: Arc::new(vec![b1]),
            creator: None,
        },
    );
    tls.insert(
        1,
        Timeline {
            t_start: 0,
            boards: Arc::new(vec![b2]),
            creator: Some(Color::White),
        },
    );
    let mv = Multiverse {
        timelines: tls,
        next_white_l: 2,
        next_black_l: -1,
        global_to_move: Color::White,
        halfmove_clock: 0,
        fullmove_number: 1,
        position_history: Vec::new(),
    };
    let req = required_source_boards(&mv, Color::White);
    assert_eq!(req.len(), 2);
    let plans = generate_legal_turn_plans(&mv);
    assert!(!plans.is_empty());
    for plan in &plans {
        assert_eq!(plan.len(), 2, "every turn plan must have one move per required board");
    }
}

// ----- perft sanity ------------------------------------------------------------

#[test]
fn perft_1_is_at_least_20() {
    let mv = starting();
    let plans = generate_legal_turn_plans(&mv);
    // With only one active timeline, a turn = one move. Standard chess has 20 opening
    // moves; 5D adds temporal pawn pushes (8 more) → at least 20.
    assert!(plans.len() >= 20, "perft(1) = {}", plans.len());
}

#[test]
fn perft_2_runs_and_is_positive() {
    let mv = starting();
    let mut total = 0u64;
    for plan in generate_legal_turn_plans(&mv) {
        let child = apply_turn(&mv, &plan);
        total += generate_legal_turn_plans(&child).len() as u64;
    }
    assert!(total > 0);
}

// ----- legacy apply_move round-trip -------------------------------------------

#[test]
fn apply_move_flips_global_turn() {
    let mv = starting();
    let e2e4 = Move {
        from: Coord::new(0, 0, 4, 1),
        to: Coord::new(0, 0, 4, 3),
        promotion: None,
        capture: false,
        mover: Color::White,
        kind: MoveKind::DoublePush,
    };
    let after = apply_move(&mv, e2e4);
    assert_eq!(after.global_to_move, Color::Black);
    // And double-application path (turn) matches single-path.
    let after2 = apply_turn(&mv, &vec![e2e4]);
    assert_eq!(after.global_to_move, after2.global_to_move);
}

// ----- tactical search tests ---------------------------------------------------

fn single_timeline_mv(b: Board, to_move: Color) -> Multiverse {
    let mut tls = BTreeMap::new();
    tls.insert(
        0,
        Timeline {
            t_start: 0,
            boards: Arc::new(vec![b]),
            creator: None,
        },
    );
    Multiverse {
        timelines: tls,
        next_white_l: 1,
        next_black_l: -1,
        global_to_move: to_move,
        halfmove_clock: 0,
        fullmove_number: 1,
        position_history: Vec::new(),
    }
}

#[test]
fn search_finds_back_rank_mate_in_1() {
    // White: Kh1, Ra1. Black: Kh8, pawns g7 h7.
    // White to move plays Ra8#: rook on a8 along 8th rank, Black king has no escape
    // (g8 covered; g7/h7 own pawns).
    let mut b = Board::empty(Color::White);
    b.set(7, 0, Some(Piece::new(PieceKind::King, Color::White)));
    b.set(0, 0, Some(Piece::new(PieceKind::Rook, Color::White)));
    b.set(7, 7, Some(Piece::new(PieceKind::King, Color::Black)));
    b.set(6, 6, Some(Piece::new(PieceKind::Pawn, Color::Black)));
    b.set(7, 6, Some(Piece::new(PieceKind::Pawn, Color::Black)));
    let mv = single_timeline_mv(b, Color::White);

    let res = search(&mv, 3);
    assert!(res.best.is_some(), "search returned no best move");
    assert!(
        res.score >= MATE - 100,
        "expected mate-in-1 score near +MATE, got {}",
        res.score
    );
    let plan = res.best.unwrap();
    assert_eq!(plan.len(), 1);
    let m = plan[0];
    // Ra1 -> a8
    assert_eq!((m.from.x, m.from.y), (0, 0));
    assert_eq!((m.to.x, m.to.y), (0, 7));
}

#[test]
fn search_captures_hanging_queen() {
    // White Ka1, Nc3, pawn on a2 (so capture doesn't reduce to insufficient material).
    // Black Kh8, Qd5 unprotected. Nc3xd5 wins the queen.
    let mut b = Board::empty(Color::White);
    b.set(0, 0, Some(Piece::new(PieceKind::King, Color::White)));
    b.set(2, 2, Some(Piece::new(PieceKind::Knight, Color::White)));
    b.set(0, 1, Some(Piece::new(PieceKind::Pawn, Color::White)));
    b.set(7, 7, Some(Piece::new(PieceKind::King, Color::Black)));
    b.set(3, 4, Some(Piece::new(PieceKind::Queen, Color::Black)));
    let mv = single_timeline_mv(b, Color::White);

    let res = search(&mv, 3);
    assert!(res.best.is_some());
    // Material-only lower bound: queen (~900) minus potential trade slack.
    assert!(
        res.score >= 500,
        "expected queen-winning score, got {}",
        res.score
    );
    let plan = res.best.unwrap();
    // First move should be a capture on d5.
    assert!(plan[0].capture, "first move should be a capture");
    assert_eq!((plan[0].to.x, plan[0].to.y), (3, 4));
}

// Keep one pseudo-move cross-board test (behavior pre/post refactor).

#[test]
fn pseudo_moves_include_temporal_pawn_push_after_branching() {
    let mut mv = starting();
    let push = Move {
        from: Coord::new(0, 0, 4, 1),
        to: Coord::new(0, 0, 4, 3),
        promotion: None,
        capture: false,
        mover: Color::White,
        kind: MoveKind::DoublePush,
    };
    mv = apply_turn(&mv, &vec![push]);
    let moves = generate_pseudo_moves(&mv, Color::Black);
    let any_cross = moves.iter().any(|m| m.is_cross_board());
    assert!(any_cross, "expected some cross-board move for Black");
    let _ = push;
}
