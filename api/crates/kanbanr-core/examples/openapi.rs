//! Write the read API's OpenAPI description to `docs/src/reference/openapi.json`, and its route
//! table into `docs/src/reference/http-api.md` (FEAT-177).
//!
//! `cargo run -p kanbanr-core --features schema --example openapi` — or with `--check`, change
//! nothing and fail when a committed copy differs from what the code describes.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let reference = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../docs/src/reference");
    let json = reference.join("openapi.json");
    let page = reference.join("http-api.md");
    let check = std::env::args().any(|a| a == "--check");

    let current_page = std::fs::read_to_string(&page).unwrap_or_default();
    let Some(new_page) = kanbanr_core::openapi::with_route_table(&current_page) else {
        eprintln!(
            "{} has no route-table markers ({} … {})",
            page.display(),
            kanbanr_core::openapi::TABLE_START,
            kanbanr_core::openapi::TABLE_END
        );
        return ExitCode::FAILURE;
    };
    let wanted = [
        (json, kanbanr_core::openapi::document_text()),
        (page, new_page),
    ];

    let mut stale = false;
    for (path, text) in &wanted {
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        if &committed == text {
            println!("{} matches the code", path.display());
            continue;
        }
        if check {
            eprintln!("{} is out of date", path.display());
            stale = true;
        } else if let Err(e) = std::fs::write(path, text) {
            eprintln!("cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        } else {
            println!("wrote {}", path.display());
        }
    }
    if stale {
        eprintln!("run `cargo run -p kanbanr-core --features schema --example openapi`");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
