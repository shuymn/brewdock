use std::{os::unix::fs::PermissionsExt, path::Path};

use super::*;

fn test_platform() -> PlatformContext {
    PlatformContext {
        kernel_version_major: "24".to_owned(),
        macos_version: "15.1".to_owned(),
        cpu_arch: "arm64".to_owned(),
    }
}

fn test_context(prefix: &Path, keg: &Path, version: &str) -> PostInstallContext {
    PostInstallContext::new(prefix, keg, version, &test_platform())
}

fn write_executable(path: &Path, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(path, contents)?;
    let mut perms = std::fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

fn shared_mime_info_post_install_source() -> &'static str {
    r#"
class SharedMimeInfo < Formula
  def post_install
    global_mime = HOMEBREW_PREFIX/"share/mime"
    cellar_mime = share/"mime"

    rm_r(global_mime) if global_mime.symlink?
    rm_r(cellar_mime) if cellar_mime.exist? && !cellar_mime.symlink?
    ln_sf(global_mime, cellar_mime)

    (global_mime/"packages").mkpath
    cp (pkgshare/"packages").children, global_mime/"packages"

    system bin/"update-mime-database", global_mime
  end
end
"#
}

#[test]
fn test_run_post_install_supported_subset() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(keg.join("share"))?;
    std::fs::create_dir_all(keg.join("bin"))?;
    std::fs::write(keg.join("share/src.txt"), "payload")?;
    write_executable(
        &keg.join("bin/write-flag"),
        "#!/bin/sh\nprintf '%s' \"$1\" > \"$2\"\n",
    )?;
    std::fs::write(keg.join("flag"), "go")?;

    let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    cp share/"src.txt", var/"demo/copied.txt"
    if (prefix/"flag").exist?
      system bin/"write-flag", "done", var/"demo/result.txt"
    end
  end
end
"#;

    run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

    assert_eq!(
        std::fs::read_to_string(prefix.join("var/demo/copied.txt"))?,
        "payload"
    );
    assert_eq!(
        std::fs::read_to_string(prefix.join("var/demo/result.txt"))?,
        "done"
    );
    Ok(())
}

#[test]
fn test_run_post_install_empty_source() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;

    let result = run_post_install("", &mut test_context(&prefix, &keg, "1.0"));
    assert!(matches!(result, Err(CellarError::Analysis(_))));
    Ok(())
}

#[test]
fn test_run_post_install_unsupported_syntax() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;
    let source = r#"
class Demo < Formula
  def post_install
    puts "nope"
  end
end
"#;

    let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));
    assert!(matches!(result, Err(CellarError::Analysis(_))));
    Ok(())
}

#[test]
fn test_run_post_install_ca_bundle() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/ca-certificates/1.0");
    std::fs::create_dir_all(keg.join("share/ca-certificates"))?;
    std::fs::write(
        keg.join("share/ca-certificates/cacert.pem"),
        "mozilla-bundle",
    )?;

    let source = r#"
class CaCertificates < Formula
  def post_install
    if OS.mac?
      macos_post_install
    else
      linux_post_install
    end
  end

  def macos_post_install
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write("ignored")
  end

  def linux_post_install
    cp pkgshare/"cacert.pem", pkgetc/"cert.pem"
  end
end
"#;

    run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

    assert_eq!(
        std::fs::read_to_string(prefix.join("etc/ca-certificates/cert.pem"))?,
        "mozilla-bundle"
    );
    Ok(())
}

#[test]
fn test_run_post_install_ca_bundle_helper() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/ca-certificates/1.0");
    std::fs::create_dir_all(keg.join("share/ca-certificates"))?;
    std::fs::write(
        keg.join("share/ca-certificates/cacert.pem"),
        "mozilla-bundle",
    )?;

    let source = r#"
class CaCertificates < Formula
  def post_install
    if OS.mac?
      bootstrap_bundle
    else
      unsupported_linux_path
    end
  end

  def bootstrap_bundle
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write("ignored")
  end

  def unsupported_linux_path
    puts "linux only"
  end
end
"#;

    run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

    assert_eq!(
        std::fs::read_to_string(prefix.join("etc/ca-certificates/cert.pem"))?,
        "mozilla-bundle"
    );
    Ok(())
}

