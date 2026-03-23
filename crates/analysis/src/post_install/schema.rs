use std::collections::{BTreeMap, BTreeSet};

use ruby_prism::{Node, ParseResult};

use super::{
    Argument, ContentPart, LowerCtx, LoweringTier, MethodDef, PathBase, PathCondition, PathExpr,
    PathSegment, SegmentPart, Statement, append_segment, body_statements, call_args, call_name,
    node_source, parse_helper_path_expr, parse_path_expr, parse_string, unsupported, visit_calls,
};
use crate::error::AnalysisError;

pub(super) fn match_postgresql_schemas<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    if let Some(stmts) = match_basic_postgresql_schema(body, ctx, helper_stack)? {
        return Ok(Some(stmts));
    }

    if let Some(stmts) = match_postgresql_schema(body, ctx, helper_stack)? {
        return Ok(Some(stmts));
    }

    Ok(None)
}

pub(super) fn match_gdk_pixbuf_loader_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    _helper_stack: &mut BTreeSet<String>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    let source = node_source(ctx.parsed, body)?;
    if !source.contains("GDK_PIXBUF_MODULEDIR") || !source.contains("gdk-pixbuf-query-loaders") {
        return Ok(None);
    }

    let loader_dir =
        if ctx.methods.contains_key("gdk_so_ver") && ctx.methods.contains_key("gdk_module_ver") {
            let so_ver = helper_string_literal("gdk_so_ver", ctx.methods)?;
            let module_ver = helper_string_literal("gdk_module_ver", ctx.methods)?;
            format!("/lib/gdk-pixbuf-{so_ver}/{module_ver}/loaders")
        } else {
            "/lib/gdk-pixbuf-2.0/2.10.0/loaders".to_owned()
        };

    let command = if source.contains("Formula[\"gdk-pixbuf\"].opt_bin") {
        Argument::Path(PathExpr {
            base: PathBase::FormulaOptBin("gdk-pixbuf".to_owned()),
            segments: vec![PathSegment::Literal("gdk-pixbuf-query-loaders".to_owned())],
        })
    } else {
        Argument::Path(PathExpr::new(PathBase::Bin, &["gdk-pixbuf-query-loaders"]))
    };

    Ok(Some(vec![
        Statement::SetEnv {
            variable: "GDK_PIXBUF_MODULEDIR".to_owned(),
            value: vec![
                ContentPart::HomebrewPrefix,
                ContentPart::Literal(loader_dir),
            ],
        },
        Statement::System(vec![command, Argument::String("--update-cache".to_owned())]),
    ]))
}

fn helper_string_literal(
    name: &str,
    methods: &BTreeMap<String, MethodDef<'_>>,
) -> Result<String, AnalysisError> {
    let method = methods
        .get(name)
        .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
            message: format!("missing helper string literal: {name}"),
        })?;
    let Some(body) = method.body.as_ref() else {
        return unsupported(&format!("empty helper body: {name}"));
    };
    let statements = body_statements(body)?;
    if statements.len() != 1 {
        return unsupported(&format!("helper must lower to one string literal: {name}"));
    }
    parse_string(&statements[0])?.ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
        message: format!("helper must be a string literal: {name}"),
    })
}

pub(super) fn match_mysql_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    let source = node_source(ctx.parsed, body)?;
    if !source.contains("(var/\"mysql\").mkpath") || !source.contains("my.cnf") {
        return Ok(None);
    }

    let statements = body_statements(body)?;
    if !statements.iter().any(|statement| {
        statement.as_call_node().is_some_and(|call| {
            call_name(&call).is_ok_and(|name| name == "mkpath")
                && call.receiver().is_some_and(|receiver| {
                    parse_path_expr(&receiver, ctx, helper_stack)
                        .is_ok_and(|path| path == PathExpr::new(PathBase::Var, &["mysql"]))
                })
        })
    }) {
        return Ok(None);
    }

    Ok(Some(vec![Statement::Mkpath(PathExpr::new(
        PathBase::Var,
        &["mysql"],
    ))]))
}

