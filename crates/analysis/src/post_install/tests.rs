use super::*;

#[test]
fn test_extract_post_install_block() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    if (prefix/"flag").exist?
      cp share/"src.txt", var/"demo/dst.txt"
    end
  end
end
"#;

    let block = extract_post_install_block(source)?;

    assert!(block.contains(r#"(var/"demo").mkpath"#));
    assert!(block.contains(r#"cp share/"src.txt", var/"demo/dst.txt""#));
    Ok(())
}

#[test]
fn test_validate_bundle_bootstrap_with_helper() -> Result<(), Box<dyn std::error::Error>> {
    let direct = r#"
class CaCertificates < Formula
  def post_install
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write(File.read(pkgshare/"cacert.pem"))
  end
end
"#;
    lower_post_install(direct, "2024.01")?;

    let with_helper = r#"
class CurlCaBundle < Formula
  def openssldir
    pkgetc
  end
  def post_install
    openssldir.mkpath
    (openssldir/"cert.pem").atomic_write(File.read(pkgshare/"cacert.pem"))
  end
end
"#;
    lower_post_install(with_helper, "2024.01")?;
    Ok(())
}

#[test]
fn test_lower_post_install_rejects_empty_source() {
    let result = lower_post_install("", "1.0");
    assert!(result.is_err());
}

#[test]
fn test_lower_post_install_rejects_unsupported_syntax() {
    let source = r#"
class Demo < Formula
  def post_install
    require "fileutils"
  end
end
"#;
    let result = lower_post_install(source, "1.0");
    assert!(result.is_err());
}

#[test]
fn test_quiet_system_lowered_as_system() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Gnupg < Formula
  def post_install
    (var/"run").mkpath
    quiet_system "killall", "gpg-agent"
  end
end
"#;
    let program = lower_post_install(source, "2.4.1")?;
    assert_eq!(
        program.statements,
        vec![
            Statement::Mkpath(PathExpr::new(PathBase::Var, &["run"])),
            Statement::System(vec![
                Argument::String("killall".to_owned()),
                Argument::String("gpg-agent".to_owned()),
            ]),
        ]
    );
    Ok(())
}

#[test]
fn test_atomic_write_string_lowered_as_write_file() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Node22 < Formula
  def post_install
    (lib/"node_modules/npm/npmrc").atomic_write("prefix = #{HOMEBREW_PREFIX}\n")
  end
end
"#;
    let program = lower_post_install(source, "22.0.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::WriteFile {
            path: PathExpr::new(PathBase::Lib, &["node_modules", "npm", "npmrc"]),
            content: vec![
                ContentPart::Literal("prefix = ".to_owned()),
                ContentPart::HomebrewPrefix,
                ContentPart::Literal("\n".to_owned()),
            ],
        }]
    );
    Ok(())
}

#[test]
fn test_atomic_write_plain_string() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    (etc/"demo.conf").atomic_write("hello\n")
  end
end
"#;
    let program = lower_post_install(source, "1.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::WriteFile {
            path: PathExpr::new(PathBase::Etc, &["demo.conf"]),
            content: vec![ContentPart::Literal("hello\n".to_owned())],
        }]
    );
    Ok(())
}

#[test]
fn test_bundle_bootstrap_atomic_write_still_produces_copy() -> Result<(), Box<dyn std::error::Error>>
{
    let source = r#"
class CaCertificates < Formula
  def post_install
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write(File.read(pkgshare/"cacert.pem"))
  end
end
"#;
    let program = lower_post_install(source, "2024.01")?;
    assert!(
        program
            .statements
            .iter()
            .any(|s| matches!(s, Statement::Copy { .. }))
    );
    assert!(
        !program
            .statements
            .iter()
            .any(|s| matches!(s, Statement::WriteFile { .. }))
    );
    Ok(())
}

#[test]
fn test_return_unless_os_mac_is_skipped() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    return unless OS.mac?
    (var/"run").mkpath
  end
end
"#;
    let program = lower_post_install(source, "1.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::Mkpath(PathExpr::new(PathBase::Var, &["run"]))]
    );
    Ok(())
}

#[test]
fn test_unless_os_mac_alone_produces_empty_program() -> Result<(), Box<dyn std::error::Error>> {
    let source = r"
class Demo < Formula
  def post_install
    return unless OS.mac?
  end
end
";
    let program = lower_post_install(source, "1.0")?;
    assert!(program.statements.is_empty());
    Ok(())
}