#[test]
fn test_run_post_install_openssl_cert() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/openssl@3/1.0");
    std::fs::create_dir_all(prefix.join("etc/ca-certificates"))?;
    std::fs::write(prefix.join("etc/ca-certificates/cert.pem"), "bundle")?;

    let source = r#"
class OpensslAT3 < Formula
  def openssldir
    etc/"openssl@3"
  end

  def post_install
    rm(openssldir/"cert.pem") if (openssldir/"cert.pem").exist?
    openssldir.install_symlink Formula["ca-certificates"].pkgetc/"cert.pem"
  end
end
"#;

    run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

    let cert_link = prefix.join("etc/openssl@3/cert.pem");
    assert!(cert_link.is_symlink());
    assert_eq!(std::fs::read_to_string(cert_link)?, "bundle");
    Ok(())
}

#[test]
fn test_run_post_install_cert_symlink_helper() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/openssl@3/1.0");
    std::fs::create_dir_all(prefix.join("etc/ca-certificates"))?;
    std::fs::write(prefix.join("etc/ca-certificates/cert.pem"), "bundle")?;

    let source = r#"
class OpensslAT3 < Formula
  def cert_dir
    etc/"openssl@3"
  end

  def post_install
    rm(cert_dir/"cert.pem") if (cert_dir/"cert.pem").exist?
    cert_dir.install_symlink Formula["ca-certificates"].pkgetc/"cert.pem"
  end
end
"#;

    run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

    let cert_link = prefix.join("etc/openssl@3/cert.pem");
    assert!(cert_link.is_symlink());
    assert_eq!(std::fs::read_to_string(cert_link)?, "bundle");
    Ok(())
}

#[test]
fn test_run_post_install_rollback() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(keg.join("share"))?;
    std::fs::write(keg.join("share/src.txt"), "payload")?;

    let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    cp share/"src.txt", var/"demo/copied.txt"
    system "/bin/sh", "-c", "exit 1"
  end
end
"#;

    let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));

    assert!(matches!(
        result,
        Err(CellarError::PostInstallCommandFailed { .. })
    ));
    assert!(!prefix.join("var/demo").exists());
    Ok(())
}

#[test]
fn test_run_post_install_rejects_path_traversal() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;

    let escape = prefix.join("escape");
    let source = r#"
class Demo < Formula
  def post_install
    (var/"demo"/".."/".."/"escape").mkpath
    system "/bin/sh", "-c", "exit 1"
  end
end
"#;

    let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));
    assert!(
        result.is_err(),
        "path traversal in post_install should fail closed before mutating outside the prefix"
    );
    assert!(
        !escape.exists(),
        "path traversal should not leave artifacts outside the prefix"
    );
    Ok(())
}

#[test]
fn test_run_post_install_rejects_parent_directory() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;

    let source = r#"
class Demo
  def post_install
    (HOMEBREW_PREFIX/".."/".."/"tmp"/"brewdock-owned").mkpath
  end
end
"#;

    let escaped = std::env::temp_dir().join("brewdock-owned");
    let _ = std::fs::remove_dir_all(&escaped);

    let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));

    assert!(
        result.is_err(),
        "post_install path traversal must fail closed before mutating outside prefix"
    );
    assert!(
        !escaped.exists(),
        "post_install must not create directories outside the prefix"
    );
    Ok(())
}

#[test]
fn test_run_post_install_rejects_atomic_write_traversal() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;

    let escape = prefix.join("escape.txt");
    let source = r#"
class Demo < Formula
  def post_install
    (etc/"demo"/".."/".."/"escape.txt").atomic_write("owned")
  end
end
"#;

    let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));
    assert!(result.is_err(), "atomic_write traversal should fail closed");
    assert!(
        !escape.exists(),
        "atomic_write traversal should not create files outside allowed roots"
    );
    Ok(())
}

#[test]
fn test_run_post_install_ruby_cleanup() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/ruby/3.4.2");
    std::fs::create_dir_all(&keg)?;

    let gems_dir = prefix.join("lib/ruby/gems/3.4.0");
    std::fs::create_dir_all(gems_dir.join("bin"))?;
    std::fs::write(gems_dir.join("bin/bundle"), "bundler")?;
    std::fs::write(gems_dir.join("bin/bundler"), "bundler")?;
    std::fs::create_dir_all(gems_dir.join("gems/bundler-2.5.0"))?;
    std::fs::write(gems_dir.join("gems/bundler-2.5.0/fake"), "content")?;
    std::fs::create_dir_all(gems_dir.join("gems/rake-13.0.0"))?;
    std::fs::write(gems_dir.join("gems/rake-13.0.0/keep"), "keep")?;

    let source = r##"