pub(super) fn match_llvm_clang_config_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    if ctx.tier != LoweringTier::WithAttributes {
        return Ok(None);
    }

    let source = node_source(ctx.parsed, body)?;
    if !source.contains("write_config_files")
        || !source.contains("kernel_version")
        || !source.contains("Hardware::CPU")
    {
        return Ok(None);
    }

    let has_config_dir_helper = ctx.methods.contains_key("clang_config_file_dir");
    let has_write_helper = ctx.methods.contains_key("write_config_files");
    if !has_config_dir_helper || !has_write_helper {
        return Ok(None);
    }

    let config_dir = PathExpr::new(PathBase::Etc, &["clang"]);
    let clt_path = "/Library/Developer/CommandLineTools";
    let mut stmts = vec![Statement::Mkpath(config_dir.clone())];
    let systems: &[(&str, SegmentPart)] = &[
        ("darwin", SegmentPart::KernelVersionMajor),
        ("macosx", SegmentPart::MacOSVersion),
    ];
    let arches = ["arm64", "x86_64", "aarch64"];

    for (system_name, version_part) in systems {
        for arch in &arches {
            let filename_parts = vec![
                SegmentPart::Literal(format!("{arch}-apple-{system_name}")),
                version_part.clone(),
                SegmentPart::Literal(".cfg".to_owned()),
            ];
            let file_path = PathExpr {
                base: config_dir.base.clone(),
                segments: {
                    let mut segs = config_dir.segments.clone();
                    segs.push(PathSegment::Interpolated(filename_parts));
                    segs
                },
            };
            let content = vec![
                ContentPart::Literal(format!("-isysroot {clt_path}/SDKs/MacOSX")),
                ContentPart::Runtime(SegmentPart::MacOSVersion),
                ContentPart::Literal(".sdk\n".to_owned()),
            ];
            stmts.push(Statement::WriteFile {
                path: file_path,
                content,
            });
        }
    }

    Ok(Some(stmts))
}

pub(super) fn matches_bundle_bootstrap_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<bool, AnalysisError> {
    let static_ctx = LowerCtx {
        tier: LoweringTier::Static,
        ..*ctx
    };
    let mut saw_mkpath = false;
    let mut saw_atomic_write = false;
    visit_calls(body, &mut |call| {
        let name = call_name(call)?;
        if name == "mkpath" {
            if let Some(receiver) = call.receiver()
                && parse_path_expr(&receiver, &static_ctx, helper_stack)
                    .is_ok_and(|path| path.base == PathBase::Pkgetc && path.segments.is_empty())
            {
                saw_mkpath = true;
            }
        } else if name == "atomic_write"
            && let Some(receiver) = call.receiver()
            && parse_path_expr(&receiver, &static_ctx, helper_stack).is_ok_and(|path| {
                path.base == PathBase::Pkgetc
                    && path.segments == [PathSegment::Literal("cert.pem".to_owned())]
            })
        {
            saw_atomic_write = true;
        }
        Ok(())
    })?;

    Ok(saw_mkpath && saw_atomic_write)
}

pub(super) fn detect_cert_symlink_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<(PathExpr, PathExpr)>, AnalysisError> {
    let static_ctx = LowerCtx {
        tier: LoweringTier::Static,
        ..*ctx
    };
    let mut link_dir = None;
    let mut target = None;

    for statement in body_statements(body)? {
        if let Some(call) = statement.as_call_node() {
            let name = call_name(&call)?;
            if name == "install_symlink"
                && let Some(receiver) = call.receiver()
            {
                let receiver = parse_path_expr(&receiver, &static_ctx, helper_stack)?;
                let arguments = call_args(&call);
                if arguments.len() == 1 {
                    link_dir = Some(receiver);
                    target = Some(parse_path_expr(&arguments[0], &static_ctx, helper_stack)?);
                }
            }
        }
    }

    Ok(link_dir.zip(target))
}

pub(super) fn matches_ruby_bundler_cleanup_schema(
    body: &Node<'_>,
    methods: &BTreeMap<String, MethodDef<'_>>,
    formula_version: &str,
) -> Result<bool, AnalysisError> {
    if formula_version.is_empty()
        || !methods.contains_key("api_version")
        || !methods.contains_key("rubygems_bindir")
    {
        return Ok(false);
    }
    let mut has_rm = false;
    let mut has_rm_r = false;
    visit_calls(body, &mut |call| {
        match call_name(call)?.as_str() {
            "rm" => has_rm = true,
            "rm_r" => has_rm_r = true,
            _ => {}
        }
        Ok(())
    })?;
    Ok(has_rm && has_rm_r)
}

