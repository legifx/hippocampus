//! One document per event into a directory the peer already watches (D17).
//!
//! The safest transport there is: it needs no credential, it fails closed when the directory
//! is not there, and a peer that deduplicates files by name gets deduplication for free
//! because the name is the event id.
//!
//! The street down is a single file the peer may leave in the same directory,
//! `ontology.json`. It is read on every push and never removed — it is the peer's.

use crate::protocol::{Ontology, SyncAck, SyncBatch};
use crate::transport::{Transport, TransportError};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const ONTOLOGY_FILE: &str = "ontology.json";

pub struct FileDrop {
    dir: PathBuf,
}

impl FileDrop {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The name of an event's document. Stable, so a resend overwrites rather than duplicates.
    pub fn document_name(event_id: &str) -> String {
        format!("{event_id}.json")
    }
}

impl Transport for FileDrop {
    fn name(&self) -> String {
        "file-drop".into()
    }

    fn push(&mut self, batch: &SyncBatch) -> Result<SyncAck, TransportError> {
        // Not created: a missing directory is most likely an unmounted share, and creating one
        // in its place would write the peer's mail onto the local disk where the peer will
        // never look. Fail closed (D17).
        if !self.dir.is_dir() {
            return Err(TransportError::Unreachable(format!(
                "drop directory is not there: {}",
                self.dir.display()
            )));
        }

        let mut ack = SyncAck::default();
        for event in &batch.events {
            let name = Self::document_name(&event.event_id);
            let tmp = self.dir.join(format!(".{name}.part"));
            let done = self.dir.join(&name);
            let write = (|| -> std::io::Result<()> {
                let mut file = std::fs::File::create(&tmp)?;
                serde_json::to_writer(&mut file, event)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                file.write_all(b"\n")?;
                file.sync_all()?;
                std::fs::rename(&tmp, &done)
            })();
            match write {
                Ok(()) => ack.accepted.push(event.event_id.clone()),
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    // A disk that stopped accepting writes half way is unreachable for the
                    // rest; what was written is acknowledged, the rest is retried whole.
                    if ack.accepted.is_empty() {
                        return Err(TransportError::Unreachable(e.to_string()));
                    }
                    break;
                }
            }
        }

        if let Ok(text) = std::fs::read_to_string(self.dir.join(ONTOLOGY_FILE)) {
            match serde_json::from_str::<Ontology>(&text) {
                Ok(ontology) => ack.ontology = Some(ontology),
                Err(e) => tracing::warn!(error = %e, "the peer's ontology.json is not readable"),
            }
        }
        Ok(ack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{SyncEvent, PROTOCOL_VERSION};

    fn batch(ids: &[&str]) -> SyncBatch {
        SyncBatch {
            protocol: PROTOCOL_VERSION,
            origin: "o".into(),
            lamport: ids.len() as i64,
            sent_at: 0,
            events: ids
                .iter()
                .enumerate()
                .map(|(i, id)| SyncEvent {
                    event_id: (*id).into(),
                    lamport: i as i64 + 1,
                    origin: "o".into(),
                    ts: 0,
                    kind: "episode".into(),
                    payload: serde_json::json!({ "n": i }),
                })
                .collect(),
        }
    }

    #[test]
    fn a_missing_directory_is_unreachable_and_nothing_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("share").join("inbox");
        let mut t = FileDrop::new(&dir);
        match t.push(&batch(&["a"])) {
            Err(TransportError::Unreachable(_)) => {}
            other => panic!("{other:?}"),
        }
        assert!(!dir.exists());
    }

    #[test]
    fn one_document_per_event_named_by_id_and_a_resend_makes_no_second_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut t = FileDrop::new(tmp.path());
        let ack = t.push(&batch(&["a", "b"])).unwrap();
        assert_eq!(ack.accepted, vec!["a", "b"]);
        t.push(&batch(&["a", "b"])).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.json", "b.json"]);
        let doc: SyncEvent =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join("a.json")).unwrap())
                .unwrap();
        assert_eq!(doc.event_id, "a");
    }

    #[test]
    fn the_peer_may_leave_an_ontology_and_it_comes_back_in_the_ack() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(ONTOLOGY_FILE),
            r#"{"version": 3, "tags": [{"name": "finance"}]}"#,
        )
        .unwrap();
        let mut t = FileDrop::new(tmp.path());
        let ack = t.push(&batch(&["a"])).unwrap();
        let ontology = ack.ontology.unwrap();
        assert_eq!(ontology.version, 3);
        assert_eq!(ontology.tags[0].name, "finance");
        assert!(tmp.path().join(ONTOLOGY_FILE).exists(), "the peer's file is the peer's");
    }
}