class Ruby < Formula
  def rubygems_bindir
    HOMEBREW_PREFIX/"lib/ruby/gems/#{api_version}/bin"
  end

  def api_version
    "#{version.major.to_i}.#{version.minor.to_i}.0"
  end

  def post_install
    rm(%W[
      #{rubygems_bindir}/bundle
      #{rubygems_bindir}/bundler
    ].select { |file| File.exist?(file) })
    rm_r(Dir[HOMEBREW_PREFIX/"lib/ruby/gems/#{api_version}/gems/bundler-*"])
  end
end
"##;

    let mut context = test_context(&prefix, &keg, "3.4.2");
    run_post_install(source, &mut context)?.commit()?;

    assert!(!gems_dir.join("bin/bundle").exists());
    assert!(!gems_dir.join("bin/bundler").exists());
    assert!(!gems_dir.join("gems/bundler-2.5.0").exists());
    assert!(
        gems_dir.join("gems/rake-13.0.0/keep").exists(),
        "non-bundler gems should be preserved"
    );
    Ok(())
}

#[test]
fn test_run_post_install_node_npm() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/node/22.0.0");

    std::fs::create_dir_all(keg.join("libexec/lib/node_modules/npm/bin"))?;
    std::fs::write(
        keg.join("libexec/lib/node_modules/npm/bin/npm-cli.js"),
        "npm",
    )?;
    std::fs::write(
        keg.join("libexec/lib/node_modules/npm/bin/npx-cli.js"),
        "npx",
    )?;
    std::fs::create_dir_all(keg.join("libexec/lib/node_modules/npm/man/man1"))?;
    std::fs::write(
        keg.join("libexec/lib/node_modules/npm/man/man1/npm.1"),
        "man",
    )?;
    std::fs::write(
        keg.join("libexec/lib/node_modules/npm/man/man1/package-lock.json.5"),
        "pkg-man",
    )?;
    std::fs::create_dir_all(keg.join("bin"))?;

    let source = r#"
class Node < Formula
  def post_install
    node_modules = HOMEBREW_PREFIX/"lib/node_modules"
    node_modules.mkpath
    rm_r node_modules/"npm" if (node_modules/"npm").exist?

    cp_r libexec/"lib/node_modules/npm", node_modules
    ln_sf node_modules/"npm/bin/npm-cli.js", bin/"npm"
    ln_sf node_modules/"npm/bin/npx-cli.js", bin/"npx"
    ln_sf bin/"npm", HOMEBREW_PREFIX/"bin/npm"
    ln_sf bin/"npx", HOMEBREW_PREFIX/"bin/npx"

    %w[man1 man5 man7].each do |man|
      mkdir_p HOMEBREW_PREFIX/"share/man/#{man}"
      rm(Dir[HOMEBREW_PREFIX/"share/man/#{man}/{npm.,npm-,npmrc.,package.json.,npx.}*"])
      ln_sf Dir[node_modules/"npm/man/#{man}/{npm,package-,shrinkwrap-,npx}*"], HOMEBREW_PREFIX/"share/man/#{man}"
    end

    (node_modules/"npm/npmrc").atomic_write("prefix = #{HOMEBREW_PREFIX}\n")
  end
end
"#;

    let mut context = test_context(&prefix, &keg, "22.0.0");
    run_post_install(source, &mut context)?.commit()?;

    assert!(
        prefix.join("lib/node_modules/npm/bin/npm-cli.js").exists(),
        "npm should be copied to prefix"
    );
    assert!(
        keg.join("bin/npm").is_symlink(),
        "bin/npm should be a symlink"
    );
    assert!(
        keg.join("bin/npx").is_symlink(),
        "bin/npx should be a symlink"
    );
    assert!(
        prefix.join("bin/npm").is_symlink(),
        "prefix bin/npm should be a symlink"
    );
    assert!(
        prefix.join("bin/npx").is_symlink(),
        "prefix bin/npx should be a symlink"
    );

    let npmrc = std::fs::read_to_string(prefix.join("lib/node_modules/npm/npmrc"))?;
    assert!(
        npmrc.starts_with("prefix = "),
        "npmrc should contain prefix setting"
    );
    assert!(
        npmrc.contains(&prefix.to_string_lossy().to_string()),
        "npmrc prefix should point to the actual prefix path"
    );

    assert!(
        prefix.join("share/man/man1").is_dir(),
        "man1 dir should exist"
    );
    let man1_npm = prefix.join("share/man/man1/npm.1");
    assert!(man1_npm.is_symlink(), "npm.1 man page should be symlinked");
    let man1_pkg = prefix.join("share/man/man1/package-lock.json.5");
    assert!(
        man1_pkg.is_symlink(),
        "package-lock.json.5 should be symlinked (matches package- prefix)"
    );

    Ok(())
}