pub(super) fn normalize_ruby_bundler_cleanup(formula_version: &str) -> Vec<Statement> {
    let v = compute_ruby_api_version(formula_version);
    let gems = |sub: &[&str]| {
        let mut segs: Vec<PathSegment> = ["lib", "ruby", "gems"]
            .iter()
            .map(|&s| PathSegment::Literal(s.to_owned()))
            .collect();
        segs.push(PathSegment::Literal(v.clone()));
        segs.extend(sub.iter().map(|&s| PathSegment::Literal(s.to_owned())));
        PathExpr {
            base: PathBase::HomebrewPrefix,
            segments: segs,
        }
    };
    vec![
        Statement::RemoveIfExists(gems(&["bin", "bundle"])),
        Statement::RemoveIfExists(gems(&["bin", "bundler"])),
        Statement::GlobRemove {
            dir: gems(&["gems"]),
            pattern: "bundler-*".to_owned(),
        },
    ]
}

pub(super) fn matches_node_npm_propagation_schema(
    body: &Node<'_>,
    parsed: &ParseResult<'_>,
) -> Result<bool, AnalysisError> {
    let source = node_source(parsed, body)?;
    if !source.contains("HOMEBREW_PREFIX") || !source.contains("node_modules") {
        return Ok(false);
    }
    let mut has_cp_r = false;
    let mut has_ln_sf = false;
    visit_calls(body, &mut |call| {
        match call_name(call)?.as_str() {
            "cp_r" => has_cp_r = true,
            "ln_sf" => has_ln_sf = true,
            _ => {}
        }
        Ok(())
    })?;
    Ok(has_cp_r && has_ln_sf)
}

pub(super) fn normalize_node_npm_propagation() -> Vec<Statement> {
    let hp = |segs: &[&str]| PathExpr::new(PathBase::HomebrewPrefix, segs);
    let bp = |segs: &[&str]| PathExpr::new(PathBase::Bin, segs);
    let node_modules = hp(&["lib", "node_modules"]);
    let npm_dir = hp(&["lib", "node_modules", "npm"]);

    let mut stmts = vec![
        Statement::Mkpath(node_modules.clone()),
        Statement::IfPath {
            condition: npm_dir.clone(),
            kind: PathCondition::Exists,
            then_branch: vec![Statement::RemoveIfExists(npm_dir)],
        },
        Statement::RecursiveCopy {
            from: PathExpr::new(PathBase::Libexec, &["lib", "node_modules", "npm"]),
            to: node_modules,
        },
        Statement::ForceSymlink {
            target: hp(&["lib", "node_modules", "npm", "bin", "npm-cli.js"]),
            link: bp(&["npm"]),
        },
        Statement::ForceSymlink {
            target: hp(&["lib", "node_modules", "npm", "bin", "npx-cli.js"]),
            link: bp(&["npx"]),
        },
        Statement::ForceSymlink {
            target: bp(&["npm"]),
            link: hp(&["bin", "npm"]),
        },
        Statement::ForceSymlink {
            target: bp(&["npx"]),
            link: hp(&["bin", "npx"]),
        },
    ];

    for man_section in &["man1", "man5", "man7"] {
        stmts.push(Statement::Mkpath(hp(&["share", "man", man_section])));
        stmts.push(Statement::GlobRemove {
            dir: hp(&["share", "man", man_section]),
            pattern: "{npm.,npm-,npmrc.,package.json.,npx.}*".to_owned(),
        });
        stmts.push(Statement::GlobSymlink {
            source_dir: hp(&["lib", "node_modules", "npm", "man", man_section]),
            pattern: "{npm,package-,shrinkwrap-,npx}*".to_owned(),
            link_dir: hp(&["share", "man", man_section]),
        });
    }

    stmts.push(Statement::WriteFile {
        path: hp(&["lib", "node_modules", "npm", "npmrc"]),
        content: vec![
            ContentPart::Literal("prefix = ".to_owned()),
            ContentPart::HomebrewPrefix,
            ContentPart::Literal("\n".to_owned()),
        ],
    });

    stmts
}

