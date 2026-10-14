use super::*;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt, symlink},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "epoxy-pref-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn key() -> CertificateKey {
    CertificateKey {
        context: ContextKey {
            module_name: "Synthetic".into(),
            module_library: None,
            token_uri: b"pkcs11:token=synthetic".to_vec(),
        },
        der_sha256: [3; 32],
    }
}
#[test]
fn explicit_remember_round_trips_and_no_selection_creates_no_directory() {
    let fixture = Fixture::new();
    let path = fixture.0.join("app/selection.json");
    let store = PreferenceStore::new(path.clone());
    assert!(store.load().unwrap().is_none());
    assert!(!path.parent().unwrap().exists());
    store.save(&key()).unwrap();
    assert!(store.load().unwrap() == Some(key()));
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    store.forget().unwrap();
    assert!(store.load().unwrap().is_none());
}
#[test]
fn malformed_oversized_unknown_version_or_unrelated_file_is_never_overwritten() {
    let fixture = Fixture::new();
    let path = fixture.0.join("selection.json");
    let store = PreferenceStore::new(path.clone());
    for contents in [
        b"unrelated file".to_vec(),
        vec![b'x'; 8193],
        br#"{"version":99,"kind":"epoxy-certificate-selection","key":null}"#.to_vec(),
    ] {
        fs::write(&path, &contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(store.load().err(), Some(SelectionError::Preference));
        assert_eq!(store.save(&key()), Err(SelectionError::Preference));
        assert_eq!(store.forget(), Err(SelectionError::Preference));
        assert_eq!(fs::read(&path).unwrap(), contents);
    }
}
#[test]
fn symlink_files_and_directories_are_rejected_without_modifying_targets() {
    let fixture = Fixture::new();
    let target = fixture.0.join("target");
    fs::write(&target, b"unchanged").unwrap();
    let link = fixture.0.join("selection.json");
    symlink(&target, &link).unwrap();
    let store = PreferenceStore::new(link);
    assert!(store.load().is_err());
    assert!(store.save(&key()).is_err());
    assert!(store.forget().is_err());
    symlink(&fixture.0, fixture.0.join("alias")).unwrap();
    let alias = PreferenceStore::new(fixture.0.join("alias/other.json"));
    assert!(alias.save(&key()).is_err());
    assert_eq!(fs::read(target).unwrap(), b"unchanged");
}
#[test]
fn failed_save_keeps_the_old_choice_and_session_only_selection_does_not_write() {
    let fixture = Fixture::new();
    let path = fixture.0.join("selection.json");
    let store = PreferenceStore::new(path.clone());
    let (controller, _) = CertificateController::new();
    let mut a = super::tests::cert(b"a", 1);
    let b = super::tests::cert(b"b", 2);
    a.key.context.module_name = "synthetic".into();
    controller.publish(vec![PublicContext {
        key: a.key.context.clone(),
        label: "Synthetic".into(),
        requires_authentication: false,
        certificates: vec![a.clone(), b.clone()],
    }]);
    controller
        .commit_choice(a.key.clone(), false, &store)
        .unwrap();
    assert!(!path.exists());
    let revision = controller.snapshot().revision;
    fs::write(&path, b"unrelated").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        controller.commit_choice(b.key, true, &store),
        Err(SelectionError::Preference)
    );
    assert!(controller.snapshot().selected == Some(a.key));
    assert_eq!(controller.snapshot().revision, revision);
}
