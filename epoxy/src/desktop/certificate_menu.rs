use crate::certificates::*;
use ksni::{
    MenuItem,
    menu::{CheckmarkItem, StandardItem, SubMenu},
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
    let refresh = controller.clone();
    let clear = controller.clone();
    let mut menu = vec![
        information(catalog_status_text(&snapshot).into()),
        StandardItem {
            label: "Refresh".into(),
            enabled: !matches!(
                snapshot.status,
                CatalogStatus::Busy | CatalogStatus::Loading
            ),
            activate: Box::new(move |_| {
                let _ = refresh.request_refresh();
            }),
            ..Default::default()
        }
        .into(),
        StandardItem {
            label: "Use portal selection and forget saved choice".into(),
            enabled,
            activate: Box::new(move |_| {
                let _ = clear.request_clear(true);
            }),
            ..Default::default()
        }
        .into(),
    ];
    if let Some(error) = snapshot.error {
        menu.push(information(error.to_string()));
    }
    menu.push(information(selection_status_text(&snapshot).into()));
    if snapshot.selected.is_some() {
        menu.push(information(
            "After changing selection, reload/reselect certificates in the portal".into(),
        ));
    }
    for context in &snapshot.contexts {
        let mut children = vec![information(format!("Module: {}", context.key.module_name))];
        if context.requires_authentication {
            children.push(information(
                "Certificates require authentication; choose this context in the portal".into(),
            ));
        } else if context.certificates.is_empty() {
            children.push(information(
                "No public signing certificates in this PIN context".into(),
            ));
        }
        for cert in &context.certificates {
            let selected = snapshot.selected.as_ref() == Some(&cert.key);
            let select = controller.clone();
            let remember = controller.clone();
            let key = cert.key.clone();
            let remembered_key = key.clone();
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
                information(format!("Holder: {}", cert.holder)),
                information(format!("Issuer: {}", cert.issuer)),
                information(format!("Valid from: {}", cert.not_before)),
                information(format!("Valid until: {}", cert.not_after)),
                information(format!("Purpose: {}", cert.purpose)),
                information(cert.eligibility.label().into()),
                CheckmarkItem {
                    label: "Selected for signing".into(),
                    checked: selected,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Use for this session".into(),
                    enabled: eligible,
                    activate: Box::new(move |_| {
                        let _ = select.request_select(key.clone(), false, generation);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Use and remember".into(),
                    enabled: eligible,
                    activate: Box::new(move |_| {
                        let _ = remember.request_select(remembered_key.clone(), true, generation);
                    }),
                    ..Default::default()
                }
                .into(),
            ];
            children.push(
                SubMenu {
                    label: format!(
                        "{}{}",
                        if selected { "✓ " } else { "" },
                        menu_text(&cert.holder)
                    ),
                    submenu: details,
                    ..Default::default()
                }
                .into(),
            );
        }
        menu.push(
            SubMenu {
                label: menu_text(&context.label),
                submenu: children,
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
        controller.publish(vec![PublicContext {
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
    fn menu_actions_dispatch_exact_context_and_remember_intent() {
        let (controller, mut rx, key) = setup();
        let menu = build::<()>(&controller);
        (action(&menu, "Use for this session").activate)(&mut ());
        match rx.try_recv().unwrap() {
            CertificateCommand::Select {
                key: k,
                remember,
                generation,
            } => {
                assert!(k == key);
                assert!(!remember);
                assert_eq!(generation, 1);
            }
            _ => panic!("selection expected"),
        }
        controller.finish(Ok(()));
        let menu = build::<()>(&controller);
        (action(&menu, "Use and remember").activate)(&mut ());
        assert!(matches!(
            rx.try_recv(),
            Ok(CertificateCommand::Select { remember: true, .. })
        ));
        controller.finish(Ok(()));
        let menu = build::<()>(&controller);
        (action(&menu, "Use portal selection and forget saved choice").activate)(&mut ());
        assert!(matches!(
            rx.try_recv(),
            Ok(CertificateCommand::Clear { forget_saved: true })
        ));
    }
    #[test]
    fn old_menu_actions_cannot_select_after_refresh_or_during_pin_wait() {
        let (controller, mut rx, _) = setup();
        let menu = build::<()>(&controller);
        controller.publish(controller.snapshot().contexts);
        (action(&menu, "Use for this session").activate)(&mut ());
        assert!(rx.try_recv().is_err());
        assert_eq!(controller.snapshot().error, Some(SelectionError::Stale));
        let guard = controller.native_operation();
        let menu = build::<()>(&controller);
        assert!(!action(&menu, "Use for this session").enabled);
        (action(&menu, "Use for this session").activate)(&mut ());
        assert!(rx.try_recv().is_err());
        drop(guard);
        let menu = build::<()>(&controller);
        (action(&menu, "Refresh").activate)(&mut ());
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