#[test]
fn test_install_symlink_multiple_args() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Rustup < Formula
  def post_install
    (HOMEBREW_PREFIX/"bin").install_symlink bin/"rustup", bin/"rustup-init"
  end
end
"#;
    let program = lower_post_install(source, "1.0")?;
    let hp_bin = PathExpr::new(PathBase::HomebrewPrefix, &["bin"]);
    assert_eq!(
        program.statements,
        vec![
            Statement::InstallSymlink {
                link_dir: hp_bin.clone(),
                target: PathExpr::new(PathBase::Bin, &["rustup"]),
            },
            Statement::InstallSymlink {
                link_dir: hp_bin,
                target: PathExpr::new(PathBase::Bin, &["rustup-init"]),
            },
        ]
    );
    Ok(())
}

#[test]
fn test_mkdir_p_without_receiver() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Glibc < Formula
  def post_install
    mkdir_p lib/"locale"
  end
end
"#;
    let program = lower_post_install(source, "2.39")?;
    assert_eq!(
        program.statements,
        vec![Statement::Mkpath(PathExpr::new(PathBase::Lib, &["locale"]))]
    );
    Ok(())
}

#[test]
fn test_system_with_formula_opt_bin_interpolation() -> Result<(), Box<dyn std::error::Error>> {
    let source = r##"
class Gtk4 < Formula
  def post_install
    system "#{Formula["glib"].opt_bin}/glib-compile-schemas", "#{HOMEBREW_PREFIX}/share/glib-2.0/schemas"
  end
end
"##;
    let program = lower_post_install(source, "4.0.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::System(vec![
            Argument::Path(PathExpr::new(
                PathBase::FormulaOptBin("glib".to_owned()),
                &["glib-compile-schemas"]
            )),
            Argument::Path(PathExpr::new(
                PathBase::HomebrewPrefix,
                &["share", "glib-2.0", "schemas"]
            )),
        ])]
    );
    Ok(())
}

#[test]
fn test_install_statement_lowered_from_bin_install() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Buildapp < Formula
  def post_install
    if (prefix/"buildapp.gz").exist?
      system "gunzip", prefix/"buildapp.gz"
      bin.install prefix/"buildapp"
      (bin/"buildapp").chmod 0755
    end
  end
end
"#;
    let program = lower_post_install(source, "1.5.6")?;
    assert_eq!(
        program.statements,
        vec![Statement::IfPath {
            condition: PathExpr::new(PathBase::Prefix, &["buildapp.gz"]),
            kind: PathCondition::Exists,
            then_branch: vec![
                Statement::System(vec![
                    Argument::String("gunzip".to_owned()),
                    Argument::Path(PathExpr::new(PathBase::Prefix, &["buildapp.gz"])),
                ]),
                Statement::Install {
                    into_dir: PathExpr::new(PathBase::Bin, &[]),
                    from: PathExpr::new(PathBase::Prefix, &["buildapp"]),
                },
                Statement::Chmod {
                    path: PathExpr::new(PathBase::Bin, &["buildapp"]),
                    mode: 0o755,
                },
            ],
        }]
    );
    Ok(())
}

#[test]
fn test_tier2_name_attribute_in_path_join() -> Result<(), Box<dyn std::error::Error>> {
    let source = r"
class Demo < Formula
  def post_install
    (etc/name).mkpath
  end
end
";
    assert!(lower_post_install(source, "1.0").is_err());

    let program = lower_post_install_tier2(source, "1.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::Mkpath(PathExpr {
            base: PathBase::Etc,
            segments: vec![PathSegment::Interpolated(vec![SegmentPart::FormulaName])],
        })]
    );
    Ok(())
}

#[test]
fn test_tier2_version_major_minor_in_interpolated_segment() -> Result<(), Box<dyn std::error::Error>>
{
    let source = r"
class Demo < Formula
  def post_install
    mkdir_p lib/version.major_minor
  end
end
";
    assert!(lower_post_install(source, "1.6.0").is_err());

    let program = lower_post_install_tier2(source, "1.6.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::Mkpath(PathExpr {
            base: PathBase::Lib,
            segments: vec![PathSegment::Interpolated(vec![
                SegmentPart::VersionMajorMinor,
            ])],
        })]
    );
    Ok(())
}

#[test]
fn test_tier2_name_in_chained_path_join() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    (HOMEBREW_PREFIX/"share"/name).mkpath
  end
