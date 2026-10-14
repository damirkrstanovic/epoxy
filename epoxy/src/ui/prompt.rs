use super::PinReply;
pub(super) struct PendingReply(Option<PinReply>);
impl PendingReply {
    pub fn new(reply: PinReply) -> Self {
        Self(Some(reply))
    }
    pub fn cancel(&mut self) {
        if let Some(reply) = self.0.take() {
            let _ = reply.send(None);
        }
    }
    pub fn submit(&mut self, pin: String) {
        if let Some(reply) = self.0.take() {
            let _ = reply.send(Some(pin));
        }
    }
}
impl Drop for PendingReply {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_resolves_once_and_drop_does_not_leave_waiter() {
        let (tx, rx) = oneshot::channel();
        let mut reply = PendingReply::new(tx);
        reply.cancel();
        reply.cancel();
        assert_eq!(rx.try_recv().unwrap(), None);
        let (tx, rx) = oneshot::channel();
        drop(PendingReply::new(tx));
        assert_eq!(rx.try_recv().unwrap(), None);
    }
    #[test]
    fn submit_wins_over_later_close_and_closed_receiver_is_safe() {
        let (tx, rx) = oneshot::channel();
        let mut reply = PendingReply::new(tx);
        reply.submit("synthetic-secret".into());
        reply.cancel();
        assert_eq!(rx.try_recv().unwrap(), Some("synthetic-secret".into()));
        let (tx, rx) = oneshot::channel();
        drop(rx);
        PendingReply::new(tx).cancel();
    }
}
