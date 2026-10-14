use super::{PinReply, prompt::PendingReply};
use crate::nss::PinInfo;
use gtk4::{Application, Box, Button, Label, Orientation, PasswordEntry, Window, glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub(super) struct PinDialog {
    pub window: Window,
    pub entry: PasswordEntry,
    reply: Rc<RefCell<PendingReply>>,
}
impl PinDialog {
    pub fn cancel(&self) {
        self.finish(false);
    }
    fn finish(&self, submit: bool) {
        if submit {
            self.reply.borrow_mut().submit(self.entry.text().into());
        } else {
            self.reply.borrow_mut().cancel();
        }
        self.entry.set_text("");
        self.window.destroy();
    }
}
pub(super) fn new_pin_popup(
    app: &Application,
    info: PinInfo,
    token: String,
    reply: PinReply,
) -> PinDialog {
    let content = Box::new(Orientation::Vertical, 12);
    for side in ["start", "end", "top", "bottom"] {
        match side {
            "start" => content.set_margin_start(18),
            "end" => content.set_margin_end(18),
            "top" => content.set_margin_top(18),
            _ => content.set_margin_bottom(18),
        }
    }
    for text in [
        Some(info.reason),
        info.cert,
        Some(format!("Enter PIN for {token}")),
    ]
    .into_iter()
    .flatten()
    {
        let label = Label::new(Some(&text));
        label.set_wrap(true);
        label.set_xalign(0.0);
        content.append(&label);
    }
    let entry = PasswordEntry::builder()
        .show_peek_icon(false)
        .hexpand(true)
        .build();
    entry.update_property(&[gtk4::accessible::Property::Label("Card PIN")]);
    content.append(&entry);
    let buttons = Box::new(Orientation::Horizontal, 12);
    buttons.set_halign(gtk4::Align::End);
    let cancel = Button::with_label("Cancel");
    let submit = Button::with_label("Continue");
    submit.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&submit);
    content.append(&buttons);
    let window = Window::builder()
        .application(app)
        .title("Epoxy — Card PIN")
        .default_width(420)
        .resizable(true)
        .child(&content)
        .build();
    if let Some(parent) = app.active_window().filter(|parent| parent.is_visible()) {
        window.set_transient_for(Some(&parent));
    }
    let dialog = PinDialog {
        window: window.clone(),
        entry: entry.clone(),
        reply: Rc::new(RefCell::new(PendingReply::new(reply))),
    };
    // Weak widget references avoid cycles; the reply is shared and resolves exactly once.
    let finish = Rc::new({
        let window = window.downgrade();
        let entry = entry.downgrade();
        let reply = dialog.reply.clone();
        move |submit: bool| {
            if let Some(entry) = entry.upgrade() {
                if submit {
                    reply.borrow_mut().submit(entry.text().into());
                } else {
                    reply.borrow_mut().cancel();
                }
                entry.set_text("");
            } else {
                reply.borrow_mut().cancel();
            }
            if let Some(window) = window.upgrade() {
                window.destroy();
            }
        }
    });
    entry.connect_activate({
        let finish = finish.clone();
        move |_| finish(true)
    });
    submit.connect_clicked({
        let finish = finish.clone();
        move |_| finish(true)
    });
    cancel.connect_clicked({
        let finish = finish.clone();
        move |_| finish(false)
    });
    window.connect_close_request({
        let finish = finish.clone();
        move |_| {
            finish(false);
            glib::Propagation::Stop
        }
    });
    let keys = gtk4::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk4::gdk::Key::Escape {
            finish(false);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    window.present();
    entry.grab_focus();
    dialog
}
