//! Walkthroughs that run themselves (FTR-037): a guide's runs driven headlessly, decisions taken on
//! the scratch board by a scripted person, every question and board claim captured.
//!
//! The pieces, in the order a run uses them: `runbook` (the steps and the answer policy), `guard`
//! (the fence and `setup.log`), `remotes` (the run's own repositories), `driver` (the developer's
//! Claude Code session), `answer` (every question Claude asks), `person` (the scripted person on
//! the monitor), `truth` and `capture` (what was really so, and the raw record of it), `derive` (the
//! tables). The `run` command here strings them together.
// The modules land task by task (W1–W9); until the `walkthrough` command is wired in W9, what they
// export is used only by their tests.
#![allow(dead_code)]

pub mod driver;
