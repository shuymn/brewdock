use crate::{
    error::{FormulaError, UnsupportedReason},
    select_bottle,
    types::Formula,
};

/// Checks if a formula can be installed via brewdock.
///
/// Verifies that the formula:
/// - Is not disabled
/// - Has a compatible bottle or source fallback metadata
/// - Has no `pour_bottle_only_if` restriction
///
/// # Errors
///
/// Returns [`FormulaError::Unsupported`] with the specific reason if any
/// check fails.
pub fn check_supportability(formula: &Formula, host_tag: &str) -> Result<(), FormulaError> {
    if formula.disabled {
        return Err(unsupported(&formula.name, UnsupportedReason::Disabled));
    }

    if formula.pour_bottle_only_if.is_some() {
        return Err(unsupported(
            &formula.name,
            UnsupportedReason::PourBottleRestricted,
        ));
    }

    if formula.versions.bottle && select_bottle(formula, host_tag).is_some() {
        return Ok(());
    }

    if formula.urls.stable.is_none() {
        let reason = if formula.versions.bottle {
            UnsupportedReason::NoBottleForTag(host_tag.to_owned())
        } else {
            UnsupportedReason::NoBottle
        };
        return Err(unsupported(&formula.name, reason));
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
    use crate::{error::UnsupportedReason, test_support::test_formula};

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
    fn test_supportability_no_bottle_or_source() {
        let mut formula = test_formula("nobottle", &[]);
        formula.versions.bottle = false;
        formula.urls.stable = None;
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
    fn test_supportability_post_install_remains_plannable() -> Result<(), FormulaError> {
        let mut formula = test_formula("postinst", &[]);
        formula.post_install_defined = true;
        check_supportability(&formula, TAG)?;
        Ok(())
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
        let mut formula = test_formula("noarch", &[]);
        formula.urls.stable = None;
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
        assert!(check_supportability(&formula, TAG).is_ok());
    }

    #[test]
    fn test_supportability_compatible_bottle_is_supported() -> Result<(), FormulaError> {
        let mut formula = test_formula("compat", &[]);
        let stable = formula
            .bottle
            .stable
            .as_mut()
            .ok_or_else(|| FormulaError::NotFound {
                name: "compat".to_owned(),
            })?;
        let bottle = stable
            .files
            .remove(TAG)
            .ok_or_else(|| FormulaError::NotFound {
                name: TAG.to_owned(),
            })?;
        stable.files.insert("arm64_sonoma".to_owned(), bottle);

        check_supportability(&formula, TAG)?;
        Ok(())
    }
}
