use super::{PinReply, prompt::PendingReply};
use crate::{nss::PinInfo, pin::SecretPin};
use gtk4::glib::translate::ToGlibPtr;
use gtk4::{
    Application, Box, Button, Expander, Label, Orientation, PasswordEntry, Window, glib, prelude::*,
};
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
        if let Some(pin) = take_pin(&self.entry, submit) {
            self.reply.borrow_mut().submit(pin);
        } else {
            self.reply.borrow_mut().cancel();
        }
        self.window.destroy();
    }
}
// Copy directly from GTK's borrowed text into our zeroizing owner. Calling
// Editable::text() would create an additional unprotected GString allocation.
fn take_pin(entry: &PasswordEntry, submit: bool) -> Option<SecretPin> {
    let Some(text) = entry.delegate().and_downcast::<gtk4::Text>() else {
        entry.set_text("");
        return None;
    };
    let pin = if submit {
        // GTK owns this NUL-terminated UTF-8 pointer. The widget is kept alive
        // and not mutated until the copy completes; no borrowed pointer escapes.
        let pointer = unsafe {
            gtk4::ffi::gtk_editable_get_text(entry.upcast_ref::<gtk4::Editable>().to_glib_none().0)
        };
        if pointer.is_null() {
            None
        } else {
            unsafe { std::ffi::CStr::from_ptr(pointer) }
                .to_str()
                .ok()
                .map(SecretPin::from)
        }
    } else {
        None
    };
    // Clearing the text alone retains the old backing allocation. Release that
    // buffer before sending the PIN, including on cancellation. GTK owns its
    // secure allocator and any input-method copies; we do not mutate C memory.
    text.set_buffer(&gtk4::PasswordEntryBuffer::new());
    pin
}
fn wrapped_label(text: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    label.set_max_width_chars(42);
    label.set_lines(2);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_xalign(0.0);
    label
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
    let reason = wrapped_label(&crate::certificates::display_text(&info.reason, 160));
    content.append(&reason);
    let site = info.reported_site.as_deref().unwrap_or("not provided");
    let website = wrapped_label(&format!("Website (reported by page): {site}"));
    content.append(&website);
    content.append(&wrapped_label("Card PIN"));
    let entry = PasswordEntry::builder()
        .show_peek_icon(false)
        .hexpand(true)
        .build();
    entry.update_property(&[gtk4::accessible::Property::Label("Card PIN")]);
    content.append(&entry);
    let details = Box::new(Orientation::Vertical, 6);
    for text in info.cert.into_iter().chain(std::iter::once(token)) {
        let label = wrapped_label(&crate::certificates::display_text(&text, 160));
        details.append(&label);
    }
    let details = Expander::builder()
        .label("Certificate and card")
        .child(&details)
        .expanded(false)
        .build();
    content.append(&details);
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
        .default_width(360)
        .resizable(false)
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
                if let Some(pin) = take_pin(&entry, submit) {
                    reply.borrow_mut().submit(pin);
                } else {
                    reply.borrow_mut().cancel();
                }
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