end
"#;
    assert!(lower_post_install(source, "1.0").is_err());

    let program = lower_post_install_tier2(source, "1.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::Mkpath(PathExpr {
            base: PathBase::HomebrewPrefix,
            segments: vec![
                PathSegment::Literal("share".to_owned()),
                PathSegment::Interpolated(vec![SegmentPart::FormulaName]),
            ],
        })]
    );
    Ok(())
}

#[test]
fn test_tier1_still_rejects_name() {
    let source = r"
class Demo < Formula
  def post_install
    (etc/name).mkpath
  end
end
";
    assert!(lower_post_install(source, "1.0").is_err());
}

#[test]
fn test_tier2_postgresql_family_schema() -> Result<(), Box<dyn std::error::Error>> {
    let source = r##"
class PostgresqlAT17 < Formula
  def postgresql_datadir
    var/name
  end
  def pg_version_exists?
    (postgresql_datadir/"PG_VERSION").exist?
  end
  def post_install
    (var/"log").mkpath
    postgresql_datadir.mkpath

    %w[include lib share].each do |dir|
      dst_dir = HOMEBREW_PREFIX/dir/name
      src_dir = prefix/dir/"postgresql"
      src_dir.find do |src|
        dst = dst_dir/src.relative_path_from(src_dir)
        next if dst.directory? && !dst.symlink? && src.directory? && !src.symlink?
        rm_r(dst) if dst.exist? || dst.symlink?
        if src.symlink? || src.file?
          Find.prune if src.basename.to_s == ".DS_Store"
          dst.parent.install_symlink src
        elsif src.directory?
          dst.mkpath
        end
      end
    end

    bin.each_child { |f| (HOMEBREW_PREFIX/"bin").install_symlink f => "#{f.basename}-#{version.major}" }

    return if ENV["HOMEBREW_GITHUB_ACTIONS"]

    system bin/"initdb", "--locale=en_US.UTF-8", "-E", "UTF-8", postgresql_datadir unless pg_version_exists?
  end
end
"##;

    let program = lower_post_install_tier2(source, "17.2")?;
    let stmts = &program.statements;

    assert!(
        stmts.iter().any(
            |s| matches!(s, Statement::Mkpath(p) if p.base == PathBase::Var
                && p.segments == [PathSegment::Literal("log".to_owned())])
        ),
        "should have Mkpath(var/log)"
    );

    assert!(
        stmts.iter().any(|s| matches!(s, Statement::Mkpath(p)
            if p.base == PathBase::Var
                && p.segments == [PathSegment::Interpolated(vec![SegmentPart::FormulaName])])),
        "should have Mkpath(var/<name>)"
    );

    let mirror_count = stmts
        .iter()
        .filter(|s| matches!(s, Statement::MirrorTree { .. }))
        .count();
    assert_eq!(mirror_count, 3, "should have 3 MirrorTree statements");

    assert!(
        stmts
            .iter()
            .any(|s| matches!(s, Statement::ChildrenSymlink { .. })),
        "should have ChildrenSymlink"
    );

    assert!(
        stmts.iter().any(
            |s| matches!(s, Statement::IfEnv { variable, negate: true, .. } if variable == "HOMEBREW_GITHUB_ACTIONS")
        ),
        "should have IfEnv guard"
    );

    Ok(())
}

#[test]
fn test_tier2_llvm_clang_config_schema() -> Result<(), Box<dyn std::error::Error>> {
    let source = r##"
class Llvm < Formula
  def clang_config_file_dir
    etc/"clang"
  end
  def post_install
    return unless OS.mac?

    config_files = {
      darwin: OS.kernel_version.major,
      macosx: MacOS.version,
    }.map do |system, version|
      clang_config_file_dir/"#{Hardware::CPU.arch}-apple-#{system}#{version}.cfg"
    end
    return if config_files.all?(&:exist?)

    write_config_files(MacOS.version, OS.kernel_version.major, Hardware::CPU.arch)
  end

  def write_config_files(macos_version, kernel_version, arch)
    clang_config_file_dir.mkpath

    arches = Set.new([:arm64, :x86_64, :aarch64])
    arches << arch

    sysroot = if macos_version.blank? || MacOS.version > macos_version
      "#{MacOS::CLT::PKG_PATH}/SDKs/MacOSX.sdk"
    else
      "#{MacOS::CLT::PKG_PATH}/SDKs/MacOSX#{macos_version}.sdk"
    end

    {
      darwin: kernel_version,
      macosx: macos_version,
    }.each do |system, version|
      arches.each do |target_arch|
        config_file = "#{target_arch}-apple-#{system}#{version}.cfg"
        (clang_config_file_dir/config_file).atomic_write <<~CONFIG
          -isysroot #{sysroot}
        CONFIG
      end
    end
  end
end
"##;

    let program = lower_post_install_tier2(source, "19.1.7")?;
    let stmts = &program.statements;

    assert!(
        stmts
            .iter()
            .any(|s| matches!(s, Statement::Mkpath(p) if p.base == PathBase::Etc)),
        "should create clang config dir"
    );

    let write_count = stmts
        .iter()
        .filter(|s| matches!(s, Statement::WriteFile { .. }))
        .count();
    assert!(
        write_count > 0,
        "should have WriteFile statements for config files"
    );

    Ok(())
}

