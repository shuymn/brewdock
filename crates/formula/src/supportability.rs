use crate::{
    error::{FormulaError, UnsupportedReason},
    types::Formula,
};

/// Checks if a formula can be installed via brewdock.
///
/// Verifies that the formula:
/// - Is not disabled
/// - Has a pre-built bottle
/// - Does not define a `post_install` hook
/// - Has no `pour_bottle_only_if` restriction
/// - Has a bottle file for the given host tag
///
/// # Errors
///
/// Returns [`FormulaError::Unsupported`] with the specific reason if any
/// check fails.
pub fn check_supportability(formula: &Formula, host_tag: &str) -> Result<(), FormulaError> {
    if formula.disabled {
        return Err(unsupported(&formula.name, UnsupportedReason::Disabled));
    }

    if !formula.versions.bottle {
        return Err(unsupported(&formula.name, UnsupportedReason::NoBottle));
    }

    if formula.post_install_defined {
        return Err(unsupported(
            &formula.name,
            UnsupportedReason::PostInstallDefined,
        ));
    }

    if formula.pour_bottle_only_if.is_some() {
        return Err(unsupported(
            &formula.name,
            UnsupportedReason::PourBottleRestricted,
        ));
    }

    let has_bottle = formula
        .bottle
        .stable
        .as_ref()
        .is_some_and(|s| s.files.contains_key(host_tag));

    if !has_bottle {
        return Err(unsupported(
            &formula.name,
            UnsupportedReason::NoBottleForTag(host_tag.to_owned()),
        ));
    }

    Ok(())
}

fn unsupported(name: &str, reason: UnsupportedReason) -> FormulaError {
    FormulaError::Unsupported {
        name: name.to_owned(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::UnsupportedReason, types::test_formula};

    const TAG: &str = "arm64_sequoia";

    #[test]
    fn test_supportability_valid_formula() -> Result<(), FormulaError> {
        let formula = test_formula("jq", &["oniguruma"]);
        check_supportability(&formula, TAG)?;
        Ok(())
    }

    #[test]
    fn test_supportability_disabled() {
        let mut formula = test_formula("disabled", &[]);
        formula.disabled = true;
        let err = check_supportability(&formula, TAG);
        assert!(matches!(
            err,
            Err(FormulaError::Unsupported {
                reason: UnsupportedReason::Disabled,
                ..
            })
        ));
    }

    #[test]
    fn test_supportability_no_bottle() {
        let mut formula = test_formula("nobottle", &[]);
        formula.versions.bottle = false;
        let err = check_supportability(&formula, TAG);
        assert!(matches!(
            err,
            Err(FormulaError::Unsupported {
                reason: UnsupportedReason::NoBottle,
                ..
            })
        ));
    }

    #[test]
    fn test_supportability_post_install() {
        let mut formula = test_formula("postinst", &[]);
        formula.post_install_defined = true;
        let err = check_supportability(&formula, TAG);
        assert!(matches!(
            err,
            Err(FormulaError::Unsupported {
                reason: UnsupportedReason::PostInstallDefined,
                ..
            })
        ));
    }

    #[test]
    fn test_supportability_pour_bottle_only_if() {
        let mut formula = test_formula("pour", &[]);
        formula.pour_bottle_only_if = Some("some_condition".to_owned());
        let err = check_supportability(&formula, TAG);
        assert!(matches!(
            err,
            Err(FormulaError::Unsupported {
                reason: UnsupportedReason::PourBottleRestricted,
                ..
            })
        ));
    }

    #[test]
    fn test_supportability_no_bottle_for_tag() {
        let formula = test_formula("noarch", &[]);
        let err = check_supportability(&formula, "x86_64_monterey");
        assert!(matches!(
            err,
            Err(FormulaError::Unsupported {
                reason: UnsupportedReason::NoBottleForTag(_),
                ..
            })
        ));
    }

    #[test]
    fn test_supportability_empty_bottle_spec() {
        let mut formula = test_formula("empty", &[]);
        formula.bottle.stable = None;
        let err = check_supportability(&formula, TAG);
        assert!(matches!(
            err,
            Err(FormulaError::Unsupported {
                reason: UnsupportedReason::NoBottleForTag(_),
                ..
            })
        ));
    }
}
