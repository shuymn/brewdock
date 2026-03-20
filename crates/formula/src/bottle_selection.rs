use crate::{CellarType, Formula};

const APPLE_SILICON_CODENAMES: &[&str] = &[
    "tahoe", "sequoia", "sonoma", "ventura", "monterey", "big_sur",
];

/// A bottle selected for installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedBottle {
    /// Selected platform tag.
    pub tag: String,
    /// Bottle download URL.
    pub url: String,
    /// Bottle SHA-256.
    pub sha256: String,
    /// Cellar relocation mode.
    pub cellar: CellarType,
}

/// Selects the best available bottle for the requested host tag.
#[must_use]
pub fn select_bottle(formula: &Formula, host_tag: &str) -> Option<SelectedBottle> {
    let stable = formula.bottle.stable.as_ref()?;
    bottle_candidates(host_tag).find_map(|candidate| {
        stable.files.get(&candidate).map(|file| SelectedBottle {
            tag: candidate,
            url: file.url.clone(),
            sha256: file.sha256.clone(),
            cellar: file.cellar.clone(),
        })
    })
}

fn bottle_candidates(host_tag: &str) -> impl Iterator<Item = String> {
    let compatible =
        apple_silicon_candidates(host_tag).unwrap_or_else(|| default_bottle_candidates(host_tag));
    compatible.into_iter()
}

fn apple_silicon_candidates(host_tag: &str) -> Option<Vec<String>> {
    let codename = host_tag.strip_prefix("arm64_")?;
    let index = APPLE_SILICON_CODENAMES
        .iter()
        .position(|candidate| *candidate == codename)?;
    let mut candidates = APPLE_SILICON_CODENAMES[index..]
        .iter()
        .map(|candidate| format!("arm64_{candidate}"))
        .collect::<Vec<_>>();
    candidates.push("all".to_owned());
    Some(candidates)
}

fn default_bottle_candidates(host_tag: &str) -> Vec<String> {
    if host_tag == "all" {
        vec!["all".to_owned()]
    } else {
        vec![host_tag.to_owned(), "all".to_owned()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_formula;

    #[test]
    fn test_select_bottle_prefers_exact_host_tag() -> Result<(), Box<dyn std::error::Error>> {
        let formula = test_formula("jq", &[]);
        let selected =
            select_bottle(&formula, "arm64_sequoia").ok_or("expected exact-match bottle")?;
        assert_eq!(selected.tag, "arm64_sequoia");
        Ok(())
    }

    #[test]
    fn test_select_bottle_falls_back_to_older_compatible_tag()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut formula = test_formula("jq", &[]);
        let stable = formula
            .bottle
            .stable
            .as_mut()
            .ok_or("expected stable bottle metadata")?;
        let sequoia = stable
            .files
            .remove("arm64_sequoia")
            .ok_or("expected sequoia bottle")?;
        stable.files.insert("arm64_sonoma".to_owned(), sequoia);

        let selected = select_bottle(&formula, "arm64_sequoia")
            .ok_or("expected compatible fallback bottle")?;
        assert_eq!(selected.tag, "arm64_sonoma");
        Ok(())
    }

    #[test]
    fn test_select_bottle_falls_back_to_all_tag() -> Result<(), Box<dyn std::error::Error>> {
        let mut formula = test_formula("jq", &[]);
        let stable = formula
            .bottle
            .stable
            .as_mut()
            .ok_or("expected stable bottle metadata")?;
        let bottle = stable
            .files
            .remove("arm64_sequoia")
            .ok_or("expected sequoia bottle")?;
        stable.files.insert("all".to_owned(), bottle);

        let selected =
            select_bottle(&formula, "arm64_sequoia").ok_or("expected all-tag fallback bottle")?;
        assert_eq!(selected.tag, "all");
        Ok(())
    }

    #[test]
    fn test_select_bottle_supports_future_arm64_tahoe_tag() -> Result<(), Box<dyn std::error::Error>>
    {
        let formula = test_formula("jq", &[]);
        let selected =
            select_bottle(&formula, "arm64_tahoe").ok_or("expected tahoe fallback bottle")?;
        assert_eq!(selected.tag, "arm64_sequoia");
        Ok(())
    }
}
