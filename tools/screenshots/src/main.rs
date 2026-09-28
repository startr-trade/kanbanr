//! Capture screenshots of every kanbanr web-monitor page through a Selenium Grid.
//!
//! Talks WebDriver (Selenium) to a Chromium running in Docker (see `capture.sh` /
//! `docker-compose.yml`), points it at a locally-running `kanbanr serve`, and writes one PNG per
//! page into the output directory (default `docs/images/`).
//!
//! Framing:
//!   - The **dashboard** (board) is captured at its full content height — it genuinely has a lot.
//!   - **Other pages** use a fixed portrait "15-inch laptop, vertically twisted" frame
//!     (default 900x1440) so the docs get uniform, screen-like images.
//!
//! Env overrides:
//!   SELENIUM_URL  WebDriver endpoint of the grid   (default http://localhost:4444)
//!   KANBANR_URL   base URL of the running monitor   (default http://localhost:8080)
//!   KANBANR_PROJECT  project slug                   (default kanbanr)
//!   KANBANR_FEATURE  feature code for detail pages  (default FEAT-001)
//!   KANBANR_MILESTONE milestone code for detail     (default MS-001)
//!   FRAME_W, FRAME_H  portrait frame size           (default 900 x 1440)
//!   BOARD_W       board (dashboard) width           (default 1440)
//!   OUT_DIR       output directory                  (default docs/images)

use std::time::Duration;
use thirtyfour::prelude::*;

/// How to size a page's screenshot.
#[derive(Clone, Copy)]
enum Frame {
    /// Fixed portrait frame (width, height) — uniform, screen-like; the app's `min-height: 100vh`
    /// fills any extra space, so short pages still look like a real screen.
    Fixed(u32, u32),
    /// Fit tightly to the page's real content height at the given width (for the busy dashboard).
    Fit(u32),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let grid = env("SELENIUM_URL", "http://localhost:4444");
    let base = env("KANBANR_URL", "http://localhost:8080");
    let project = env("KANBANR_PROJECT", "kanbanr");
    let feature = env("KANBANR_FEATURE", "FEAT-001");
    let milestone = env("KANBANR_MILESTONE", "MS-001");
    let out = env("OUT_DIR", "docs/src/images");
    // A portfolio only means anything across several projects, and kanbanr's own board is one — so
    // these pages are shot against the demo board `tools/demo/portfolio.sh` builds, by pointing
    // KANBANR_URL at a daemon serving it and setting PORTFOLIO_ONLY=1.
    let portfolio_only = std::env::var("PORTFOLIO_ONLY").is_ok();
    let fw: u32 = env("FRAME_W", "900").parse().unwrap_or(900);
    let fh: u32 = env("FRAME_H", "1440").parse().unwrap_or(1440);
    let board_w: u32 = env("BOARD_W", "1440").parse().unwrap_or(1440);
    std::fs::create_dir_all(&out)?;

    let p = urlencode(&project);
    let portrait = Frame::Fixed(fw, fh);

    // Non-dashboard pages: a uniform portrait laptop frame, light theme.
    let pages: Vec<(String, &str)> = vec![
        ("/".to_string(), "home.png"),
        (format!("/p/{p}/state/Completed"), "status.png"),
        (format!("/p/{p}/state/Ongoing"), "ongoing.png"),
        (format!("/p/{p}/feature/{}", urlencode(&feature)), "feature.png"),
        (format!("/p/{p}/milestones"), "milestones.png"),
        (format!("/p/{p}/milestone/{}", urlencode(&milestone)), "milestone.png"),
        (format!("/p/{p}/schedule"), "schedule.png"),
        // Added with the method (MS-006): the charter that says why the project exists, the
        // lessons beside its goals, and the queue where a definition is agreed to before work
        // starts. These are the pages the method is actually practised on, so a docs set without
        // them shows the tool as it was two milestones ago.
        (format!("/p/{p}/review"), "review.png"),
        (format!("/p/{p}/gantt"), "gantt.png"),
        (format!("/p/{p}/workflow"), "workflow.png"),
        (format!("/p/{p}/docs"), "docs.png"),
        (format!("/p/{p}/docs/folder/design"), "docs-folder.png"),
        (format!("/p/{p}/docs/file/design/overview.md"), "doc-file.png"),
        (format!("/p/{p}/docs/file/design/data-flow.md"), "doc-mermaid.png"),
    ];

    // Cross-project rollups and the cross-project board, from the demo portfolio.
    let pages: Vec<(String, &str)> = if portfolio_only {
        vec![
            ("/portfolio".to_string(), "portfolio.png"),
            ("/".to_string(), "portfolio-home.png"),
        ]
    } else {
        pages
    };

