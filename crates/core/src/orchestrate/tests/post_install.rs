use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicUsize},
};

use super::{BrewdockError, Layout};
use crate::testutil::{
    BottleArchiveEntry, assert_installed, assert_not_installed, create_bottle_tar_gz,
    create_bottle_tar_gz_with_entries, make_formula, make_orchestrator,
    make_orchestrator_with_sources,
};

#[tokio::test]
async fn test_install_runs_post_install_before_link() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let mut formula = make_formula("demo", "1.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz(
        "demo",
        "1.0",
        &[
            (
                "bin/write-flag",
                b"#!/bin/sh\nprintf '%s' \"$1\" > \"$2\"\n",
            ),
            ("share/src.txt", b"payload"),
        ],
    )?;

    let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    cp share/"src.txt", var/"demo/copied.txt"
    system "/bin/sh", bin/"write-flag", "done", var/"demo/result.txt"
  end
end
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/demo.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["demo"]).await?;

    assert_eq!(installed, vec!["demo"]);
    assert_eq!(
        std::fs::read_to_string(layout.prefix().join("var/demo/copied.txt"))?,
        "payload"
    );
    assert_eq!(
        std::fs::read_to_string(layout.prefix().join("var/demo/result.txt"))?,
        "done"
    );
    assert!(layout.prefix().join("bin/write-flag").is_symlink());
    assert!(
        layout
            .cellar()
            .join("demo/1.0/INSTALL_RECEIPT.json")
            .exists()
    );
    assert_installed(&layout, "demo");
    Ok(())
}

#[tokio::test]
async fn test_install_post_install_creates_link_target() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
    let mut formula = make_formula("shared-mime-info", "2.4", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz(
        "shared-mime-info",
        "2.4",
        &[
            (
                "bin/update-mime-database",
                b"#!/bin/sh\nmkdir -p \"$1\"\ntouch \"$1/mime.cache\"\n",
            ),
            (
                "share/shared-mime-info/packages/freedesktop.org.xml",
                b"<mime-info/>",
            ),
        ],
    )?;

    let source = r#"
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
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/shared-mime-info.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["shared-mime-info"]).await?;
    let mime_dir = layout.prefix().join("share/mime");

    assert_eq!(installed, vec!["shared-mime-info"]);
    assert_eq!(
        std::fs::read_to_string(mime_dir.join("packages/freedesktop.org.xml"))?,
        "<mime-info/>"
    );
    assert!(mime_dir.join("mime.cache").exists());
    assert!(
        layout
            .prefix()
            .join("bin/update-mime-database")
            .is_symlink()
    );
    assert!(
        layout
            .cellar()
            .join("shared-mime-info/2.4/share/mime")
            .is_symlink()
    );
    assert_installed(&layout, "shared-mime-info");
    Ok(())
}

#[tokio::test]
async fn test_install_cleans_up_failed_post_install() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let mut formula = make_formula("demo", "1.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz("demo", "1.0", &[("share/src.txt", b"payload")])?;
    let source = r#"
class Demo < Formula
  def post_install
    unsupported_call "boom"
  end
end
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/demo.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let result = orchestrator.install(&["demo"]).await;

    assert!(matches!(
        result,
        Err(BrewdockError::Cellar(
            brewdock_cellar::CellarError::Analysis(_)
        ))
    ));
    assert!(!layout.cellar().join("demo/1.0").exists());
    assert!(layout.opt_dir().join("demo").symlink_metadata().is_err());
    assert!(
        !layout
            .cellar()
            .join("demo/1.0/INSTALL_RECEIPT.json")
            .exists()
    );
    assert_not_installed(&layout, "demo");
    assert!(!layout.prefix().join("bin").exists());
    Ok(())
}

#[tokio::test]
async fn test_install_runs_bottle_prefix_install_before_post_install()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "ababababcdcdcdcdababababcdcdcdcdababababcdcdcdcdababababcdcdcdcd";
    let mut formula = make_formula("kafka", "4.2.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz_with_entries(
        "kafka",
        "4.2.0",
        &[
            BottleArchiveEntry::File(".bottle/etc/kafka/server.properties", b"logs=1\n"),
            BottleArchiveEntry::Symlink("libexec/config", "../../../../etc/kafka"),
            BottleArchiveEntry::File("bin/check-config", b"#!/bin/sh\ncat \"$1\" > \"$2\"\n"),
        ],
    )?;

    let source = r#"
class Kafka < Formula
  def post_install
    system "/bin/sh", bin/"check-config", etc/"kafka/server.properties", var/"copied.txt"
  end