#[test]
fn test_tier2_safe_popen_read_lowering() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    output = Utils.safe_popen_read(bin/"myapp", "--version")
  end
end
"#;
    assert!(lower_post_install(source, "1.0").is_err());

    let program = lower_post_install_tier2(source, "1.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::ProcessCapture {
            variable: "output".to_owned(),
            command: vec![
                Argument::Path(PathExpr::new(PathBase::Bin, &["myapp"])),
                Argument::String("--version".to_owned()),
            ],
        }]
    );

    Ok(())
}

#[test]
fn test_parent_path_expression_lowers() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    (lib/"python3.11/site-packages").parent.mkpath
  end
end
"#;

    let program = lower_post_install(source, "1.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::Mkpath(PathExpr::new(
            PathBase::Lib,
            &["python3.11"]
        ))]
    );
    Ok(())
}

#[test]
fn test_env_assignment_lowers_to_set_env() -> Result<(), Box<dyn std::error::Error>> {
    let source = r##"
class Librsvg < Formula
  def post_install
    ENV["GDK_PIXBUF_MODULEDIR"] = "#{HOMEBREW_PREFIX}/lib/gdk-pixbuf-2.0/2.10.0/loaders"
    system "gdk-pixbuf-query-loaders", "--update-cache"
  end
end
"##;

    let program = lower_post_install(source, "2.61.2")?;
    assert_eq!(
        program.statements[0],
        Statement::SetEnv {
            variable: "GDK_PIXBUF_MODULEDIR".to_owned(),
            value: vec![
                ContentPart::HomebrewPrefix,
                ContentPart::Literal("/lib/gdk-pixbuf-2.0/2.10.0/loaders".to_owned()),
            ],
        }
    );
    Ok(())
}

#[test]
fn test_gdk_pixbuf_loader_schema_uses_helper_literals() -> Result<(), Box<dyn std::error::Error>> {
    let source = r##"
class GdkPixbuf < Formula
  def gdk_so_ver
    "2.0"
  end

  def gdk_module_ver
    "2.10.0"
  end

  def module_dir
    "#{HOMEBREW_PREFIX}/lib/gdk-pixbuf-#{gdk_so_ver}/#{gdk_module_ver}"
  end

  def post_install
    ENV["GDK_PIXBUF_MODULEDIR"] = "#{module_dir}/loaders"
    system bin/"gdk-pixbuf-query-loaders", "--update-cache"
  end
end
"##;

    let program = lower_post_install(source, "2.44.5")?;
    assert_eq!(
        program.statements,
        vec![
            Statement::SetEnv {
                variable: "GDK_PIXBUF_MODULEDIR".to_owned(),
                value: vec![
                    ContentPart::HomebrewPrefix,
                    ContentPart::Literal("/lib/gdk-pixbuf-2.0/2.10.0/loaders".to_owned()),
                ],
            },
            Statement::System(vec![
                Argument::Path(PathExpr::new(PathBase::Bin, &["gdk-pixbuf-query-loaders"])),
                Argument::String("--update-cache".to_owned()),
            ]),
        ]
    );
    Ok(())
}

#[test]
fn test_mysql_schema_lowers_required_mkpath() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Mysql < Formula
  def post_install
    (var/"mysql").mkpath

    if (my_cnf = ["/etc/my.cnf", "/etc/mysql/my.cnf"].find { |x| File.exist? x })
      opoo "conflict: #{my_cnf}"
    end
  end
end
"#;

    let program = lower_post_install(source, "9.0.0")?;
    assert_eq!(
        program.statements,
        vec![Statement::Mkpath(PathExpr::new(PathBase::Var, &["mysql"]))]
    );
    Ok(())
}