    let mut caps = DesiredCapabilities::chrome();
    caps.add_arg("--no-sandbox")?;
    caps.add_arg("--disable-dev-shm-usage")?;
    caps.add_arg("--force-device-scale-factor=1")?;
    caps.add_arg("--hide-scrollbars")?;

    let driver = WebDriver::new(&grid, caps).await?;

    for (route, file) in &pages {
        capture(&driver, &format!("{base}{route}"), &format!("{out}/{file}"), Some("light"), portrait).await?;
    }

    if portfolio_only {
        // Dark too: the lanes were unreadable in dark mode for as long as the page existed,
        // because nobody had looked at it that way (FEAT-096).
        let portfolio = format!("{base}/portfolio");
        capture(&driver, &portfolio, &format!("{out}/portfolio-dark.png"), Some("dark"), portrait).await?;
        driver.quit().await?;
        println!("done — portfolio screenshots in {out}/");
        return Ok(());
    }

    // The charter runs past a portrait frame, and what gets cropped is the lessons — the half a
    // reader most wants. Full content height, like the board.
    let charter = format!("{base}/p/{p}/charter");
    capture(&driver, &charter, &format!("{out}/charter.png"), Some("light"), Frame::Fit(fw)).await?;

    // Dashboard: full content height, light theme.
    let board = format!("{base}/p/{p}");
    capture(&driver, &board, &format!("{out}/board.png"), Some("light"), Frame::Fit(board_w)).await?;

    // Two dark-theme showcases (kept few so the docs aren't a wall of black):
    //  - the board, and
    //  - the feature page with its todo-lists (in the same portrait frame as its light version).
    capture(&driver, &board, &format!("{out}/board-dark.png"), Some("dark"), Frame::Fit(board_w)).await?;
    let feat = format!("{base}/p/{p}/feature/{}", urlencode(&feature));
    capture(&driver, &feat, &format!("{out}/feature-dark.png"), Some("dark"), portrait).await?;

    driver.quit().await?;
    println!("done — screenshots in {out}/");
    Ok(())
}

/// Navigate to `url`, apply `theme` (Some("light"|"dark") via localStorage), size per `frame`, and
/// write a PNG to `path`.
async fn capture(driver: &WebDriver, url: &str, path: &str, theme: Option<&str>, frame: Frame) -> anyhow::Result<()> {
    let width = match frame {
        Frame::Fixed(w, _) | Frame::Fit(w) => w,
    };
    // Size the window early so the page lays out at the target width before we measure/paint.
    let initial_h = match frame {
        Frame::Fixed(_, h) => h,
        Frame::Fit(_) => 1000,
    };
    let _ = driver.set_window_rect(0, 0, width, initial_h).await;

    driver.goto(url).await?;
    if let Some(t) = theme {
        // The monitor reads its theme from localStorage; set it then reload to apply before paint.
        let _ = driver
            .execute(&format!("window.localStorage.setItem('kanbanr-theme','{t}');"), Vec::new())
            .await;
        driver.refresh().await?;
    }
    // Let React render and async work settle (docs tree, first SSE tick).
    tokio::time::sleep(Duration::from_millis(1600)).await;

    // Mermaid renders asynchronously (it lazily loads diagram-type chunks, then produces SVG). Poll
    // until every ```mermaid code block has been replaced — rather than guessing a fixed delay.
    for _ in 0..50 {
        let pending = match driver
            .execute("return document.querySelectorAll('code.language-mermaid').length;", Vec::new())
            .await
        {
            Ok(r) => r.json().as_i64().unwrap_or(0),
            Err(_) => 0,
        };
        if pending == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    match frame {
        // Fixed frame: keep `min-height: 100vh` active so the app fills the frame; capture WxH.
        Frame::Fixed(w, h) => {
            let _ = driver.set_window_rect(0, 0, w, h).await;
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        // Fit frame: neutralize `min-height: 100vh` so the measured height is the REAL content
        // height (not the window height), then resize the window to it.
        Frame::Fit(w) => {
            let _ = driver
                .execute(
                    "var s=document.getElementById('shot-fit')||document.createElement('style');\
                     s.id='shot-fit';s.textContent='.app{min-height:0 !important}';\
                     document.head.appendChild(s);return true;",
                    Vec::new(),
                )
                .await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            let content_h = match driver
                .execute(
                    "return Math.max(document.body.scrollHeight, document.documentElement.scrollHeight);",
                    Vec::new(),
                )
                .await
            {
                Ok(ret) => ret.json().as_i64().unwrap_or(900),
                Err(_) => 900,
            };
            let h = content_h.clamp(240, 6000) as u32;
            let _ = driver.set_window_rect(0, 0, w, h).await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    let png = driver.screenshot_as_png().await?;
    std::fs::write(path, png)?;
    println!("  wrote {path}");
    Ok(())
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Minimal percent-encoding for the path segments we control (project slug / codes).
fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "%20".to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}
