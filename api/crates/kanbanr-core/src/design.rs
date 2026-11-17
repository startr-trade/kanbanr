//! A Design's shape per kind of item (FTR-006): the subsections a spec's Design must hold, and the
//! skeleton a design starts from.
//!
//! A feature's Design is written at implementation level; a defect's is Cause, Fix and its
//! reproducing test; a chore's says what changes; a docs item's names its pages; a spike's says
//! what question it answers and what comes back. The lists are data here, so the `design_sections`
//! check and `kanbanr feature design --template` read the same thing. Every skeleton ends with
//! **Views changed**: the catalog rows the change affects, mirrored into `affects_views`.

/// The Design one kind of item carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindDesign {
    /// The kind's name, as `kind:` on an item.
    pub kind: &'static str,
    /// Other kind names that share this Design (`bug` for a defect).
    pub also: &'static [&'static str],
    /// The headings the spec must hold, each with content, for the `design_sections` check.
    pub sections: &'static [&'static str],
    /// The markdown a Design starts from: its headings, each with a line saying what goes there.
    pub skeleton: &'static str,
}

const FEATURE_SKELETON: &str = "\
## Design

### What exists
<!-- what the code and docs do today that this builds on -->

### Where
<!-- the crates, modules and files it touches -->

### Schema
<!-- what changes on disk, and whether an older binary still reads it -->

### Public signatures
<!-- each new or changed signature, and what it must do -->

### Data formats and algorithms
<!-- formats, algorithms, ordering -->

### Concurrency and failure handling
<!-- what runs at once, what fails, and how each failure is reported -->

### Configuration
<!-- defaults, and the benchmark that chose them -->

### Dependencies, security, performance
<!-- new dependencies, the security boundary, the performance budget -->

### Monitor / CLI / skill / docs
<!-- what each surface shows or says -->

### Test plan
<!-- each requirement and the tests that verify it -->

### ADRs
<!-- the decisions this records or follows, as links -->

### Open questions
<!-- undecided questions go to the person before build -->

### Implementation tasks
<!-- tasks a builder follows without design decisions: files, signatures, behaviour, tests, depends on -->

### Affects docs
<!-- links to each board doc the change affects -->

### Views changed
<!-- the catalog rows (`kanbanr catalog` lists them) whose docs are under Affects docs, plus any generated row the change affects; the row ids go into `affects_views` -->
";

const DEFECT_SKELETON: &str = "\
## Design

### Cause
<!-- why it happens, traced to the code -->

### Fix
<!-- what changes, and why that removes the cause -->

### Test plan
<!-- the reproducing test, red then green -->

### Affects docs
<!-- links to each board doc the change affects -->

### Views changed
<!-- the catalog rows (`kanbanr catalog` lists them) whose docs are under Affects docs, plus any generated row the change affects; the row ids go into `affects_views` -->
";

const CHORE_SKELETON: &str = "\
## Design

### What changes
<!-- what changes, and what must stay as it is -->

### Test plan
<!-- the tests that show nothing else changed -->

### Affects docs
<!-- links to each board doc the change affects -->

### Views changed
<!-- the catalog rows (`kanbanr catalog` lists them) whose docs are under Affects docs, plus any generated row the change affects; the row ids go into `affects_views` -->
";

const DOCS_SKELETON: &str = "\
## Design

### Pages
<!-- the pages written or changed -->

### What each says
<!-- for each page, what it says and to whom -->

### Affects docs
<!-- links to each board doc the change affects -->

### Views changed
<!-- the catalog rows (`kanbanr catalog` lists them) whose docs are under Affects docs, plus any generated row the change affects; the row ids go into `affects_views` -->
";

const SPIKE_SKELETON: &str = "\
## Design

### Question
<!-- the question the spike answers -->

### Method
<!-- how it will be answered -->

### What comes back
<!-- a change to a definition, never code -->

### Views changed
<!-- the catalog rows (`kanbanr catalog` lists them) whose docs are under Affects docs, plus any generated row the change affects; the row ids go into `affects_views` -->
";

/// Each kind's Design. The first entry, `feature`, is the fallback for an item with no kind or an
/// unknown one.
pub const KINDS: &[KindDesign] = &[
    KindDesign {
        kind: "feature",
        also: &[],
        sections: &["Design", "Implementation tasks", "Affects docs"],
        skeleton: FEATURE_SKELETON,
    },
    KindDesign {
        kind: "defect",
        also: &["bug"],
        sections: &["Design", "Cause", "Fix", "Affects docs"],
        skeleton: DEFECT_SKELETON,
    },
    KindDesign {
        kind: "chore",
        also: &["refactor", "recurring"],
        sections: &["Design", "Affects docs"],
        skeleton: CHORE_SKELETON,
    },
    KindDesign {
        kind: "docs",
        also: &[],
        sections: &["Design", "Affects docs"],
        skeleton: DOCS_SKELETON,
    },
    KindDesign {
        kind: "spike",
        also: &[],
        sections: &["Design", "Question", "What comes back"],
        skeleton: SPIKE_SKELETON,
    },
];

/// The Design for `kind`, matched case-insensitively by name or alias; none or an unknown kind is
/// a feature.
pub fn for_kind(kind: Option<&str>) -> &'static KindDesign {
    let wanted = kind.map(str::trim).unwrap_or_default();
    KINDS
        .iter()
        .find(|k| {
            k.kind.eq_ignore_ascii_case(wanted)
                || k.also.iter().any(|a| a.eq_ignore_ascii_case(wanted))
        })
        .unwrap_or(&KINDS[0])
}