#[test]
fn test_basic_postgresql_schema_uses_missing_guard() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class PostgresqlAT14 < Formula
  def postgresql_datadir
    var/name
  end

  def post_install
    (var/"log").mkpath
    postgresql_datadir.mkpath
    old_postgres_data_dir = var/"postgres"
    if old_postgres_data_dir.exist?
      opoo "legacy dir"
    end
    return if ENV["HOMEBREW_GITHUB_ACTIONS"]
    system bin/"initdb", "--locale=en_US.UTF-8", "-E", "UTF-8", postgresql_datadir unless pg_version_exists?
  end

  def pg_version_exists?
    (postgresql_datadir/"PG_VERSION").exist?
  end
end
"#;

    let program = lower_post_install_tier2(source, "14.18")?;
    assert!(
        program.statements.iter().any(|statement| matches!(
            statement,
            Statement::IfEnv { then_branch, .. }
                if then_branch.iter().any(|inner| matches!(
                    inner,
                    Statement::IfPath { kind: PathCondition::Missing, .. }
                ))
        )),
        "expected initdb to be guarded by Missing(PG_VERSION)"
    );
    Ok(())
}

#[test]
fn test_python_site_packages_schema_lowers() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class PythonAT311 < Formula
  resource "pip" do
    url "https://files.pythonhosted.org/packages/foo/pip-26.0.1.tar.gz"
  end

  resource "setuptools" do
    url "https://files.pythonhosted.org/packages/foo/setuptools-82.0.0.tar.gz"
  end

  resource "wheel" do
    url "https://files.pythonhosted.org/packages/foo/wheel-0.46.3.tar.gz"
  end

  def lib_cellar
    on_macos do
      return frameworks/"Python.framework/Versions"/version.major_minor/"lib/python#{version.major_minor}"
    end
  end

  def site_packages_cellar
    lib_cellar/"site-packages"
  end

  def site_packages
    HOMEBREW_PREFIX/"lib/python#{version.major_minor}/site-packages"
  end

  def python3
    bin/"python#{version.major_minor}"
  end

  def post_install
    ENV.delete "PYTHONPATH"
    site_packages.mkpath
    site_packages_cellar.unlink if site_packages_cellar.exist?
    site_packages_cellar.parent.install_symlink site_packages
    system python3, "-Im", "ensurepip"
    bundled = lib_cellar/"ensurepip/_bundled"
    system python3, "-Im", "pip", "install", "-v",
           "--no-deps",
           "--no-index",
           "--upgrade",
           "--isolated",
           "--target=#{site_packages}",
           bundled/"setuptools-#{resource("setuptools").version}-py3-none-any.whl",
           bundled/"pip-#{resource("pip").version}-py3-none-any.whl",
           libexec/"wheel-#{resource("wheel").version}-py3-none-any.whl"
    mv (site_packages/"bin").children, bin
    rmdir site_packages/"bin"
    rm_r(bin.glob("pip{,3}"))
    mv bin/"wheel", bin/"wheel#{version.major_minor}"
  end
end
"#;

    let program = lower_post_install_tier2(source, "3.11.15")?;
    assert!(
        program
            .statements
            .iter()
            .any(|statement| matches!(statement, Statement::MoveChildren { .. }))
    );
    assert!(
        program
            .statements
            .iter()
            .any(|statement| matches!(statement, Statement::Move { .. }))
    );
    assert!(program.statements.iter().any(|statement| matches!(
        statement,
        Statement::System(arguments)
            if arguments.iter().any(|argument| matches!(
                argument,
                Argument::String(value) if value == "ensurepip"
            ))
    )));
    Ok(())
}

