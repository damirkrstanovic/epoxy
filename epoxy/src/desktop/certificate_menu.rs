use crate::certificates::*;
use ksni::{
    MenuItem,
    menu::{StandardItem, SubMenu},
};
fn menu_text(text: &str) -> String {
    display_text(text, 160).replace('_', "__")
}
fn information<T>(text: String) -> MenuItem<T> {
    StandardItem {
        label: menu_text(&text),
        enabled: false,
        ..Default::default()
    }
    .into()
}
pub(super) fn build<T: 'static>(controller: &CertificateController) -> Vec<MenuItem<T>> {
    let snapshot = controller.snapshot();
    let enabled = !matches!(
        snapshot.status,
        CatalogStatus::Busy | CatalogStatus::Loading
    );
    let mut menu = Vec::new();
    if snapshot.status != CatalogStatus::Ready {
        menu.push(information(catalog_status_text(&snapshot).into()));
    }
    if let Some(error) = snapshot.error {
        menu.push(information(error.to_string()));
    }
    if snapshot.selected.is_some()
        && !snapshot
            .contexts
            .iter()
            .flat_map(|c| &c.certificates)
            .any(|c| snapshot.selected.as_ref() == Some(&c.key))
    {
        menu.push(information(
            "Selected certificate unavailable; insert its card or choose another".into(),
        ));
    }
    if snapshot.status == CatalogStatus::Ready
        && snapshot.contexts.iter().all(|c| c.certificates.is_empty())
    {
        menu.push(information(
            "No signing certificates found; check your card".into(),
        ));
    }
    let mut contexts: Vec<_> = snapshot.contexts.iter().collect();
    contexts.sort_by_key(|c| !c.is_signing_context());
    for context in &contexts {
        for cert in &context.certificates {
            let selected = snapshot.selected.as_ref() == Some(&cert.key);
            let key = cert.key.clone();
            let choose = controller.clone();
            let generation = snapshot.generation;
            let eligible = enabled
                && cert.eligibility == Eligibility::Eligible
                && snapshot
                    .contexts
                    .iter()
                    .flat_map(|c| &c.certificates)
                    .filter(|c| c.key == cert.key)
                    .count()
                    == 1;
            let details = vec![
                information(format!("Issued by: {}", cert.issuer)),
                information(format!("Valid until: {}", cert.not_after)),
                information(format!("Valid from: {}", cert.not_before)),
                information(format!("Purpose: {}", cert.purpose)),
            ];
            let mut actions = vec![
                StandardItem {
                    label: if selected {
                        "Used for signing"
                    } else {
                        "Use for signing"
                    }
                    .into(),
                    enabled: eligible && !selected,
                    activate: Box::new(move |_| {
                        let _ = choose.request_select(key.clone(), true, generation);
                    }),
                    ..Default::default()
                }
                .into(),
                SubMenu {
                    label: "Certificate details…".into(),
                    submenu: details,
                    ..Default::default()
                }
                .into(),
            ];
            if cert.eligibility != Eligibility::Eligible {
                actions.insert(0, information(cert.eligibility.label().into()));
            }
            menu.push(
                SubMenu {
                    label: format!(
                        "{}{}",
                        if selected { "✓ " } else { "" },
                        menu_text(&signing_certificate_title(context, cert))
                    ),
                    submenu: actions,
                    ..Default::default()
                }
                .into(),
            );
        }
    }
    let refresh = controller.clone();
    menu.push(MenuItem::Separator);
    menu.push(
        StandardItem {
            label: "Refresh certificates".into(),
            enabled,
            activate: Box::new(move |_| {
                let _ = refresh.request_refresh();
            }),
            ..Default::default()
        }
        .into(),
    );
    if !snapshot.automatic {
        let clear = controller.clone();
        menu.push(
            StandardItem {
                label: "Forget saved certificate choice".into(),
                enabled,
                activate: Box::new(move |_| {
                    let _ = clear.request_clear(true);
                }),
                ..Default::default()
            }
            .into(),
        );
    }
    menu
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (
        CertificateController,
        tokio::sync::mpsc::Receiver<CertificateCommand>,
        CertificateKey,
    ) {
        let (controller, rx) = CertificateController::new();
        let context = ContextKey {
            module_name: "Synthetic_module".into(),
            module_library: None,
            token_uri: b"synthetic".to_vec(),
        };
        let cert = PublicCertificate::from_der(
            context.clone(),
            include_bytes!("../../tests/fixtures/eligible.der"),
        )
        .unwrap();
        let key = cert.key.clone();
        controller.publish_for_selection(vec![PublicContext {
            key: context,
            label: "Signature_PIN context".into(),
            requires_authentication: false,
            certificates: vec![cert],
        }]);
        (controller, rx, key)
    }
    fn action<'a>(menu: &'a [MenuItem<()>], label: &str) -> &'a ksni::menu::StandardItem<()> {
        for item in menu {
            match item {
                MenuItem::Standard(s) if s.label == label => return s,
                MenuItem::SubMenu(s) => {
                    if let Some(a) = find(&s.submenu, label) {
                        return a;
                    }
                }
                _ => (),
            }
        }
        panic!("missing action {label}")
    }
    fn find<'a>(menu: &'a [MenuItem<()>], label: &str) -> Option<&'a ksni::menu::StandardItem<()>> {
        for item in menu {
            match item {
                MenuItem::Standard(s) if s.label == label => return Some(s),
                MenuItem::SubMenu(s) => {
                    if let Some(a) = find(&s.submenu, label) {
                        return Some(a);
                    }
                }
                _ => (),
            }
        }
        None
    }
    #[test]
    fn one_certificate_menu_click_remembers_the_exact_identity() {
        let (controller, mut rx, key) = setup();
        let menu = build::<()>(&controller);
        let card = menu
            .iter()
            .find_map(|i| match i {
                MenuItem::SubMenu(s) if s.label.to_ascii_lowercase().contains("signing") => Some(s),
                _ => None,
            })
            .expect("certificate is directly listed");
        let use_this = card
            .submenu
            .iter()
            .find_map(|i| match i {
                MenuItem::Standard(s) if s.label == "Use for signing" => Some(s),
                _ => None,
            })
            .expect("one use action at certificate level");
        (use_this.activate)(&mut ());
        match rx.try_recv().unwrap() {
            CertificateCommand::Select {
                key: k,
                remember,
                generation,
            } => {
                assert!(k == key);
                assert!(remember);
                assert_eq!(generation, 1);
            }
            _ => panic!("selection expected"),
        }
        controller.finish(Ok(()));
        controller.commit_selection(Some(key.clone()), false);
        let menu = build::<()>(&controller);
        (action(&menu, "Forget saved certificate choice").activate)(&mut ());
        assert!(matches!(
            rx.try_recv(),
            Ok(CertificateCommand::Clear { forget_saved: true })
        ));
        controller
            .clear_choice(
                false,
                &PreferenceStore::new("/synthetic/unused/preference.json".into()),
            )
            .unwrap();
        controller.finish(Ok(()));
        let menu = build::<()>(&controller);
        assert!(find(&menu, "Forget saved certificate choice").is_none());
        assert!(!action(&menu, "Used for signing").enabled);
    }
    #[test]
    fn sole_certificate_is_used_without_manual_selection_or_mode_controls() {
        let (controller, mut rx, key) = setup();
        controller.publish(controller.snapshot().contexts);
        assert!(controller.snapshot().selected == Some(key));
        let menu = build::<()>(&controller);
        assert!(!action(&menu, "Used for signing").enabled);
        assert!(find(&menu, "Use for signing").is_none());
        assert!(find(&menu, "Forget saved certificate choice").is_none());
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn old_menu_actions_cannot_select_after_refresh_or_during_pin_wait() {
        let (controller, mut rx, _) = setup();
        let menu = build::<()>(&controller);
        controller.publish(controller.snapshot().contexts);
        (action(&menu, "Use for signing").activate)(&mut ());
        assert!(rx.try_recv().is_err());
        assert_eq!(controller.snapshot().error, Some(SelectionError::Stale));
        let guard = controller.native_operation();
        let menu = build::<()>(&controller);
        assert!(!action(&menu, "Used for signing").enabled);
        assert!(rx.try_recv().is_err());
        drop(guard);
        let menu = build::<()>(&controller);
        (action(&menu, "Refresh certificates").activate)(&mut ());
        assert!(matches!(rx.try_recv(), Ok(CertificateCommand::Refresh)));
    }
    #[test]
    fn untrusted_menu_labels_cannot_create_access_keys_or_multiline_entries() {
        assert_eq!(
            menu_text("Signature_PIN\ncontext"),
            "Signature__PIN context"
        );
    }
}