pub(super) fn matches_shared_mime_info_schema(
    body: &Node<'_>,
    parsed: &ParseResult<'_>,
) -> Result<bool, AnalysisError> {
    let source = node_source(parsed, body)?;
    if !source.contains("HOMEBREW_PREFIX/\"share/mime\"")
        || !source.contains("ln_sf(global_mime, cellar_mime)")
        || !source.contains("(pkgshare/\"packages\").children")
        || !source.contains("update-mime-database")
    {
        return Ok(false);
    }

    let mut has_rm_r = false;
    let mut has_ln_sf = false;
    let mut has_cp = false;
    let mut has_system = false;
    visit_calls(body, &mut |call| {
        match call_name(call)?.as_str() {
            "rm_r" => has_rm_r = true,
            "ln_sf" => has_ln_sf = true,
            "cp" => has_cp = true,
            "system" => has_system = true,
            _ => {}
        }
        Ok(())
    })?;

    Ok(has_rm_r && has_ln_sf && has_cp && has_system)
}

pub(super) fn normalize_shared_mime_info() -> Vec<Statement> {
    let hp = |segs: &[&str]| PathExpr::new(PathBase::HomebrewPrefix, segs);
    let share = |segs: &[&str]| PathExpr::new(PathBase::Share, segs);
    let pkgshare = |segs: &[&str]| PathExpr::new(PathBase::Pkgshare, segs);
    let bin = |segs: &[&str]| PathExpr::new(PathBase::Bin, segs);

    let global_mime = hp(&["share", "mime"]);
    let cellar_mime = share(&["mime"]);

    vec![
        Statement::IfPath {
            condition: global_mime.clone(),
            kind: PathCondition::Symlink,
            then_branch: vec![Statement::RemoveIfExists(global_mime.clone())],
        },
        Statement::IfPath {
            condition: cellar_mime.clone(),
            kind: PathCondition::ExistsAndNotSymlink,
            then_branch: vec![Statement::RemoveIfExists(cellar_mime.clone())],
        },
        Statement::ForceSymlink {
            target: global_mime.clone(),
            link: cellar_mime,
        },
        Statement::Mkpath(hp(&["share", "mime", "packages"])),
        Statement::CopyChildren {
            from_dir: pkgshare(&["packages"]),
            to_dir: hp(&["share", "mime", "packages"]),
        },
        Statement::System(vec![
            Argument::Path(bin(&["update-mime-database"])),
            Argument::Path(global_mime),
        ]),
    ]
}

pub(super) fn match_python_site_packages_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    if ctx.tier != LoweringTier::WithAttributes {
        return Ok(None);
    }

    let source = node_source(ctx.parsed, body)?;
    if !source.contains("site_packages.mkpath")
        || !source.contains("site_packages_cellar.parent.install_symlink site_packages")
        || !source.contains("system python3, \"-Im\", \"ensurepip\"")
        || !source.contains("system python3, \"-Im\", \"pip\", \"install\", \"-v\"")
        || !source.contains("mv (site_packages/\"bin\").children, bin")
        || !source.contains("mv bin/\"wheel\", bin/\"wheel#{version.major_minor}\"")
    {
        return Ok(None);
    }

    let formula_source = std::str::from_utf8(ctx.parsed.source()).map_err(|error| {
        AnalysisError::UnsupportedPostInstallSyntax {
            message: format!("invalid formula source utf-8: {error}"),
        }
    })?;
    let setuptools_version = extract_resource_version(formula_source, "setuptools")?;
    let pip_version = extract_resource_version(formula_source, "pip")?;
    let wheel_version = extract_resource_version(formula_source, "wheel")?;

    Ok(Some(normalize_python_site_packages(
        &setuptools_version,
        &pip_version,
        &wheel_version,
    )))
}