end
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/kafka.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    orchestrator.install(&["kafka"]).await?;

    assert_eq!(
        std::fs::read_to_string(layout.prefix().join("var/copied.txt"))?,
        "logs=1\n"
    );
    assert_eq!(
        std::fs::read_link(layout.cellar().join("kafka/4.2.0/libexec/config"))?,
        std::path::PathBuf::from("../../../../etc/kafka")
    );
    Ok(())
}

#[tokio::test]
async fn test_install_rolls_back_bottle_prefix_entries_on_post_install_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "efefefefcdcdcdcdababababcdcdcdcdababababcdcdcdcdababababcdcdcdcd";
    let mut formula = make_formula("demo", "1.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz_with_entries(
        "demo",
        "1.0",
        &[BottleArchiveEntry::File(
            ".bottle/etc/demo/config.toml",
            b"demo=true\n",
        )],
    )?;
    let source = r#"
class Demo < Formula
  def post_install
    unsupported_call "boom"
  end
end
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/demo.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let result = orchestrator.install(&["demo"]).await;

    assert!(result.is_err());
    assert!(!layout.prefix().join("etc/demo/config.toml").exists());
    assert!(!layout.prefix().join("etc/demo").exists());
    Ok(())
}

#[tokio::test]
async fn test_install_cleans_up_on_ruby_source_fail() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "1212121212121212121212121212121212121212121212121212121212121212";
    let mut formula = make_formula("demo", "1.0", &[], sha);
    formula.post_install_defined = true;

    let orchestrator = make_orchestrator(
        vec![formula],
        vec![(
            sha,
            create_bottle_tar_gz("demo", "1.0", &[("bin/demo", b"binary")])?,
        )],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let result = orchestrator.install(&["demo"]).await;

    assert!(matches!(
        result,
        Err(BrewdockError::Formula(
            brewdock_formula::FormulaError::NotFound { .. }
        ))
    ));
    assert!(!layout.cellar().join("demo/1.0").exists());
    assert!(layout.opt_dir().join("demo").symlink_metadata().is_err());
    Ok(())
}

#[tokio::test]
async fn test_install_bootstraps_certificate_bundle() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let mut formula = make_formula("ca-certificates", "1.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz(
        "ca-certificates",
        "1.0",
        &[("share/ca-certificates/cacert.pem", b"mozilla-bundle")],
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

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/ca-certificates.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["ca-certificates"]).await?;

    assert_eq!(installed, vec!["ca-certificates"]);
    assert_eq!(
        std::fs::read_to_string(layout.prefix().join("etc/ca-certificates/cert.pem"))?,
        "mozilla-bundle"
    );
    assert_installed(&layout, "ca-certificates");
    Ok(())
}

#[tokio::test]
async fn test_install_bootstraps_openssl_cert_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "abababababababababababababababababababababababababababababababab";
    let mut formula = make_formula("openssl@3", "1.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz(
        "openssl@3",
        "1.0",
        &[
            ("bin/openssl", b"binary"),
            ("share/ca-certificates/cert.pem", b"bundle"),
        ],
    )?;

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

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/openssl@3.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["openssl@3"]).await?;

    assert_eq!(installed, vec!["openssl@3"]);
    let cert_link = layout.prefix().join("etc/openssl@3/cert.pem");
    assert!(cert_link.is_symlink());
    assert!(layout.prefix().join("bin/openssl").is_symlink());
    assert_installed(&layout, "openssl@3");
    Ok(())
}

#[tokio::test]
async fn test_install_rolls_back_post_install_state() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "abababababababababababababababababababababababababababababababab";
    let mut formula = make_formula("demo", "1.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz("demo", "1.0", &[("share/src.txt", b"payload")])?;

    let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    cp share/"src.txt", var/"demo/copied.txt"
    system "/bin/sh", "-c", "exit 1"
  end
end
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/demo.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let result = orchestrator.install(&["demo"]).await;

    assert!(matches!(
        result,
        Err(BrewdockError::Cellar(
            brewdock_cellar::CellarError::PostInstallCommandFailed { .. }
        ))
    ));
    assert!(!layout.prefix().join("var/demo").exists());
    assert!(!layout.cellar().join("demo/1.0").exists());
    Ok(())
}

#[tokio::test]
async fn test_tier2_fallback_uses_name_attribute() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1";
    let mut formula = make_formula("myapp", "2.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz("myapp", "2.0", &[("bin/myapp", b"#!/bin/sh\necho myapp\n")])?;

    let source = r"
class Myapp < Formula
  def post_install
    (etc/name).mkpath
  end
end
";

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/myapp.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["myapp"]).await?;

    assert_eq!(installed, vec!["myapp"]);
    assert!(layout.prefix().join("etc/myapp").is_dir());
    assert!(
        layout
            .cellar()
            .join("myapp/2.0/INSTALL_RECEIPT.json")
            .exists()
    );
    Ok(())
}

