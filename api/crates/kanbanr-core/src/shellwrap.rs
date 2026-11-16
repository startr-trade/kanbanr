//! Reading a shell command through the wrappers in front of it (FTR-059).
//!
//! Other tools put words in front of a command: a command-rewriting hook runs `rtk git commit …`
//! where the person typed `git commit …`, and people write `sudo`, `env A=1`, `time` and the like
//! themselves. Every kanbanr reader of a command line (the git guard, capture) goes through
//! [`command_start`], so it judges the command the wrappers run, not the wrappers.
//!
//! The wrapper list is fixed and compared by whole words (decided 16 Nov 2026, B-40; R-11): a
//! wrapper the list does not know is read as the command itself. That weakens only the guard's
//! early feedback, never a rule, because the git hooks still enforce every commit rule (R-5).
//! Taking "the first word that ends in `git`" instead would also match `echo git commit` (L-23).

/// Wrappers known to rewrite the output of the command they run. `rtk proxy` is not one: it runs
/// the command with its output untouched.
pub const CONDENSING: &[&str] = &["rtk"];

/// The file name of a command word: `/usr/bin/sudo` is `sudo`.
fn basename(word: &str) -> &str {
    word.rsplit(['/', '\\']).next().unwrap_or(word)
}

/// Is `word` a `NAME=value` assignment, `NAME` matching `[A-Za-z_][A-Za-z0-9_]*`?
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The index of the first word of `words` past every leading wrapper, and the wrappers set aside,
/// in order.
///
/// The rules, applied repeatedly from the start; the first word of each wrapper is compared by its
/// basename, every other word whole:
/// - a `NAME=value` assignment, set aside with no label;
/// - `sudo`, then its flags (`-u`, `-g`, `-h`, `-p` take one argument; other `-x` words are
///   skipped), label `sudo`;
/// - `env`, then `-i`, `-`, `-u NAME`, `--` and assignments, label `env`;
/// - `command`, `exec`, `nohup`, label each;
/// - `time`, then `-p`, label `time`;
/// - `nice`, then `-n N` or `-N`, label `nice`;
/// - `rtk proxy`, label `rtk proxy`, or `rtk` followed by any other word, label `rtk`.
///
/// `rtkx`, `my-rtk` and `echo rtk git commit` hold no wrapper.
pub fn command_start<S: AsRef<str>>(words: &[S]) -> (usize, Vec<&'static str>) {
    let word = |i: usize| words.get(i).map(AsRef::as_ref);
    let mut i = 0;
    let mut labels = Vec::new();
    while let Some(w) = word(i) {
        if is_assignment(w) {
            i += 1;
            continue;
        }
        match basename(w) {
            "sudo" => {
                labels.push("sudo");
                i += 1;
                while let Some(f) = word(i) {
                    match f {
                        "-u" | "-g" | "-h" | "-p" => i += 2,
                        _ if f.starts_with('-') => i += 1,
                        _ => break,
                    }
                }
            }
            "env" => {
                labels.push("env");
                i += 1;
                while let Some(f) = word(i) {
                    match f {
                        "-i" | "-" | "--" => i += 1,
                        "-u" => i += 2,
                        _ if is_assignment(f) => i += 1,
                        _ => break,
                    }
                }
            }
            "command" => {
                labels.push("command");
                i += 1;
            }
            "exec" => {
                labels.push("exec");
                i += 1;
            }
            "nohup" => {
                labels.push("nohup");
                i += 1;
            }
            "time" => {
                labels.push("time");
                i += 1;
                if word(i) == Some("-p") {
                    i += 1;
                }
            }
            "nice" => {
                labels.push("nice");
                i += 1;
                match word(i) {
                    Some("-n") => i += 2,
                    Some(f)
                        if f.len() > 1
                            && f.starts_with('-')
                            && f[1..].chars().all(|c| c.is_ascii_digit()) =>
                    {
                        i += 1
                    }
                    _ => {}
                }
            }
            "rtk" => {
                if word(i + 1) == Some("proxy") {
                    labels.push("rtk proxy");
                    i += 2;
                } else {
                    labels.push("rtk");
                    i += 1;
                }
            }
            _ => break,
        }
    }
    (i.min(words.len()), labels)
}

/// The first wrapper of any simple command of `command` that is in [`CONDENSING`]: the wrapper
/// that may have cut the command's output short.
pub fn condensing_wrapper(command: &str) -> Option<&'static str> {
    simple_words(command).iter().find_map(|words| {
        command_start(words)
            .1
            .into_iter()
            .find(|label| CONDENSING.contains(label))
    })
}

/// The simple commands of a command line, each as its words: split at `&&`, `||`, `;`, `|` and
/// newlines outside quotes. A part with no words is left out.
pub fn simple_words(command: &str) -> Vec<Vec<String>> {
    let mut parts = vec![String::new()];
    let mut quote: Option<char> = None;
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                parts.last_mut().unwrap().push(c);
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                parts.last_mut().unwrap().push(c);
            }
            None if c == ';'
                || c == '\n'
                || c == '|'
                || (c == '&' && chars.get(i + 1) == Some(&'&')) =>
            {
                if c == '&' || (c == '|' && chars.get(i + 1) == Some(&'|')) {
                    i += 1;
                }
                parts.push(String::new());
            }
            None => parts.last_mut().unwrap().push(c),
        }
        i += 1;
    }
    parts
        .iter()
        .map(|p| shell_words(p))
        .filter(|words| !words.is_empty())
        .collect()
}

