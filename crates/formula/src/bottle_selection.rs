use crate::{CellarType, Formula};

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
        stable.files.get(candidate).map(|file| SelectedBottle {
            tag: candidate.to_owned(),
            url: file.url.clone(),
            sha256: file.sha256.clone(),
            cellar: file.cellar.clone(),
        })
    })
}

fn bottle_candidates(host_tag: &str) -> impl Iterator<Item = &str> {
    let compatible = match host_tag {
        "arm64_sequoia" => vec!["arm64_sequoia", "arm64_sonoma", "arm64_ventura", "all"],
        "arm64_sonoma" => vec!["arm64_sonoma", "arm64_ventura", "all"],
        "arm64_ventura" => vec!["arm64_ventura", "all"],
        _ => vec![host_tag, "all"],
    };
    compatible.into_iter()
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
}
