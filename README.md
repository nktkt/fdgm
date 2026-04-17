# fdgm

A Rust rule engine and search-based grandmaster for **5D Chess with Multiverse Time Travel**.

Pieces move across four axes — file, rank, time (`T`), and timeline (`L`) — and a turn may require one move per required source board, so the branching factor explodes fast. `fdgm` implements the full ruleset and a classical alpha-beta engine tuned for that branching.

## Features

**Rule engine**
- Full 4-axis movement (x, y, T, L) for all standard pieces
- Variant pieces: **Unicorn** (3-axis slider), **Dragon** (4-axis slider), **Princess** (B+N), **Brawn** (t-capturing pawn)
- Timeline branching, activation rule (`|L| ≤ min(white, black) + 1`), present-T detection
- Castling, en passant, promotion (Q/R/B/N)
- Draw detection: 50-move rule, threefold repetition (Zobrist-based), insufficient material

**Search**
- Iterative deepening with aspiration windows
- Negamax + principal variation search (PVS)
- Transposition table with ply-adjusted mate scores
- Killer moves + history heuristic
- Late move reduction (LMR)
- Check extensions (including at horizon)
- MVV-LVA quiescence

**Performance tricks**
- Zobrist hashing with on-demand splitmix64 key derivation
- Cached direction tables via `OnceLock`
- `Arc<Vec<Board>>` copy-on-write for timeline cloning (the engine's hottest op)
- Turn-plan count cap to bound pathological cartesian explosion

## Install

```bash
cargo build --release
```

## Usage

```bash
# Search the starting position
./target/release/fdgm bestmove --depth 6 --ms 15000

# Engine vs engine
./target/release/fdgm selfplay --depth 4 --ms 2000 --plies 20

# Play against the engine (text interface)
./target/release/fdgm play --depth 4 --ms 3000

# Perft sanity
./target/release/fdgm perft --depth 2

# Print current starting-position multiverse
./target/release/fdgm show
```

### Flags
- `--depth N` — maximum search depth
- `--ms MS` — soft time budget in milliseconds
- `--plies P` — number of plies for `selfplay`

## Project layout

```
src/
  board.rs        single 8x8 board with piece arrays, castling, en passant
  multiverse.rs   BTreeMap<L, Timeline> with Arc-shared board history
  types.rs        Color, PieceKind, Coord (l, t, x, y), BoardId
  moves.rs        Move, MoveKind, TurnPlan
  movegen.rs      pseudo-/legal move generation, turn-plan DFS
  zobrist.rs      64-bit position hashing
  eval.rs         static evaluation (material + piece-square + mobility)
  search.rs       iterative deepening, negamax, quiescence, TT
  notation.rs     coord printing
  main.rs         CLI
tests/
  rules.rs        rule-correctness + tactical tests
```

## Tests

```bash
cargo test --release
```

The suite covers:
- Opening move count, turn application, global-turn flipping
- Cross-board moves and timeline branching
- Back-rank checkmate detection
- Castling (including through-check rejection)
- En passant (one-turn window)
- Promotion (all four choices)
- Multi-timeline turn plans (one move per required board)
- Perft(1) and perft(2) sanity
- Tactical search: mate-in-1 and hanging-queen capture

## Status

Early prototype. The engine plays legal 5D chess and finds simple tactics; it's not yet strong enough to outplay a serious human player on multi-timeline positions.

## License

MIT
