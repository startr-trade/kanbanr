//! Write the read API's OpenAPI description to `docs/src/reference/openapi.json` (FEAT-177).
//!
//! `cargo run -p kanbanr-core --features schema --example openapi` — or with `--check`, change
//! nothing and fail when the committed copy differs from what the code describes.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let target =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../docs/src/reference/openapi.json");
    let text = kanbanr_core::openapi::document_text();
    if std::env::args().any(|a| a == "--check") {
        let committed = std::fs::read_to_string(&target).unwrap_or_default();
        if committed == text {
            println!("{} matches the code", target.display());
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "{} is out of date: run `cargo run -p kanbanr-core --features schema --example openapi`",
            target.display()
        );
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&target, &text) {
        eprintln!("cannot write {}: {e}", target.display());
        return ExitCode::FAILURE;
    }
    println!("wrote {}", target.display());
    ExitCode::SUCCESS
}