pub(super) fn match_php_pear_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    if ctx.tier != LoweringTier::WithAttributes {
        return Ok(None);
    }

    let source = node_source(ctx.parsed, body)?;
    if !source.contains("pecl_path.mkpath")
        || !source.contains("Utils.safe_popen_read(bin/\"php-config\", \"--extension-dir\")")
        || !source.contains("cp_r pkgshare/\"pear/.\", pear_path")
        || !source.contains("system bin/\"pear\", \"update-channels\"")
    {
        return Ok(None);
    }

    Ok(Some(normalize_php_pear_schema()))
}

/// Detects the older postgresql-family `post_install` pattern that only
/// creates the data dir and runs `initdb`.
fn match_basic_postgresql_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    if ctx.tier != LoweringTier::WithAttributes {
        return Ok(None);
    }

    let source = node_source(ctx.parsed, body)?;
    if !source.contains("postgresql_datadir.mkpath")
        || !source.contains("initdb")
        || !source.contains("HOMEBREW_GITHUB_ACTIONS")
    {
        return Ok(None);
    }
    if source.contains("each_child") || source.contains("relative_path_from") {
        return Ok(None);
    }

    if !ctx.methods.contains_key("postgresql_datadir") {
        return Ok(None);
    }

    let datadir = parse_helper_path_expr("postgresql_datadir", ctx, helper_stack)?;

    let mut statements = vec![
        Statement::Mkpath(PathExpr::new(PathBase::Var, &["log"])),
        Statement::Mkpath(datadir.clone()),
    ];
    statements.extend(postgresql_mirror_trees(&[]));
    statements.push(postgresql_ci_guard(&datadir, true));

    Ok(Some(statements))
}

fn postgresql_initdb_statement(data_dir: PathExpr) -> Statement {
    Statement::System(vec![
        Argument::Path(PathExpr::new(PathBase::Bin, &["initdb"])),
        Argument::String("--locale=en_US.UTF-8".to_owned()),
        Argument::String("-E".to_owned()),
        Argument::String("UTF-8".to_owned()),
        Argument::Path(data_dir),
    ])
}

/// Generates `MirrorTree` statements for the standard `%w[include lib share].each` pattern.
fn postgresql_mirror_trees(source_subdirs: &[&str]) -> Vec<Statement> {
    let name_segment = PathSegment::Interpolated(vec![SegmentPart::FormulaName]);
    ["include", "lib", "share"]
        .iter()
        .map(|dir| {
            let mut source_parts: Vec<&str> = vec![dir];
            source_parts.extend_from_slice(source_subdirs);
            Statement::MirrorTree {
                source: PathExpr::new(PathBase::Prefix, &source_parts),
                dest: PathExpr {
                    base: PathBase::HomebrewPrefix,
                    segments: vec![
                        PathSegment::Literal((*dir).to_owned()),
                        name_segment.clone(),
                    ],
                },
                prune_names: vec![".DS_Store".to_owned()],
            }
        })
        .collect()
}

/// Wraps an `initdb` invocation in the standard `HOMEBREW_GITHUB_ACTIONS` guard.
///
/// When `check_pg_version` is true, initdb is further guarded by `PG_VERSION` missing.
fn postgresql_ci_guard(datadir: &PathExpr, check_pg_version: bool) -> Statement {
    let initdb = postgresql_initdb_statement(datadir.clone());
    let then_branch = if check_pg_version {
        vec![Statement::IfPath {
            condition: append_segment(datadir, "PG_VERSION"),
            kind: PathCondition::Missing,
            then_branch: vec![initdb],
        }]
    } else {
        vec![initdb]
    };
    Statement::IfEnv {
        variable: "HOMEBREW_GITHUB_ACTIONS".to_owned(),
        negate: true,
        then_branch,
    }
}

