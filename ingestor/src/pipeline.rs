use std::time::UNIX_EPOCH;

use tokio::sync::mpsc;

use crate::serial::Received;
use crate::store::Store;

#[derive(Debug, PartialEq)]
pub enum SeqEvent {
    InOrder,
    Gap { missing: u32 },
    Reset { last: u32 },
}

#[derive(Default)]
pub struct SeqTracker {
    last: Option<u32>,
}

impl SeqTracker {
    pub fn observe(&mut self, seq: u32) -> SeqEvent {
        let event = match self.last {
            None => SeqEvent::InOrder,
            Some(last) if seq == last.wrapping_add(1) => SeqEvent::InOrder,
            Some(last) if seq > last => SeqEvent::Gap {
                missing: seq - last - 1,
            },
            Some(last) => SeqEvent::Reset { last },
        };
        self.last = Some(seq);
        event
    }
}

pub async fn run(mut rx: mpsc::Receiver<Received>, store: Store) {
    let mut tracker = SeqTracker::default();
    while let Some(received) = rx.recv().await {
        let seq = received.frame.seq.0;
        match tracker.observe(seq) {
            SeqEvent::InOrder => {}
            SeqEvent::Gap { missing } => {
                eprintln!("Sequence gap: {missing} frame(s) missing before seq {seq}")
            }
            SeqEvent::Reset { last } => eprintln!("Sequence reset: seq {seq} after {last}"),
        }
        let ms = received
            .received_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        println!("{ms} {}", received.frame);
        if let Err(e) = store.insert(&received).await {
            eprintln!("Failed to store seq {seq}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_gaps_and_resets() {
        let mut tracker = SeqTracker::default();
        let events: Vec<_> = [5, 6, 9, 10, 1, 2]
            .into_iter()
            .map(|seq| tracker.observe(seq))
            .collect();

        assert_eq!(
            events,
            vec![
                SeqEvent::InOrder,
                SeqEvent::InOrder,
                SeqEvent::Gap { missing: 2 },
                SeqEvent::InOrder,
                SeqEvent::Reset { last: 10 },
                SeqEvent::InOrder,
            ]
        );
    }
}
