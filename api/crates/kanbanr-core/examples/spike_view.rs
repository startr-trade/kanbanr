//! SPK-001's proof binary, built and run but never shipped: print a written doc and the `docs`
//! view at one or more positions, then the proof table (each position read twice).
//!
//! `spike_view <repo root> --doc <path> [--at <branch or commit>]... [--live] [--board <dir>]
//!  [--max-nodes N] [--root R]`. With no `--at` and no `--live` the live checkout is read. Writes
//! nothing into any repository. Exits 1 when the proof is not ok, 2 on a failure or bad usage.

use kanbanr_core::position::blob_id;
use kanbanr_core::views::{answer_bytes, docs_view, facts_version, proof, state_at};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(ok) => {
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("spike_view: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, String> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or("usage: spike_view <repo root> --doc <path> ...")?,
    );
    let (mut doc, mut board, mut docs_root, mut max_nodes) = (None, None, "docs".to_string(), 60);
    let mut ats: Vec<Option<String>> = Vec::new();
    while let Some(a) = args.next() {
        let mut value = |what: &str| args.next().ok_or(format!("{what} needs a value"));
        match a.as_str() {
            "--doc" => doc = Some(value("--doc")?),
            "--at" => ats.push(Some(value("--at")?)),
            "--live" => ats.push(None),
            "--board" => board = Some(PathBuf::from(value("--board")?)),
            "--root" => docs_root = value("--root")?,
            "--max-nodes" => {
                max_nodes = value("--max-nodes")?
                    .parse()
                    .map_err(|_| "--max-nodes needs a number".to_string())?
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let doc = doc.ok_or("--doc <path> is required")?;
    if ats.is_empty() {
        ats.push(None);
    }
    let board_commit = match &board {
        Some(b) => kanbanr_core::coderepo::rev_parse(b, "HEAD").map_err(|e| e.to_string())?,
        None => "none".to_string(),
    };
    let facts = facts_version(&board_commit, "");
    let repo = "spike";
    for at in &ats {
        let at = at.as_deref();
        let st = state_at(&root, repo, at).map_err(|e| e.to_string())?;
        println!("== at {} ==", at.unwrap_or("working tree"));
        println!("position: {}", st.position.text());
        match kanbanr_core::position::read_at(&root, &st, &doc).map_err(|e| e.to_string())? {
            Some(bytes) => {
                println!("doc {doc} (blob {}):", blob_id(&bytes));
                println!("{}", String::from_utf8_lossy(&bytes).trim_end());
            }
            None => println!("doc {doc}: absent at this position"),
        }
        let view =
            docs_view(&root, &st, &facts, &docs_root, max_nodes).map_err(|e| e.to_string())?;
        println!("view mermaid:\n{}", view.mermaid.trim_end());
        println!(
            "view bytes: {}",
            String::from_utf8_lossy(&answer_bytes(&view))
        );
        println!("view id: {}", view.view_id.as_deref().unwrap_or(""));
        println!();
    }
    let refs: Vec<Option<&str>> = ats.iter().map(|a| a.as_deref()).collect();
    let table = proof(&root, repo, &facts, &doc, &refs, &docs_root, max_nodes)
        .map_err(|e| e.to_string())?;
    println!("== proof ==");
    print!("{}", table.render());
    Ok(table.ok)
}