/// Detects the postgresql-family `post_install` pattern.
///
/// Pattern markers:
/// - Has a `postgresql_datadir` helper that returns `var/name`
/// - Has `%w[include lib share].each` with `find` + `relative_path_from`
/// - Has `bin.each_child` for versioned symlinks
/// - Has `ENV["HOMEBREW_GITHUB_ACTIONS"]` guard
/// - Has `system bin/"initdb"` conditional on `pg_version_exists?`
fn match_postgresql_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    if ctx.tier != LoweringTier::WithAttributes {
        return Ok(None);
    }

    let source = node_source(ctx.parsed, body)?;

    if !source.contains("each_child")
        || !source.contains(".find")
        || !source.contains("relative_path_from")
    {
        return Ok(None);
    }

    let has_datadir_helper = ctx.methods.contains_key("postgresql_datadir");
    let has_pg_version_check = ctx.methods.contains_key("pg_version_exists?");
    if !has_datadir_helper {
        return Ok(None);
    }

    let datadir = parse_helper_path_expr("postgresql_datadir", ctx, helper_stack)?;

    let mut stmts = vec![
        Statement::Mkpath(PathExpr::new(PathBase::Var, &["log"])),
        Statement::Mkpath(datadir.clone()),
    ];
    stmts.extend(postgresql_mirror_trees(&["postgresql"]));

    stmts.push(Statement::ChildrenSymlink {
        source_dir: PathExpr::new(PathBase::Bin, &[]),
        link_dir: PathExpr::new(PathBase::HomebrewPrefix, &["bin"]),
        suffix: vec![
            SegmentPart::Literal("-".to_owned()),
            SegmentPart::VersionMajor,
        ],
    });

    stmts.push(postgresql_ci_guard(&datadir, has_pg_version_check));

    Ok(Some(stmts))
}

fn compute_ruby_api_version(version: &str) -> String {
    let mut parts = version.splitn(3, '.');
    let major = parts.next().unwrap_or(version);
    parts.next().map_or_else(
        || format!("{version}.0"),
        |minor| format!("{major}.{minor}.0"),
    )
}

fn normalize_python_site_packages(
    setuptools_version: &str,
    pip_version: &str,
    wheel_version: &str,
) -> Vec<Statement> {
    let version_major_minor_segment =
        PathSegment::Interpolated(vec![SegmentPart::VersionMajorMinor]);
    let python_bin = PathExpr {
        base: PathBase::Bin,
        segments: vec![PathSegment::Interpolated(vec![
            SegmentPart::Literal("python".to_owned()),
            SegmentPart::VersionMajorMinor,
        ])],
    };
    let lib_cellar = PathExpr {
        base: PathBase::Prefix,
        segments: vec![
            PathSegment::Literal("Frameworks".to_owned()),
            PathSegment::Literal("Python.framework".to_owned()),
            PathSegment::Literal("Versions".to_owned()),
            version_major_minor_segment,
            PathSegment::Literal("lib".to_owned()),
            PathSegment::Interpolated(vec![
                SegmentPart::Literal("python".to_owned()),
                SegmentPart::VersionMajorMinor,
            ]),
        ],
    };
    let site_packages = PathExpr {
        base: PathBase::HomebrewPrefix,
        segments: vec![
            PathSegment::Literal("lib".to_owned()),
            PathSegment::Interpolated(vec![
                SegmentPart::Literal("python".to_owned()),
                SegmentPart::VersionMajorMinor,
            ]),
            PathSegment::Literal("site-packages".to_owned()),
        ],
    };
    let site_packages_cellar = append_segment(&lib_cellar, "site-packages");
    let bundled_dir = append_segment(&lib_cellar, "ensurepip/_bundled");
    let libexec_bin = PathExpr::new(PathBase::Libexec, &["bin"]);
    let site_packages_bin = append_segment(&site_packages, "bin");
    let bin_dir = PathExpr::new(PathBase::Bin, &[]);

    let mut statements = vec![
        Statement::Mkpath(site_packages.clone()),
        Statement::IfPath {
            condition: site_packages_cellar.clone(),
            kind: PathCondition::Exists,
            then_branch: vec![Statement::RemoveIfExists(site_packages_cellar)],
        },
        Statement::InstallSymlink {
            link_dir: lib_cellar,
            target: site_packages.clone(),
        },
        Statement::RemoveIfExists(append_segment(&site_packages, "sitecustomize.pyc")),
        Statement::RemoveIfExists(append_segment(&site_packages, "sitecustomize.pyo")),
        Statement::System(vec![
            Argument::Path(python_bin.clone()),
            Argument::String("-Im".to_owned()),
            Argument::String("ensurepip".to_owned()),
        ]),
        Statement::System(vec![
            Argument::Path(python_bin),
            Argument::String("-Im".to_owned()),
            Argument::String("pip".to_owned()),
            Argument::String("install".to_owned()),
            Argument::String("-v".to_owned()),
            Argument::String("--no-deps".to_owned()),
            Argument::String("--no-index".to_owned()),
            Argument::String("--upgrade".to_owned()),
            Argument::String("--isolated".to_owned()),
            Argument::String("--target".to_owned()),
            Argument::Path(site_packages.clone()),
            Argument::Path(append_segment(
                &bundled_dir,
                &format!("setuptools-{setuptools_version}-py3-none-any.whl"),
            )),
            Argument::Path(append_segment(
                &bundled_dir,
                &format!("pip-{pip_version}-py3-none-any.whl"),
            )),
            Argument::Path(PathExpr::new(
                PathBase::Libexec,
                &[&format!("wheel-{wheel_version}-py3-none-any.whl")],
            )),
        ]),
        Statement::MoveChildren {
            from_dir: site_packages_bin.clone(),
            to_dir: bin_dir.clone(),
        },
        Statement::RemoveIfExists(site_packages_bin),
        Statement::RemoveIfExists(append_segment(&bin_dir, "pip")),
        Statement::RemoveIfExists(append_segment(&bin_dir, "pip3")),
        Statement::Move {
            from: append_segment(&bin_dir, "wheel"),
            to: PathExpr {
                base: PathBase::Bin,
                segments: vec![PathSegment::Interpolated(vec![
                    SegmentPart::Literal("wheel".to_owned()),
                    SegmentPart::VersionMajorMinor,
                ])],
            },
        },
    ];

    for (short_name, long_name) in [
        ("pip", "pip"),
        ("pip3", "pip"),
        ("wheel", "wheel"),
        ("wheel3", "wheel"),
    ] {
        statements.push(Statement::InstallSymlink {
            link_dir: libexec_bin.clone(),
            target: PathExpr {
                base: PathBase::Bin,
                segments: vec![PathSegment::Interpolated(vec![
                    SegmentPart::Literal(long_name.to_owned()),
                    SegmentPart::VersionMajorMinor,
                ])],
            },
        });
        statements.push(Statement::ForceSymlink {
            target: PathExpr::new(PathBase::Libexec, &["bin", short_name]),
            link: PathExpr::new(PathBase::HomebrewPrefix, &["bin", short_name]),
        });
    }

    statements
}

