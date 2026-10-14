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
                    label: if selected && snapshot.remembered {
                        "Using this certificate"
                    } else {
                        "Use this certificate"
                    }
                    .into(),
                    enabled: eligible,
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
    let clear = controller.clone();
    let mut advanced = vec![
        StandardItem {
            label: "Choose automatically".into(),
            enabled,
            activate: Box::new(move |_| {
                let _ = clear.request_clear(true);
            }),
            ..Default::default()
        }
        .into(),
    ];
    for context in contexts {
        let mut technical = vec![information(format!("Module: {}", context.key.module_name))];
        if context.requires_authentication {
            technical.push(information("Certificates require a PIN to view".into()));
        } else if context.certificates.is_empty() {
            technical.push(information("No signing certificates".into()));
        }
        advanced.push(
            SubMenu {
                label: menu_text(&context.label),
                submenu: technical,
                ..Default::default()
            }
            .into(),
        );
    }
    menu.push(
        SubMenu {
            label: "Advanced".into(),
            submenu: advanced,
            ..Default::default()
        }
        .into(),
    );
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
    fn one_certificate_menu_click_remembers_the_exact_identity() {
        let (controller, mut rx, key) = setup();
        let menu = build::<()>(&controller);
        let card = menu
            .iter()
            .find_map(|i| match i {
                MenuItem::SubMenu(s) if s.label.contains("signing") => Some(s),
                _ => None,
            })
            .expect("certificate is directly listed");
        let use_this = card
            .submenu
            .iter()
            .find_map(|i| match i {
                MenuItem::Standard(s) if s.label == "Use this certificate" => Some(s),
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
        (action(&menu, "Choose automatically").activate)(&mut ());
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
        (action(&menu, "Use this certificate").activate)(&mut ());
        assert!(rx.try_recv().is_err());
        assert_eq!(controller.snapshot().error, Some(SelectionError::Stale));
        let guard = controller.native_operation();
        let menu = build::<()>(&controller);
        assert!(!action(&menu, "Use this certificate").enabled);
        (action(&menu, "Use this certificate").activate)(&mut ());
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
