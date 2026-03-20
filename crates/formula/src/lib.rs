#![warn(clippy::pedantic, clippy::nursery)]

//! Formula types, API client, and dependency resolution for brewdock.

mod api;
mod bottle_selection;
pub mod cellar_type;
pub mod error;
mod resolve;
mod supportability;
mod types;

pub use api::{FormulaCache, FormulaRepository, HttpFormulaRepository};
pub use bottle_selection::{SelectedBottle, select_bottle};
pub use cellar_type::CellarType;
pub use error::{FormulaError, UnsupportedReason};
pub use resolve::resolve_install_order;
pub use supportability::check_supportability;
pub use types::{
    BottleFile, BottleSpec, BottleStable, Formula, FormulaUrls, MacOsDependency,
    MacOsDependencyDetail, NamedEntry, Requirement, StableUrl, Versions,
};

#[cfg(test)]
mod test_support {
    use std::collections::HashMap;

    use crate::{
        BottleFile, BottleSpec, BottleStable, CellarType, Formula, FormulaUrls, StableUrl, Versions,
    };

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
            ruby_source_path: Some(format!("Formula/{name}.rb")),
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
            urls: FormulaUrls {
                stable: Some(StableUrl {
                    url: format!("https://example.com/{name}-1.0.0.tar.gz"),
                    checksum: Some(
                        "feedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedface"
                            .to_owned(),
                    ),
                }),
            },
            pour_bottle_only_if: None,
            keg_only: false,
            dependencies: deps
                .iter()
                .map(|dependency| (*dependency).to_owned())
                .collect(),
            build_dependencies: Vec::new(),
            uses_from_macos: Vec::new(),
            requirements: Vec::new(),
            disabled: false,
            post_install_defined: false,
        }
    }
}