fn normalize_php_pear_schema() -> Vec<Statement> {
    let hp = |segs: &[&str]| PathExpr::new(PathBase::HomebrewPrefix, segs);
    let etc_php = PathExpr {
        base: PathBase::Etc,
        segments: vec![
            PathSegment::Literal("php".to_owned()),
            PathSegment::Interpolated(vec![SegmentPart::VersionMajorMinor]),
        ],
    };
    let pecl_path = hp(&["lib", "php", "pecl"]);
    let php_basename = SegmentPart::CapturedOutputBasename("extension_dir".to_owned());
    let pecl_ext_dir = PathExpr {
        base: PathBase::HomebrewPrefix,
        segments: vec![
            PathSegment::Literal("lib".to_owned()),
            PathSegment::Literal("php".to_owned()),
            PathSegment::Literal("pecl".to_owned()),
            PathSegment::Interpolated(vec![php_basename]),
        ],
    };
    let pear_path = hp(&["share", "pear"]);
    let opt_php_bin = hp(&["opt", "php", "bin"]);

    let mut statements = vec![
        Statement::Chmod {
            path: PathExpr::new(PathBase::Pkgshare, &["pear", ".channels"]),
            mode: 0o755,
        },
        Statement::Chmod {
            path: PathExpr::new(PathBase::Pkgshare, &["pear", ".channels", ".alias"]),
            mode: 0o755,
        },
        Statement::GlobChmod {
            dir: PathExpr::new(PathBase::Pkgshare, &["pear", ".channels"]),
            pattern: "*".to_owned(),
            mode: 0o755,
        },
        Statement::GlobChmod {
            dir: PathExpr::new(PathBase::Pkgshare, &["pear", ".channels", ".alias"]),
            pattern: "*".to_owned(),
            mode: 0o755,
        },
    ];

    for path in [
        PathExpr::new(PathBase::Pkgshare, &["pear", ".depdblock"]),
        PathExpr::new(PathBase::Pkgshare, &["pear", ".filemap"]),
        PathExpr::new(PathBase::Pkgshare, &["pear", ".depdb"]),
        PathExpr::new(PathBase::Pkgshare, &["pear", ".lock"]),
    ] {
        statements.push(Statement::Chmod { path, mode: 0o644 });
    }

    statements.extend([
        Statement::Mkpath(pecl_path.clone()),
        Statement::IfPath {
            condition: PathExpr::new(PathBase::Prefix, &["pecl"]),
            kind: PathCondition::Missing,
            then_branch: vec![Statement::ForceSymlink {
                target: pecl_path,
                link: PathExpr::new(PathBase::Prefix, &["pecl"]),
            }],
        },
        Statement::ProcessCapture {
            variable: "extension_dir".to_owned(),
            command: vec![
                Argument::Path(PathExpr::new(PathBase::Bin, &["php-config"])),
                Argument::String("--extension-dir".to_owned()),
            ],
        },
        Statement::Mkpath(pecl_ext_dir.clone()),
        Statement::RecursiveCopy {
            from: PathExpr::new(PathBase::Pkgshare, &["pear"]),
            to: hp(&["share"]),
        },
    ]);

    for path in [
        pear_path.clone(),
        append_segment(&pear_path, "doc"),
        pecl_ext_dir.clone(),
        append_segment(&pear_path, "data"),
        append_segment(&pear_path, "cfg"),
        append_segment(&pear_path, "htdocs"),
        append_segment(&pear_path, "test"),
    ] {
        statements.push(Statement::Mkpath(path));
    }

    for (key, value) in [
        ("php_ini", append_segment(&etc_php, "php.ini")),
        ("php_dir", pear_path.clone()),
        ("doc_dir", append_segment(&pear_path, "doc")),
        ("ext_dir", pecl_ext_dir),
        ("bin_dir", opt_php_bin.clone()),
        ("data_dir", append_segment(&pear_path, "data")),
        ("cfg_dir", append_segment(&pear_path, "cfg")),
        ("www_dir", append_segment(&pear_path, "htdocs")),
        ("man_dir", hp(&["share", "man"])),
        ("test_dir", append_segment(&pear_path, "test")),
        ("php_bin", append_segment(&opt_php_bin, "php")),
    ] {
        statements.push(Statement::System(vec![
            Argument::Path(PathExpr::new(PathBase::Bin, &["pear"])),
            Argument::String("config-set".to_owned()),
            Argument::String(key.to_owned()),
            Argument::Path(value),
            Argument::String("system".to_owned()),
        ]));
    }

    statements.push(Statement::System(vec![
        Argument::Path(PathExpr::new(PathBase::Bin, &["pear"])),
        Argument::String("update-channels".to_owned()),
    ]));

    statements
}

