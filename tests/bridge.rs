use argos::bridge::{BridgeError, BridgeErrorKind, ScopedPath};
use argos::error::{ArgosError, ValidationKind};
use std::path::Path;
use tempfile::tempdir;

#[test]
fn argos_io_error_maps_to_bridge_io_kind() {
    let argos = ArgosError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"));
    let bridge: BridgeError = argos.into();
    assert!(matches!(bridge.kind, BridgeErrorKind::Io));
}

#[test]
fn argos_validation_error_maps_to_bridge_validation_kind() {
    let argos = ArgosError::Validation {
        kind: ValidationKind::MissingSoi,
    };
    let bridge: BridgeError = argos.into();
    assert!(matches!(bridge.kind, BridgeErrorKind::Validation));
}

#[test]
fn argos_unsupported_maps_to_bridge_unsupported() {
    let argos = ArgosError::Unsupported;
    let bridge: BridgeError = argos.into();
    assert!(matches!(bridge.kind, BridgeErrorKind::Unsupported));
}

#[test]
fn argos_allocation_carries_details() {
    let argos = ArgosError::Allocation {
        size: 4096,
        align: 4096,
    };
    let bridge: BridgeError = argos.into();
    assert!(matches!(bridge.kind, BridgeErrorKind::Allocation));
    assert!(bridge.detail.contains("4096"));
}

#[test]
fn scoped_path_accepts_path_inside_allowed_prefix() {
    let scope = tempdir().expect("tempdir");
    let inner = scope.path().join("artifact_dir");
    std::fs::create_dir_all(&inner).expect("mkdir");
    let allowed: &[&Path] = &[scope.path()];
    let sp = ScopedPath::new(inner.to_str().unwrap(), allowed).expect("inside scope");
    assert!(sp.as_path().starts_with(scope.path()));
}

#[test]
fn scoped_path_rejects_path_outside_all_allowed_prefixes() {
    let scope = tempdir().expect("tempdir");
    let allowed: &[&Path] = &[scope.path()];
    let err = ScopedPath::new("/etc/passwd", allowed).expect_err("must be denied");
    assert!(matches!(err.kind, BridgeErrorKind::Denied));
}

#[test]
fn scoped_path_rejects_nonexistent_path() {
    let scope = tempdir().expect("tempdir");
    let missing = scope.path().join("does_not_exist_anywhere");
    let allowed: &[&Path] = &[scope.path()];
    let err =
        ScopedPath::new(missing.to_str().unwrap(), allowed).expect_err("must fail to canonicalize");
    assert!(matches!(
        err.kind,
        BridgeErrorKind::Io | BridgeErrorKind::Denied
    ));
}

#[test]
fn scoped_path_canonicalises_traversal_attempts_into_real_paths() {
    let scope = tempdir().expect("tempdir");
    let sub = scope.path().join("sub");
    std::fs::create_dir_all(&sub).expect("mkdir");
    let traversed = sub.join("..");
    let allowed: &[&Path] = &[scope.path()];
    let sp =
        ScopedPath::new(traversed.to_str().unwrap(), allowed).expect("traversal stays in scope");
    assert_eq!(sp.as_path(), scope.path().canonicalize().unwrap());
}

#[test]
fn scoped_path_resolves_symlinks_to_target_for_scope_check() {
    let scope = tempdir().expect("tempdir");
    let outside_root = tempdir().expect("tempdir outside");
    let outside_dir = outside_root.path().join("outside_target");
    std::fs::create_dir_all(&outside_dir).expect("mkdir outside");

    let link = scope.path().join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_dir, &link).expect("symlink");
    #[cfg(not(unix))]
    return;

    let allowed: &[&Path] = &[scope.path()];
    let err = ScopedPath::new(link.to_str().unwrap(), allowed)
        .expect_err("symlink target outside scope must be denied");
    assert!(matches!(err.kind, BridgeErrorKind::Denied));
}