#[tokio::test]
async fn test_tier2_fallback_uses_version_major_minor() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2f2";
    let mut formula = make_formula("cafeobj", "1.6.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz(
        "cafeobj",
        "1.6.0",
        &[("bin/cafeobj", b"#!/bin/sh\necho cafeobj\n")],
    )?;

    let source = r#"
class Cafeobj < Formula
  def post_install
    mkdir_p lib/"cafeobj-#{version.major_minor}/sbcl"
  end
end
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/cafeobj.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["cafeobj"]).await?;

    assert_eq!(installed, vec!["cafeobj"]);
    assert!(
        layout
            .cellar()
            .join("cafeobj/1.6.0/lib/cafeobj-1.6/sbcl")
            .is_dir()
    );
    Ok(())
}

#[tokio::test]
async fn test_tier2_filesystem_postgresql() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1";
    let mut formula = make_formula("postgresql@17", "17.2", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz(
        "postgresql@17",
        "17.2",
        &[
            ("bin/psql", b"#!/bin/sh\necho psql\n"),
            ("bin/pg_dump", b"#!/bin/sh\necho pg_dump\n"),
            ("bin/initdb", b"#!/bin/sh\nexit 0\n"),
            ("include/postgresql/libpq-fe.h", b"/* header */"),
            (
                "include/postgresql/server/pg_config.h",
                b"/* server header */",
            ),
            ("lib/postgresql/libpq.dylib", b"dylib-stub"),
        ],
    )?;

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

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/postgresql@17.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["postgresql@17"]).await?;
    assert_eq!(installed, vec!["postgresql@17"]);
    assert!(
        layout
            .prefix()
            .join("include/postgresql@17/libpq-fe.h")
            .is_symlink()
    );
    assert!(
        layout
            .prefix()
            .join("include/postgresql@17/server/pg_config.h")
            .is_symlink()
    );
    assert!(layout.prefix().join("bin/psql-17").is_symlink());
    assert!(layout.prefix().join("bin/pg_dump-17").is_symlink());
    assert!(layout.prefix().join("var/postgresql@17").is_dir());
    Ok(())
}

#[tokio::test]
async fn test_tier2_process_capture() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1";
    let mut formula = make_formula("myapp", "2.0", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz("myapp", "2.0", &[("bin/myapp", b"#!/bin/sh\necho hello\n")])?;

    let source = r#"
class Myapp < Formula
  def post_install
    output = Utils.safe_popen_read("echo", "captured-value")
  end
end
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/myapp.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["myapp"]).await?;
    assert_eq!(installed, vec!["myapp"]);
    assert!(
        layout
            .cellar()
            .join("myapp/2.0/INSTALL_RECEIPT.json")
            .exists()
    );
    Ok(())
}

#[tokio::test]
async fn test_post_install_prelinks_non_keg_only() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "abababababababababababababababababababababababababababababababab";
    let mut formula = make_formula("postgresql@14", "14.22", &[], sha);
    formula.post_install_defined = true;

    let tar = create_bottle_tar_gz(
        "postgresql@14",
        "14.22",
        &[
            (
                "bin/initdb",
                b"#!/bin/sh\nprefix=$(cd \"$(dirname \"$0\")/../../../..\" && pwd)\n[ -f \"$prefix/lib/postgresql@14/libpq.5.dylib\" ]\n",
            ),
            ("lib/postgresql@14/libpq.5.dylib", b"dylib-stub"),
        ],
    )?;

    let source = r#"
class PostgresqlAT14 < Formula
  def postgresql_datadir
    var/name
  end

  def pg_version_exists?
    (postgresql_datadir/"PG_VERSION").exist?
  end

  def post_install
    postgresql_datadir.mkpath
    return if ENV["HOMEBREW_GITHUB_ACTIONS"]
    system bin/"initdb", "--locale=en_US.UTF-8", "-E", "UTF-8", postgresql_datadir unless pg_version_exists?
  end
end
"#;

    let orchestrator = make_orchestrator_with_sources(
        vec![formula],
        HashMap::from([("Formula/postgresql@14.rb".to_owned(), source.to_owned())]),
        vec![(sha, tar)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["postgresql@14"]).await?;
    assert_eq!(installed, vec!["postgresql@14"]);
    assert!(
        layout
            .prefix()
            .join("lib/postgresql@14/libpq.5.dylib")
            .is_symlink()
    );
    Ok(())
}
