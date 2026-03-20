use brewdock_core::{
    BrewdockError,
    error::{BottleError, FormulaError},
};

/// Returns a user-facing hint for the given error, if applicable.
pub fn for_error(err: &anyhow::Error) -> Option<&'static str> {
    let brewdock_err = err.downcast_ref::<BrewdockError>()?;
    match brewdock_err {
        BrewdockError::Formula(formula_err) => hint_for_formula(formula_err),
        BrewdockError::Bottle(bottle_err) => hint_for_bottle(bottle_err),
        BrewdockError::Platform(_) => Some("brewdock currently supports macOS only"),
        BrewdockError::Io(_) | BrewdockError::Cellar(_) => None,
    }
}

const fn hint_for_formula(err: &FormulaError) -> Option<&'static str> {
    match err {
        FormulaError::NotFound { .. } => Some("run `bd update` to refresh the formula index"),
        FormulaError::Unsupported { .. } => Some("this formula cannot be installed as a bottle"),
        FormulaError::Network(_) => Some("check your internet connection"),
        FormulaError::Parse(_) | FormulaError::CyclicDependency(_) => None,
    }
}

const fn hint_for_bottle(err: &BottleError) -> Option<&'static str> {
    match err {
        BottleError::ChecksumMismatch { .. } => {
            Some("run `bd update` to refresh the formula index, then retry")
        }
        BottleError::Download(_) => Some("check your internet connection"),
        BottleError::Io(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use brewdock_core::platform::PlatformError;

    use super::*;

    #[test]
    fn test_hint_for_formula_not_found() {
        let err: anyhow::Error = BrewdockError::Formula(FormulaError::NotFound {
            name: "missing".to_owned(),
        })
        .into();
        let hint = for_error(&err);
        assert_eq!(hint, Some("run `bd update` to refresh the formula index"));
    }

    #[test]
    fn test_hint_for_platform_error() {
        let err: anyhow::Error = BrewdockError::Platform(PlatformError::Unsupported).into();
        let hint = for_error(&err);
        assert_eq!(hint, Some("brewdock currently supports macOS only"));
    }

    #[test]
    fn test_hint_for_checksum_mismatch() {
        let err: anyhow::Error = BrewdockError::Bottle(BottleError::ChecksumMismatch {
            expected: "abc".to_owned(),
            actual: "def".to_owned(),
        })
        .into();
        let hint = for_error(&err);
        assert_eq!(
            hint,
            Some("run `bd update` to refresh the formula index, then retry")
        );
    }

    #[test]
    fn test_hint_for_io_error_returns_none() {
        let err: anyhow::Error =
            BrewdockError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "test")).into();
        assert!(for_error(&err).is_none());
    }

    #[test]
    fn test_hint_for_non_brewdock_error_returns_none() {
        let err = anyhow::anyhow!("some other error");
        assert!(for_error(&err).is_none());
    }
}