#[test]
fn test_run_post_install_ignores_ohai_logging() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/fontconfig/2.16.0");
    std::fs::create_dir_all(keg.join("bin"))?;
    write_executable(&keg.join("bin/fc-cache"), "#!/bin/sh\ntrue\n")?;

    let source = r#"
class Fontconfig < Formula
  def post_install
    ohai "Regenerating font cache, this may take a while"
    system bin/"fc-cache", "--force"
  end
end
"#;

    run_post_install(source, &mut test_context(&prefix, &keg, "2.16.0"))?.commit()?;
    Ok(())
}

#[test]
fn test_run_post_install_homebrew_prefix() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/glib/2.88.0");
    std::fs::create_dir_all(&keg)?;

    let source = r#"
class Glib < Formula
  def post_install
    (HOMEBREW_PREFIX/"lib/gio/modules").mkpath
  end
end
"#;

    run_post_install(source, &mut test_context(&prefix, &keg, "2.88.0"))?.commit()?;
    assert!(prefix.join("lib/gio/modules").is_dir());
    Ok(())
}

#[test]
fn test_run_post_install_shared_mime_info() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/shared-mime-info/2.4");
    std::fs::create_dir_all(keg.join("bin"))?;
    std::fs::create_dir_all(keg.join("share/shared-mime-info/packages"))?;
    std::fs::write(
        keg.join("share/shared-mime-info/packages/freedesktop.org.xml"),
        "<mime-info/>",
    )?;

    let global_mime = prefix.join("share/mime");
    let stale_target = prefix.join("share/old-mime");
    std::fs::create_dir_all(&stale_target)?;
    std::fs::create_dir_all(global_mime.parent().unwrap_or(&prefix))?;
    std::os::unix::fs::symlink(&stale_target, &global_mime)?;

    let cellar_mime = keg.join("share/mime");
    std::fs::create_dir_all(&cellar_mime)?;
    std::fs::write(cellar_mime.join("stale.cache"), "old")?;

    write_executable(
        &keg.join("bin/update-mime-database"),
        "#!/bin/sh\nmkdir -p \"$1\"\ntouch \"$1/mime.cache\"\n",
    )?;

    run_post_install(
        shared_mime_info_post_install_source(),
        &mut test_context(&prefix, &keg, "2.4"),
    )?
    .commit()?;

    assert!(global_mime.is_dir());
    assert_eq!(
        std::fs::read_to_string(global_mime.join("packages/freedesktop.org.xml"))?,
        "<mime-info/>"
    );
    assert!(global_mime.join("mime.cache").exists());
    assert!(cellar_mime.is_symlink());
    assert_eq!(
        std::fs::read_link(&cellar_mime)?,
        PathBuf::from("../../../../share/mime")
    );
    Ok(())
}

#[test]
fn test_run_post_install_formula_opt_bin() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/libheif/1.0");
    std::fs::create_dir_all(&keg)?;

    let shared_mime_keg = prefix.join("Cellar/shared-mime-info/2.4");
    std::fs::create_dir_all(shared_mime_keg.join("bin"))?;
    std::fs::create_dir_all(prefix.join("opt"))?;
    std::os::unix::fs::symlink(
        "../Cellar/shared-mime-info/2.4",
        prefix.join("opt/shared-mime-info"),
    )?;

    write_executable(
        &shared_mime_keg.join("bin/update-mime-database"),
        "#!/bin/sh\nmkdir -p \"$1\"\ntouch \"$1/mime.cache\"\n",
    )?;

    let source = r##"
class Libheif < Formula
  def post_install
    system Formula["shared-mime-info"].opt_bin/"update-mime-database", "#{HOMEBREW_PREFIX}/share/mime"
  end