/// The headings a Design of `kind` must hold, each with content.
pub fn required_sections(kind: Option<&str>) -> &'static [&'static str] {
    for_kind(kind).sections
}

/// The markdown a Design of `kind` starts from.
pub fn skeleton(kind: Option<&str>) -> &'static str {
    for_kind(kind).skeleton
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The heading texts of `markdown`, in order.
    fn headings(markdown: &str) -> Vec<String> {
        markdown
            .lines()
            .map(str::trim)
            .filter_map(|line| {
                let level = line.chars().take_while(|c| *c == '#').count();
                ((1..=6).contains(&level) && line[level..].starts_with(' '))
                    .then(|| line[level..].trim().to_string())
            })
            .collect()
    }

    /// FTR-006 R-7: each kind's skeleton holds every section the kind requires, as a heading, and
    /// ends with Views changed; aliases resolve to their kind; none or unknown is a feature.
    #[test]
    fn every_kind_has_a_skeleton_whose_headings_are_its_required_sections() {
        let names: Vec<&str> = KINDS.iter().map(|k| k.kind).collect();
        assert_eq!(names, ["feature", "defect", "chore", "docs", "spike"]);
        for k in KINDS {
            let heads = headings(k.skeleton);
            for section in k.sections {
                assert!(
                    heads.iter().any(|h| h.eq_ignore_ascii_case(section)),
                    "{}: no '{section}' heading in {heads:?}",
                    k.kind
                );
            }
            assert_eq!(
                heads.last().map(String::as_str),
                Some("Views changed"),
                "{}",
                k.kind
            );
            assert_eq!(
                heads.first().map(String::as_str),
                Some("Design"),
                "{}",
                k.kind
            );
            assert_eq!(for_kind(Some(k.kind)), k);
            assert_eq!(for_kind(Some(&k.kind.to_uppercase())), k);
            for alias in k.also {
                assert_eq!(for_kind(Some(alias)).kind, k.kind, "{alias}");
            }
        }
        assert_eq!(for_kind(Some("bug")).kind, "defect");
        assert_eq!(for_kind(Some("refactor")).kind, "chore");
        assert_eq!(for_kind(Some("recurring")).kind, "chore");
        assert_eq!(for_kind(None).kind, "feature");
        assert_eq!(for_kind(Some("")).kind, "feature");
        assert_eq!(for_kind(Some("nonsense")).kind, "feature");
        assert_eq!(
            required_sections(Some("defect")),
            ["Design", "Cause", "Fix", "Affects docs"]
        );
        assert_eq!(
            required_sections(None),
            ["Design", "Implementation tasks", "Affects docs"]
        );
        assert_eq!(
            required_sections(Some("spike")),
            ["Design", "Question", "What comes back"]
        );
        assert_eq!(skeleton(Some("bug")), DEFECT_SKELETON);
        // The feature skeleton's headings, in the order the table gives.
        assert_eq!(
            headings(skeleton(None)),
            [
                "Design",
                "What exists",
                "Where",
                "Schema",
                "Public signatures",
                "Data formats and algorithms",
                "Concurrency and failure handling",
                "Configuration",
                "Dependencies, security, performance",
                "Monitor / CLI / skill / docs",
                "Test plan",
                "ADRs",
                "Open questions",
                "Implementation tasks",
                "Affects docs",
                "Views changed",
            ]
        );
    }

    /// FTR-006 R-11: the skill names the setup order (catalog, then the system baseline's init,
    /// then the acceptance waiting on the Review page), the design template, each kind's required
    /// sections, and that Claude prepares the acceptance and never accepts it.
    #[test]
    fn the_skill_names_the_setup_order_and_each_kinds_skeleton() {
        let skill = include_str!("../../../../skill/kanbanr/SKILL.md");
        let at = |needle: &str| {
            skill
                .find(needle)
                .unwrap_or_else(|| panic!("the skill never says `{needle}`"))
        };
        // The setup plan's commands, in order.
        assert!(at("kanbanr catalog init") < at("kanbanr catalog baseline init"));
        assert!(at("kanbanr catalog baseline init") < at("kanbanr hooks status"));
        for needle in [
            "review?group=system-baseline",
            "kanbanr prepare system-baseline",
            "kanbanr catalog baseline accept",
            "kanbanr feature design --template --kind",
            "affects_views",
            "`system_baseline_accepted`",
            "`design_sections`",
        ] {
            at(needle);
        }
        assert!(
            skill.contains("never accept") || skill.contains("never accepts"),
            "the skill must say Claude never accepts the system baseline"
        );
        // Each kind's required sections are named in the skill's per-kind list.
        for k in KINDS {
            let line = format!("**{}**", k.kind);
            let start = skill
                .find(&line)
                .unwrap_or_else(|| panic!("the skill names no `{line}` design"));
            let rest = &skill[start..];
            let para = rest.split('\n').next().unwrap_or("");
            for section in k.sections.iter().filter(|s| **s != "Design") {
                assert!(
                    para.contains(section),
                    "the skill's {} line misses `{section}`: {para}",
                    k.kind
                );
            }
        }
    }
}
