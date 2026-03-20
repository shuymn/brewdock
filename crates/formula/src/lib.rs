#![warn(clippy::pedantic, clippy::nursery)]

//! Formula types, API client, and dependency resolution for brewdock.

mod api;
pub mod cellar_type;
pub mod error;
mod resolve;
mod supportability;
mod types;

pub use api::{FormulaCache, FormulaRepository, HttpFormulaRepository};
pub use cellar_type::CellarType;
pub use error::{FormulaError, UnsupportedReason};
pub use resolve::resolve_install_order;
pub use supportability::check_supportability;
pub use types::{BottleFile, BottleSpec, BottleStable, Formula, Versions};

#[cfg(test)]
mod test_support {
    use std::collections::HashMap;

    use crate::{BottleFile, BottleSpec, BottleStable, CellarType, Formula, Versions};

    pub fn test_formula(name: &str, deps: &[&str]) -> Formula {
        Formula {
            name: name.to_owned(),
            full_name: name.to_owned(),
            versions: Versions {
                stable: "1.0.0".to_owned(),
                head: None,
                bottle: true,
            },
            revision: 0,
            bottle: BottleSpec {
                stable: Some(BottleStable {
                    rebuild: 0,
                    root_url: "https://example.com".to_owned(),
                    files: HashMap::from([(
                        "arm64_sequoia".to_owned(),
                        BottleFile {
                            cellar: CellarType::Any,
                            url: "https://example.com/bottle.tar.gz".to_owned(),
                            sha256:
                                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                                    .to_owned(),
                        },
                    )]),
                }),
            },
            pour_bottle_only_if: None,
            keg_only: false,
            dependencies: deps
                .iter()
                .map(|dependency| (*dependency).to_owned())
                .collect(),
            disabled: false,
            post_install_defined: false,
        }
    }
}