#[test]
fn test_php_pear_schema_lowers() -> Result<(), Box<dyn std::error::Error>> {
    let source = r##"
class Php < Formula
  def post_install
    pear_prefix = pkgshare/"pear"
    pear_files = %W[
      #{pear_prefix}/.depdblock
      #{pear_prefix}/.filemap
      #{pear_prefix}/.depdb
      #{pear_prefix}/.lock
    ]

    %W[
      #{pear_prefix}/.channels
      #{pear_prefix}/.channels/.alias
    ].each do |f|
      chmod 0755, f
      pear_files.concat(Dir["#{f}/*"])
    end

    chmod 0644, pear_files
    pecl_path = HOMEBREW_PREFIX/"lib/php/pecl"
    pecl_path.mkpath
    ln_s pecl_path, prefix/"pecl" unless (prefix/"pecl").exist?
    extension_dir = Utils.safe_popen_read(bin/"php-config", "--extension-dir").chomp
    php_basename = File.basename(extension_dir)
    (pecl_path/php_basename).mkpath
    pear_path = HOMEBREW_PREFIX/"share"/"pear"
    cp_r pkgshare/"pear/.", pear_path
    {
      "php_ini"  => etc/"php/#{version.major_minor}/php.ini",
      "php_dir"  => pear_path,
      "doc_dir"  => pear_path/"doc",
      "ext_dir"  => pecl_path/php_basename,
      "bin_dir"  => opt_bin,
      "data_dir" => pear_path/"data",
      "cfg_dir"  => pear_path/"cfg",
      "www_dir"  => pear_path/"htdocs",
      "man_dir"  => HOMEBREW_PREFIX/"share/man",
      "test_dir" => pear_path/"test",
      "php_bin"  => opt_bin/"php",
    }.each do |key, value|
      value.mkpath if /(?<!bin|man)_dir$/.match?(key)
      system bin/"pear", "config-set", key, value, "system"
    end

    system bin/"pear", "update-channels"
  end
end
"##;

    let program = lower_post_install_tier2(source, "8.5.4")?;
    assert!(program.statements.iter().any(|statement| matches!(
        statement,
        Statement::ProcessCapture { variable, .. } if variable == "extension_dir"
    )));
    assert!(
        program
            .statements
            .iter()
            .any(|statement| matches!(statement, Statement::GlobChmod { .. }))
    );
    assert!(program.statements.iter().any(|statement| matches!(
        statement,
        Statement::System(arguments)
            if arguments.iter().any(|argument| matches!(
                argument,
                Argument::String(value) if value == "update-channels"
            ))
    )));
    Ok(())
}

// ---------------------------------------------------------------------------
// Feature census tests
// ---------------------------------------------------------------------------

#[test]
fn test_features_system_mkpath() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    mkdir_p prefix/"etc"
    system "echo", "hello"
  end
end
"#;

    let analysis = analyze_post_install_all(source, "1.0")?.ok_or("should find block")?;

    assert!(analysis.features.mkpath);
    assert!(analysis.features.mkdir_p);
    assert!(analysis.features.system);
    assert!(analysis.features.var);
    assert!(analysis.features.prefix);
    Ok(())
}

#[test]
fn test_features_env() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    ENV["FOO"] = "bar"
  end
end
"#;

    let analysis = analyze_post_install_all(source, "1.0")?.ok_or("should find block")?;

    assert!(analysis.features.env);
    Ok(())
}

#[test]
fn test_features_os_condition() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    if OS.mac?
      system "echo", "mac"
    end
  end
end
"#;

    let analysis = analyze_post_install_all(source, "1.0")?.ok_or("should find block")?;

    assert!(analysis.features.os_condition);
    assert!(analysis.features.system);
    Ok(())
}

#[test]
fn test_features_path_bases() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    (bin/"demo").install_symlink prefix/"lib/demo"
    (share/"doc").mkpath
  end
end
"#;

    let analysis = analyze_post_install_all(source, "1.0")?.ok_or("should find block")?;

    assert!(analysis.features.bin);
    assert!(analysis.features.prefix);
    assert!(analysis.features.share);
    assert!(analysis.features.install_symlink);
    assert!(analysis.features.mkpath);
    Ok(())
}

#[test]
fn test_features_helper_methods() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    configure_foo
  end

  def configure_foo
    (prefix/"etc").mkpath
  end
end
"#;

    let analysis = analyze_post_install_all(source, "1.0")?.ok_or("should find block")?;

    assert!(analysis.features.helper_methods);
    Ok(())
}

#[test]
fn test_analyze_all_no_block() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def install
    system "make"
  end
end
"#;

    let result = analyze_post_install_all(source, "1.0")?;

    assert!(result.is_none());
    Ok(())
}

#[test]
fn test_analyze_all_lowerable() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    system "echo", "done"
  end
end
"#;

    let analysis = analyze_post_install_all(source, "1.0")?.ok_or("should find block")?;

    assert!(analysis.features.mkpath);
    assert!(analysis.features.system);
    assert!(analysis.lowering.is_ok());
    Ok(())
}

#[test]
fn test_analyze_all_unlowerable() -> Result<(), Box<dyn std::error::Error>> {
    let source = r"
class Demo < Formula
  def post_install
    some_totally_unsupported_call
  end
end
";

    let analysis = analyze_post_install_all(source, "1.0")?.ok_or("should find block")?;

    assert!(analysis.lowering.is_err());
    Ok(())
}
