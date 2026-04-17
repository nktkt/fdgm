//! fdgm — 5D Chess with Multiverse Time Travel engine.

use std::env;
use std::io::{self, BufRead, Write};

use fdgm::movegen::{apply_turn, apply_turn_with_history, generate_legal_turn_plans, in_check};
use fdgm::multiverse::Multiverse;
use fdgm::notation::{fmt_turn, parse_user_turn};
use fdgm::search::search_with_timeout;
use fdgm::types::Color;

fn main() {
    let mut args = env::args().skip(1);
    let sub = args.next().unwrap_or_else(|| "bestmove".to_string());
    let mut depth: u32 = 3;
    let mut plies: u32 = 20;
    let mut ms: u128 = 5000;
    let rest: Vec<String> = args.collect();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--depth" => {
                depth = rest.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(3);
                i += 2;
            }
            "--plies" => {
                plies = rest.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(20);
                i += 2;
            }
            "--ms" => {
                ms = rest.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5000);
                i += 2;
            }
            _ => i += 1,
        }
    }

    match sub.as_str() {
        "bestmove" => cmd_bestmove(depth, ms),
        "selfplay" => cmd_selfplay(depth, ms, plies),
        "play" => cmd_play(depth, ms),
        "show" => cmd_show(),
        "perft" => cmd_perft(depth),
        other => {
            eprintln!("unknown subcommand: {other}");
            eprintln!("usage: fdgm [bestmove|selfplay|play|show|perft] [--depth N] [--plies P] [--ms MS]");
            std::process::exit(2);
        }
    }
}

fn cmd_show() {
    let mv = Multiverse::standard_start();
    print!("{}", mv.ascii());
}

fn cmd_bestmove(depth: u32, ms: u128) {
    let mv = Multiverse::standard_start();
    println!("Searching from starting position at depth≤{depth}, budget={ms}ms…");
    let res = search_with_timeout(&mv, depth, ms);
    match res.best {
        Some(plan) => println!(
            "best = {}\n  score = {:+} cp   depth = {}   nodes = {}   tt_hits = {}   {}ms",
            fmt_turn(&plan),
            res.score,
            res.depth,
            res.nodes,
            res.tt_hits,
            res.elapsed_ms
        ),
        None => println!("no legal move (mate or stalemate)"),
    }
}

fn cmd_selfplay(depth: u32, ms: u128, plies: u32) {
    let mut mv = Multiverse::standard_start();
    for ply in 0..plies {
        let plans = generate_legal_turn_plans(&mv);
        if plans.is_empty() {
            if in_check(&mv, mv.global_to_move) {
                println!("-- checkmate: {:?} loses after {} plies", mv.global_to_move, ply);
            } else {
                println!("-- stalemate after {} plies", ply);
            }
            break;
        }
        let res = search_with_timeout(&mv, depth, ms);
        let Some(plan) = res.best else { break };
        println!(
            "{:>3}. {:?} : {}   (score {:+} cp, d={}, n={}, {}ms)",
            ply + 1,
            mv.global_to_move,
            fmt_turn(&plan),
            res.score,
            res.depth,
            res.nodes,
            res.elapsed_ms
        );
        mv = apply_turn_with_history(&mv, &plan);
    }
    println!();
    print!("{}", mv.ascii());
}

fn cmd_play(depth: u32, ms: u128) {
    let mut mv = Multiverse::standard_start();
    let human = Color::White;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    loop {
        let plans = generate_legal_turn_plans(&mv);
        if plans.is_empty() {
            if in_check(&mv, mv.global_to_move) {
                println!("-- checkmate: {:?} loses", mv.global_to_move);
            } else {
                println!("-- stalemate");
            }
            return;
        }
        println!();
        print!("{}", mv.ascii());
        if mv.global_to_move == human {
            println!("(multi-board turns: separate moves with ';')");
            print!("your turn: ");
            io::stdout().flush().ok();
            line.clear();
            if input.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            match parse_user_turn(&mv, &line, human) {
                Some(plan) if plans.contains(&plan) => mv = apply_turn(&mv, &plan),
                Some(_) => println!("illegal turn."),
                None => println!("could not parse."),
            }
        } else {
            let res = search_with_timeout(&mv, depth, ms);
            let Some(plan) = res.best else { return };
            println!(
                "engine plays: {}  ({:+} cp, d={}, n={}, {}ms)",
                fmt_turn(&plan),
                res.score,
                res.depth,
                res.nodes,
                res.elapsed_ms
            );
            mv = apply_turn_with_history(&mv, &plan);
        }
    }
}

fn cmd_perft(depth: u32) {
    let mv = Multiverse::standard_start();
    let total = perft(&mv, depth);
    println!("perft({}) = {} (turn plans, side-to-move = {:?})", depth, total, mv.global_to_move);
}

fn perft(mv: &Multiverse, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let plans = generate_legal_turn_plans(mv);
    if depth == 1 {
        return plans.len() as u64;
    }
    let mut total = 0u64;
    for plan in plans {
        let child = apply_turn(mv, &plan);
        total += perft(&child, depth - 1);
    }
    total
}