fn extract_resource_version(source: &str, resource_name: &str) -> Result<String, AnalysisError> {
    let marker = format!("resource \"{resource_name}\"");
    let Some(start) = source.find(&marker) else {
        return unsupported(&format!("missing resource block: {resource_name}"));
    };
    let tail = &source[start..];
    let Some(url_index) = tail.find("url ") else {
        return unsupported(&format!("missing resource url: {resource_name}"));
    };
    let url_tail = &tail[url_index + 4..];
    let Some(first_quote) = url_tail.find('"') else {
        return unsupported(&format!("missing resource url quote: {resource_name}"));
    };
    let url_tail = &url_tail[first_quote + 1..];
    let Some(second_quote) = url_tail.find('"') else {
        return unsupported(&format!("unterminated resource url: {resource_name}"));
    };
    let url = &url_tail[..second_quote];
    let Some(file_name) = url.rsplit('/').next() else {
        return unsupported(&format!("invalid resource url: {resource_name}"));
    };
    let Some(stripped) = file_name.strip_suffix(".tar.gz") else {
        return unsupported(&format!(
            "unexpected resource archive suffix: {resource_name}"
        ));
    };
    let prefix = format!("{resource_name}-");
    stripped
        .strip_prefix(&prefix)
        .map(str::to_owned)
        .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
            message: format!("resource url does not encode version: {resource_name}"),
        })
}