end
"##;

    run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;
    assert!(prefix.join("share/mime/mime.cache").exists());
    Ok(())
}

#[test]
fn test_run_post_install_install_and_chmod() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/buildapp/1.5.6");
    std::fs::create_dir_all(keg.join("bin"))?;
    // Pre-create the source file at prefix/buildapp (as if gunzip already ran)
    std::fs::write(keg.join("buildapp"), "#!/bin/sh\necho hello\n")?;

    let source = r#"
class Buildapp < Formula
  def post_install
    bin.install prefix/"buildapp"
    (bin/"buildapp").chmod 0755
  end
end
"#;

    run_post_install(source, &mut test_context(&prefix, &keg, "1.5.6"))?.commit()?;

    // bin.install moves file into bin/
    let installed = keg.join("bin/buildapp");
    assert!(installed.exists(), "bin/buildapp should exist");
    assert!(!keg.join("buildapp").exists(), "source should be removed");
    assert_eq!(
        std::fs::read_to_string(&installed)?,
        "#!/bin/sh\necho hello\n"
    );

    // chmod 0755 sets executable permissions
    let perms = std::fs::metadata(&installed)?.permissions();
    assert_eq!(perms.mode() & 0o777, 0o755);
    Ok(())
}

#[test]
fn test_run_post_install_move_and_move_children() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(keg.join("src/bin"))?;
    std::fs::create_dir_all(keg.join("bin"))?;
    std::fs::write(keg.join("src/bin/tool"), "tool")?;
    std::fs::write(keg.join("bin/wheel"), "wheel")?;

    let program = Program {
        statements: vec![
            Statement::MoveChildren {
                from_dir: PathExpr::new(PathBase::Prefix, &["src", "bin"]),
                to_dir: PathExpr::new(PathBase::Bin, &[]),
            },
            Statement::Move {
                from: PathExpr::new(PathBase::Bin, &["wheel"]),
                to: PathExpr::new(PathBase::Bin, &["wheel3.11"]),
            },
        ],
    };

    let mut context = test_context(&prefix, &keg, "1.0");
    let rollback_roots = collect_rollback_roots(&program, &context);
    run_with_rollback(&rollback_roots, &mut context, |ctx| {
        execute_statements(&program.statements, ctx)
    })?
    .commit()?;

    assert_eq!(std::fs::read_to_string(keg.join("bin/tool"))?, "tool");
    assert_eq!(std::fs::read_to_string(keg.join("bin/wheel3.11"))?, "wheel");
    assert!(!keg.join("bin/wheel").exists());
    Ok(())
}

#[test]
fn test_run_post_install_glob_chmod() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(keg.join("share/demo"))?;
    std::fs::write(keg.join("share/demo/a.txt"), "a")?;
    std::fs::write(keg.join("share/demo/b.txt"), "b")?;

    let program = Program {
        statements: vec![Statement::GlobChmod {
            dir: PathExpr::new(PathBase::Pkgshare, &[]),
            pattern: "*".to_owned(),
            mode: 0o600,
        }],
    };

    let mut context = test_context(&prefix, &keg, "1.0");
    let rollback_roots = collect_rollback_roots(&program, &context);
    run_with_rollback(&rollback_roots, &mut context, |ctx| {
        execute_statements(&program.statements, ctx)
    })?
    .commit()?;

    for file in [keg.join("share/demo/a.txt"), keg.join("share/demo/b.txt")] {
        let perms = std::fs::metadata(file)?.permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }
    Ok(())
}

#[test]
fn test_mirror_tree_creates_symlink_structure() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(keg.join("include/postgresql/server"))?;
    std::fs::write(keg.join("include/postgresql/libpq-fe.h"), "header")?;
    std::fs::write(
        keg.join("include/postgresql/server/pg_config.h"),
        "server header",
    )?;
    std::fs::create_dir_all(&prefix)?;

    let dest_dir = prefix.join("include/demo");
    let mut context = test_context(&prefix, &keg, "1.0");

    // Manually execute MirrorTree
    execute_statements(
        &[Statement::MirrorTree {
            source: PathExpr::new(PathBase::Prefix, &["include", "postgresql"]),
            dest: PathExpr::new(PathBase::HomebrewPrefix, &["include", "demo"]),
            prune_names: vec![".DS_Store".to_owned()],
        }],
        &mut context,
    )?;

    // Check that symlinks were created
    assert!(dest_dir.join("libpq-fe.h").is_symlink());
    assert!(dest_dir.join("server").is_dir());
    assert!(dest_dir.join("server/pg_config.h").is_symlink());

    // Verify symlink targets resolve correctly
    assert_eq!(
        std::fs::read_to_string(dest_dir.join("libpq-fe.h"))?,
        "header"
    );
    assert_eq!(
        std::fs::read_to_string(dest_dir.join("server/pg_config.h"))?,
        "server header"
    );

    Ok(())
}