/// The words of one simple shell command, with quotes removed and quoted text kept (`cd "a b"` is
/// two words). No expansion, no escapes beyond `\"`.
pub fn shell_words(part: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut prev = '\0';
    for c in part.chars() {
        match quote {
            Some(q) if c == q && prev != '\\' => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            None => {
                current.push(c);
                in_word = true;
            }
        }
        prev = c;
    }
    if in_word {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(command: &str) -> (usize, Vec<&'static str>) {
        command_start(&shell_words(command))
    }

    /// The first word past the wrappers, or "" when none is left.
    fn first(command: &str) -> String {
        let words = shell_words(command);
        let (i, _) = command_start(&words);
        words.get(i).cloned().unwrap_or_default()
    }

    #[test]
    fn known_wrappers_are_set_aside_by_whole_word() {
        // Each wrapper form alone: (command, index, labels).
        let cases: &[(&str, usize, &[&str])] = &[
            ("git commit -m x", 0, &[]),
            ("A=1 git commit", 1, &[]),
            ("A=1 _B2=x git commit", 2, &[]),
            ("sudo git commit", 1, &["sudo"]),
            ("/usr/bin/sudo git commit", 1, &["sudo"]),
            ("sudo -u alice -g staff git commit", 5, &["sudo"]),
            ("sudo -h host -p prompt git commit", 5, &["sudo"]),
            ("sudo -E -H git commit", 3, &["sudo"]),
            ("env git commit", 1, &["env"]),
            ("env -i A=1 B=2 git commit", 4, &["env"]),
            ("env - -u HOME -- git commit", 5, &["env"]),
            ("command git commit", 1, &["command"]),
            ("exec git commit", 1, &["exec"]),
            ("nohup git commit", 1, &["nohup"]),
            ("time git commit", 1, &["time"]),
            ("time -p git commit", 2, &["time"]),
            ("nice git commit", 1, &["nice"]),
            ("nice -n 10 git commit", 3, &["nice"]),
            ("nice -5 git commit", 2, &["nice"]),
            ("rtk git commit", 1, &["rtk"]),
            ("rtk proxy git commit", 2, &["rtk proxy"]),
            ("/home/me/.cargo/bin/rtk git commit", 1, &["rtk"]),
            // Combined forms.
            (
                "sudo -u a env X=1 rtk git commit",
                6,
                &["sudo", "env", "rtk"],
            ),
            (
                "A=1 time -p rtk proxy git commit",
                5,
                &["time", "rtk proxy"],
            ),
            (
                "nohup nice -n 5 command git status",
                5,
                &["nohup", "nice", "command"],
            ),
        ];
        for (command, index, labels) in cases {
            let (i, l) = start(command);
            assert_eq!(i, *index, "index for {command:?}");
            assert_eq!(&l, labels, "wrappers for {command:?}");
        }
        for command in [
            "git commit",
            "sudo -u a env X=1 rtk git commit",
            "rtk proxy git commit",
        ] {
            assert_eq!(first(command), "git", "{command:?}");
        }

        // `rtk` alone: set aside, nothing after it.
        let (i, _) = start("rtk");
        assert_eq!(i, 1);
        assert_eq!(first("rtk"), "");

        // R-11: not wrappers, read as commands.
        for command in [
            "rtkx git commit",
            "my-rtk git commit",
            "echo rtk git commit",
        ] {
            assert_eq!(start(command), (0, vec![]), "{command:?}");
        }
        assert_eq!(first("echo rtk git commit"), "echo");
        // `-` words that are not a wrapper's flags stop the reading.
        assert_eq!(start("env -C dir git commit"), (1, vec!["env"]));
        // An assignment-like word that is not one is a command.
        assert_eq!(start("1A=x git"), (0, vec![]));
        assert_eq!(start("=x git"), (0, vec![]));
        // Empty input.
        assert_eq!(command_start::<&str>(&[]), (0, vec![]));
        // A flag that takes an argument at the end does not run past the words.
        assert_eq!(start("sudo -u"), (2, vec!["sudo"]));

        // condensing_wrapper sees `rtk`, not `rtk proxy` or the rest.
        assert_eq!(condensing_wrapper("rtk cargo test"), Some("rtk"));
        assert_eq!(condensing_wrapper("cd api && rtk cargo test"), Some("rtk"));
        assert_eq!(
            condensing_wrapper("sudo env A=1 rtk cargo test"),
            Some("rtk")
        );
        assert_eq!(condensing_wrapper("rtk proxy cargo test"), None);
        assert_eq!(condensing_wrapper("cargo test"), None);
        assert_eq!(condensing_wrapper("echo rtk cargo test"), None);
        assert_eq!(condensing_wrapper("rtkx cargo test"), None);
    }

    #[test]
    fn simple_words_split_outside_quotes_only() {
        assert_eq!(
            simple_words("cd a && git commit -m 'x; y' | cat || true\nls;"),
            vec![
                vec!["cd".to_string(), "a".into()],
                vec!["git".into(), "commit".into(), "-m".into(), "x; y".into()],
                vec!["cat".into()],
                vec!["true".into()],
                vec!["ls".into()],
            ]
        );
        assert_eq!(shell_words(r#"cd "a b" c"#), vec!["cd", "a b", "c"]);
    }
}
