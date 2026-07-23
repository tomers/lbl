//! Fire-and-forget delivery: write the whole stream, then finish.
//!
//! Used by unidirectional protocols (raster page languages over a one-way
//! transport). No status is read; the printer consumes the bytes at its own
//! pace.

use crate::{DeliveryAction, Event, Handshake};

pub(crate) struct FireAndForget {
    bytes: Option<Vec<u8>>,
}

impl FireAndForget {
    pub(crate) fn new(label_bytes: &[u8]) -> Self {
        Self {
            bytes: Some(label_bytes.to_vec()),
        }
    }
}

impl Handshake for FireAndForget {
    fn start(&mut self) -> Vec<DeliveryAction> {
        let bytes = self.bytes.take().unwrap_or_default();
        vec![
            DeliveryAction::progress("sending"),
            DeliveryAction::send(bytes),
        ]
    }

    fn advance(&mut self, event: Event) -> Vec<DeliveryAction> {
        match event {
            Event::SendComplete => vec![DeliveryAction::Done],
            Event::Rx(_) => vec![DeliveryAction::error(
                "fire-and-forget delivery does not read from the device",
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{ClientDeliverySession, ClientHandshake, DeliveryAction};

    #[test]
    fn sends_all_bytes_then_done() {
        let payload = [1u8, 2, 3, 4];
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::FireAndForget, None, &payload).unwrap();

        // Opening progress, then the single write.
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::send(payload.to_vec()));

        let action = session.on_send_complete().unwrap();
        assert_eq!(action, DeliveryAction::Done);
        assert!(session.is_finished());
    }
}
