//! An item's earlier codes (FTR-056). A rename keeps the old code in the item's `aliases`, so a
//! reference made under it — a commit trailer, a link, a dependency, a record — still reaches the
//! item, and no other item is ever given it. These are the lookups every reader shares.

use crate::error::{CoreError, Result};
use crate::models::FeatureItem;
use crate::store::Project;
use std::collections::BTreeSet;

/// The item `code` names, current code or alias. A current code wins over an alias, should a
/// broken board have both (doctor reports that).
pub fn find<'a>(project: &'a Project, code: &str) -> Option<&'a FeatureItem> {
    project
        .features
        .iter()
        .find(|f| f.code == code)
        .or_else(|| {
            project
                .features
                .iter()
                .find(|f| f.aliases.iter().any(|a| a == code))
        })
}

/// The current code `code` names, if any item answers to it.
pub fn current<'a>(project: &'a Project, code: &str) -> Option<&'a str> {
    find(project, code).map(|f| f.code.as_str())
}

/// Every code taken in the project: current codes and aliases.
pub fn taken(project: &Project) -> BTreeSet<String> {
    project
        .features
        .iter()
        .flat_map(|f| f.codes().map(str::to_string))
        .collect()
}

/// Refuse a code a new item or a rename would take when any item answers to it.
/// `except` is the item being renamed, so renaming back to its own alias is allowed.
pub fn check_free(project: &Project, code: &str, except: Option<&str>) -> Result<()> {
    if let Some(f) = project.features.iter().find(|f| f.code == code) {
        return Err(CoreError::CodeTaken(format!("{code} is {}'s code", f.code)));
    }
    if let Some(f) = project
        .features
        .iter()
        .find(|f| f.aliases.iter().any(|a| a == code))
        && except != Some(f.code.as_str())
    {
        return Err(CoreError::CodeTaken(format!(
            "{code} was an earlier code of {}, and a code is never given to another item",
            f.code
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    fn item(code: &str, aliases: &[&str]) -> FeatureItem {
        let yaml = format!(
            "code: {code}\ntitle: {code}\nstatus: Backlog\nmilestone: MS-001\n\
             created_at: '2026-11-17T00:00:00Z'\nupdated_at: '2026-11-17T00:00:00Z'\n"
        );
        let mut f: FeatureItem = crate::yaml::from_str(&yaml).expect("item");
        f.aliases = aliases.iter().map(|a| a.to_string()).collect();
        f
    }

    fn project(features: Vec<FeatureItem>) -> Project {
        Project {
            id: "demo".into(),
            config: ProjectConfig::default_for("demo"),
            features,
            milestones: Vec::new(),
        }
    }

    #[test]
    fn an_alias_resolves_to_the_item_that_carries_it() {
        let p = project(vec![
            item("DEF-001", &["FEAT-001", "BUG-001"]),
            item("FTR-002", &[]),
        ]);
        for code in ["DEF-001", "FEAT-001", "BUG-001"] {
            assert_eq!(find(&p, code).map(|f| f.code.as_str()), Some("DEF-001"));
            assert_eq!(current(&p, code), Some("DEF-001"));
            assert_eq!(p.feature(code).expect(code).code, "DEF-001");
        }
        assert_eq!(current(&p, "FTR-002"), Some("FTR-002"));
        assert!(find(&p, "FEAT-009").is_none());
        assert!(current(&p, "FEAT-009").is_none());
        assert!(matches!(
            p.feature("FEAT-009"),
            Err(CoreError::FeatureNotFound(c)) if c == "FEAT-009"
        ));

        let f = p.feature("FEAT-001").unwrap();
        assert!(f.answers_to("DEF-001") && f.answers_to("BUG-001"));
        assert!(!f.answers_to("FTR-002"));
        assert_eq!(
            f.codes().collect::<Vec<_>>(),
            ["DEF-001", "FEAT-001", "BUG-001"]
        );

        assert_eq!(
            taken(&p).into_iter().collect::<Vec<_>>(),
            ["BUG-001", "DEF-001", "FEAT-001", "FTR-002"]
        );
        assert!(check_free(&p, "FEAT-003", None).is_ok());
        let is_code = check_free(&p, "FTR-002", None).unwrap_err().to_string();
        assert!(is_code.contains("FTR-002 is FTR-002's code"), "{is_code}");
        assert!(!is_code.contains("already exists"), "{is_code}");
        let was = check_free(&p, "FEAT-001", None).unwrap_err().to_string();
        assert!(was.contains("was an earlier code of DEF-001"), "{was}");
        assert!(check_free(&p, "FEAT-001", Some("DEF-001")).is_ok());
        assert!(check_free(&p, "FEAT-001", Some("FTR-002")).is_err());
    }
}
