use super::*;
pub(super) fn cert(token: &[u8], hash: u8) -> PublicCertificate {
    PublicCertificate {
        key: CertificateKey {
            context: ContextKey {
                module_name: "synthetic module".into(),
                module_library: Some("/synthetic/module.so".into()),
                token_uri: token.to_vec(),
            },
            der_sha256: [hash; 32],
        },
        holder: "Same synthetic subject".into(),
        issuer: "Synthetic CA".into(),
        not_before: "2020".into(),
        not_after: "2040".into(),
        purpose: "Digital signature, content commitment".into(),
        eligibility: Eligibility::Eligible,
    }
}
#[test]
fn selection_survives_reorder_but_duplicate_identity_is_ambiguous() {
    let a = cert(b"token-a", 1);
    let b = cert(b"token-b", 2);
    let mut model = CatalogModel::from_entries(vec![a.clone(), b.clone()]);
    model.select(&b.key, model.generation()).unwrap();
    model.replace_entries(vec![b.clone(), a]);
    assert!(model.selected_key() == Some(&b.key));
    assert!(model.unique(&b.key).is_ok());
    model.replace_entries(vec![b.clone(), b.clone()]);
    assert_eq!(model.unique(&b.key).err(), Some(SelectionError::Ambiguous));
}
#[test]
fn stale_and_ineligible_actions_cannot_change_selection() {
    let a = cert(b"a", 1);
    let mut b = cert(b"b", 2);
    b.eligibility = Eligibility::Expired;
    let mut model = CatalogModel::from_entries(vec![a.clone(), b.clone()]);
    let old = model.generation();
    model.replace_entries(vec![a.clone(), b.clone()]);
    assert_eq!(model.select(&a.key, old), Err(SelectionError::Stale));
    assert_eq!(
        model.select(&b.key, model.generation()),
        Err(SelectionError::Ineligible)
    );
    assert!(model.selected_key().is_none());
    b.eligibility = Eligibility::Unsupported;
    model.replace_entries(vec![a, b.clone()]);
    assert_eq!(
        model.select(&b.key, model.generation()),
        Err(SelectionError::Ineligible)
    );
}
#[test]
fn bounded_refresh_coalesces_and_busy_selection_is_rejected() {
    let (controller, mut rx) = CertificateController::new();
    controller.request_refresh().unwrap();
    controller.request_refresh().unwrap();
    assert!(matches!(rx.try_recv(), Ok(CertificateCommand::Refresh)));
    assert!(rx.try_recv().is_err());
    controller.publish(vec![PublicContext {
        key: cert(b"a", 1).key.context.clone(),
        label: "Synthetic PIN context".into(),
        requires_authentication: false,
        certificates: vec![cert(b"a", 1)],
    }]);
    let guard = controller.native_operation();
    assert_eq!(
        controller.request_select(cert(b"a", 1).key, false, controller.snapshot().generation),
        Err(SelectionError::Busy)
    );
    assert_eq!(controller.snapshot().status, CatalogStatus::Busy);
    drop(guard);
    assert_eq!(controller.snapshot().status, CatalogStatus::Ready);
    controller.close();
    assert_eq!(
        controller.request_refresh(),
        Err(SelectionError::Unavailable)
    );
}
#[test]
fn der_metadata_preserves_unicode_and_rejects_ineligible_or_malformed_data() {
    let context = cert(b"a", 1).key.context;
    for (der, eligible) in [
        (
            include_bytes!("../../tests/fixtures/eligible.der").as_slice(),
            Eligibility::Eligible,
        ),
        (
            include_bytes!("../../tests/fixtures/expired.der").as_slice(),
            Eligibility::Expired,
        ),
        (
            include_bytes!("../../tests/fixtures/unsupported.der").as_slice(),
            Eligibility::Unsupported,
        ),
    ] {
        let c = PublicCertificate::from_der(context.clone(), der).unwrap();
        assert_eq!(c.holder, "Synthetic Željko");
        assert_eq!(c.issuer, "CN=Synthetic CA");
        assert_eq!(c.eligibility, eligible);
    }
    assert_eq!(
        PublicCertificate::from_der(context.clone(), b"not DER").err(),
        Some(SelectionError::InvalidCertificate)
    );
    let mut trailing = include_bytes!("../../tests/fixtures/eligible.der").to_vec();
    trailing.push(0);
    assert!(PublicCertificate::from_der(context, &trailing).is_err());
}
#[test]
fn stale_alias_or_other_module_context_fingerprint_never_authorizes_signing() {
    let a = cert(b"a", 1);
    let b = cert(b"b", 1);
    let (controller, _) = CertificateController::new();
    controller.commit_selection(Some(a.key.clone()), false);
    let revision = controller.snapshot().revision;
    let mut sign_count = 0;
    let mut attempt = |snapshot: &CertificateSnapshot, key: &CertificateKey, rev: u64| {
        if snapshot.authorize_alias(key, rev).is_ok() {
            sign_count += 1;
        }
    };
    attempt(&controller.snapshot(), &b.key, revision);
    let mut other_module = a.key.clone();
    other_module.context.module_library = Some("/other.so".into());
    attempt(&controller.snapshot(), &other_module, revision);
    let mut other_hash = a.key.clone();
    other_hash.der_sha256 = [9; 32];
    attempt(&controller.snapshot(), &other_hash, revision);
    controller.commit_selection(Some(b.key.clone()), false);
    attempt(&controller.snapshot(), &a.key, revision);
    controller.commit_selection(None, false);
    attempt(&controller.snapshot(), &a.key, revision);
    assert_eq!(sign_count, 0);
}
#[test]
fn missing_remembered_selection_never_falls_back_to_an_available_card() {
    let a = cert(b"missing", 1);
    let b = cert(b"available", 2);
    let (controller, _) = CertificateController::new();
    controller.commit_selection(Some(a.key.clone()), true);
    controller.publish(vec![PublicContext {
        key: b.key.context.clone(),
        label: "Other card".into(),
        requires_authentication: false,
        certificates: vec![b.clone()],
    }]);
    assert_eq!(
        controller
            .snapshot()
            .authorize_alias(&b.key, controller.snapshot().revision),
        Err(SelectionError::Missing)
    );
    assert!(controller.snapshot().selected == Some(a.key));
}
#[test]
fn replacement_during_authentication_never_reuses_the_old_context_even_with_equal_der() {
    let a = cert(b"card-a", 1);
    let b = cert(b"card-b", 1);
    let original = TokenIdentity {
        context: a.key.context.clone(),
        series: 10,
    };
    let replaced = TokenIdentity {
        context: b.key.context.clone(),
        series: 11,
    };
    let same_uri_new_incarnation = TokenIdentity {
        context: a.key.context.clone(),
        series: 12,
    };
    let mut signed = 0;
    for current in [&replaced, &same_uri_new_incarnation] {
        if original.ensure_unchanged(current).is_ok() {
            signed += 1;
        }
    }
    assert_eq!(
        signed, 0,
        "the same slot pointer/certificate must not authorize a replacement card"
    );
    assert!(
        original
            .ensure_unchanged(&TokenIdentity {
                context: a.key.context,
                series: 10
            })
            .is_ok()
    );
}
#[test]
fn cached_uri_must_match_fresh_token_information_bytes() {
    let uri = b"pkcs11:model=Model;manufacturer=Synthetic;serial=001;token=Signature%20PIN";
    assert!(uri_matches_info(
        uri,
        [b"Signature PIN ", b"Synthetic ", b"001 ", b"Model "]
    ));
    assert!(!uri_matches_info(
        uri,
        [b"Signature PIN ", b"Synthetic ", b"002 ", b"Model "]
    ));
    assert!(!uri_matches_info(
        b"pkcs11:token=Signature%2",
        [b"Signature PIN", b"", b"", b""]
    ));
    assert!(!uri_matches_info(
        b"pkcs11:token=Signature%20PIN;token=Signature%20PIN",
        [b"Signature PIN", b"", b"", b""]
    ));
}