#[test]
fn test_children_symlink_with_suffix() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/postgresql/17.2");
    std::fs::create_dir_all(keg.join("bin"))?;
    std::fs::write(keg.join("bin/psql"), "#!/bin/sh\n")?;
    std::fs::write(keg.join("bin/pg_dump"), "#!/bin/sh\n")?;
    let link_dir = prefix.join("bin");
    std::fs::create_dir_all(&link_dir)?;

    let mut context = test_context(&prefix, &keg, "17.2");

    execute_statements(
        &[Statement::ChildrenSymlink {
            source_dir: PathExpr::new(PathBase::Bin, &[]),
            link_dir: PathExpr::new(PathBase::HomebrewPrefix, &["bin"]),
            suffix: vec![
                SegmentPart::Literal("-".to_owned()),
                SegmentPart::VersionMajor,
            ],
        }],
        &mut context,
    )?;

    assert!(link_dir.join("psql-17").is_symlink());
    assert!(link_dir.join("pg_dump-17").is_symlink());
    assert_eq!(
        std::fs::read_to_string(link_dir.join("psql-17"))?,
        "#!/bin/sh\n"
    );

    Ok(())
}

#[test]
fn test_if_env_skips_when_unset() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;
    std::fs::create_dir_all(&prefix)?;

    let mut context = test_context(&prefix, &keg, "1.0");

    // Use an env var we know is not set
    execute_statements(
        &[Statement::IfEnv {
            variable: "BREWDOCK_DEFINITELY_NOT_SET_12345".to_owned(),
            negate: false,
            then_branch: vec![Statement::Mkpath(PathExpr::new(
                PathBase::Var,
                &["test-env"],
            ))],
        }],
        &mut context,
    )?;

    // Branch should NOT have executed since the var is not set
    assert!(!prefix.join("var/test-env").exists());
    Ok(())
}

#[test]
fn test_if_env_negate_executes_when_unset() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;
    std::fs::create_dir_all(&prefix)?;

    let mut context = test_context(&prefix, &keg, "1.0");

    // negate=true with unset var → branch SHOULD execute
    execute_statements(
        &[Statement::IfEnv {
            variable: "BREWDOCK_DEFINITELY_NOT_SET_12345".to_owned(),
            negate: true,
            then_branch: vec![Statement::Mkpath(PathExpr::new(
                PathBase::Var,
                &["test-neg"],
            ))],
        }],
        &mut context,
    )?;

    assert!(prefix.join("var/test-neg").is_dir());
    Ok(())
}

#[test]
fn test_process_capture_stores_output() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;
    std::fs::create_dir_all(&prefix)?;

    let mut context = test_context(&prefix, &keg, "1.0");

    execute_statements(
        &[Statement::ProcessCapture {
            variable: "output".to_owned(),
            command: vec![
                Argument::String("echo".to_owned()),
                Argument::String("hello world".to_owned()),
            ],
        }],
        &mut context,
    )?;

    assert_eq!(
        context.captured_outputs.get("output"),
        Some(&"hello world".to_owned())
    );

    Ok(())
}

#[test]
fn test_set_env_applies_to_spawned_command() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().join("prefix");
    let keg = prefix.join("Cellar/demo/1.0");
    std::fs::create_dir_all(&keg)?;
    std::fs::create_dir_all(&prefix)?;

    let mut context = test_context(&prefix, &keg, "1.0");

    execute_statements(
        &[
            Statement::SetEnv {
                variable: "BREWDOCK_DEMO_ENV".to_owned(),
                value: vec![ContentPart::Literal("expected-value".to_owned())],
            },
            Statement::System(vec![
                Argument::String("sh".to_owned()),
                Argument::String("-c".to_owned()),
                Argument::String("[ \"$BREWDOCK_DEMO_ENV\" = \"expected-value\" ]".to_owned()),
            ]),
        ],
        &mut context,
    )?;

    Ok(())
}
