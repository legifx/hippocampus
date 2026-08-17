//! Vocabulary for generated file names and content.
//!
//! German and English are mixed deliberately. The corpus exists to test a system whose users
//! write in both, and an English-only corpus would let a monolingual embedding model look far
//! better than it is (see decision D6). The M2 goldset is built against these words.

pub const EN_NOUNS: &[&str] = &[
    "report", "invoice", "meeting", "budget", "contract", "schedule", "analysis", "summary",
    "proposal", "review", "backup", "archive", "photo", "recording", "transcript", "draft",
    "config", "manifest", "dataset", "benchmark", "camera", "sensor", "pipeline", "cluster",
    "workshop", "interview", "roadmap", "invoice", "receipt", "certificate", "manual",
];

pub const DE_NOUNS: &[&str] = &[
    "bericht", "rechnung", "besprechung", "haushalt", "vertrag", "zeitplan", "auswertung",
    "zusammenfassung", "angebot", "pruefung", "sicherung", "archiv", "aufnahme", "entwurf",
    "einstellungen", "datensatz", "kamera", "messung", "werkstatt", "gespraech", "fahrplan",
    "quittung", "bescheinigung", "anleitung", "notizen", "protokoll", "uebersicht", "planung",
];

pub const ADJECTIVES: &[&str] = &[
    "final", "draft", "old", "new", "annual", "monthly", "internal", "public", "shared",
    "alt", "neu", "jaehrlich", "monatlich", "intern", "oeffentlich", "vorlaeufig", "endgueltig",
];

pub const PROJECT_NAMES: &[&str] = &[
    "aurora", "basalt", "cinder", "delta", "ember", "fjord", "granite", "harbor", "indigo",
    "juniper", "kestrel", "lattice", "meridian", "nimbus", "obsidian", "pinnacle", "quartz",
    "ridgeline", "summit", "tundra",
];

pub const TOP_LEVEL: &[&str] = &[
    "Documents", "Projects", "Pictures", "Downloads", "Music", "Videos", "Archive", "Notes",
];

pub const CODE_DIRS: &[&str] = &["src", "tests", "docs", "examples", "scripts", "assets"];

/// Sentence fragments used to fill text files.
///
/// Mixed language within a single corpus, and sometimes within a single file — which is the
/// realistic case and the hard one for retrieval.
pub const SENTENCES: &[&str] = &[
    "The camera angles were compared across three different mounting positions.",
    "Die Kamerawinkel wurden an drei verschiedenen Montagepositionen verglichen.",
    "Budget approval is pending until the quarterly review is complete.",
    "Die Freigabe des Haushalts steht bis zum Abschluss der Quartalspruefung aus.",
    "Latency measurements were taken on an otherwise idle machine.",
    "Die Latenzmessungen erfolgten auf einer ansonsten unbelasteten Maschine.",
    "This document supersedes the earlier draft from the previous month.",
    "Dieses Dokument ersetzt den frueheren Entwurf aus dem Vormonat.",
    "The sensor pipeline drops frames when the buffer fills faster than it drains.",
    "Die Sensor-Pipeline verwirft Bilder, wenn der Puffer schneller volllaeuft als er geleert wird.",
    "Follow-up actions were assigned during the meeting and are tracked separately.",
    "Die Folgeaufgaben wurden in der Besprechung verteilt und werden getrennt verfolgt.",
    "Storage costs grew faster than expected after the archive migration.",
    "Die Speicherkosten stiegen nach der Archivmigration schneller als erwartet.",
    "A rebuild from scratch produced an identical index, which is the property we rely on.",
    "Ein vollstaendiger Neuaufbau erzeugte einen identischen Index, worauf wir uns verlassen.",
];

pub const TEXT_EXTENSIONS: &[&str] = &["md", "txt", "org", "rst"];
pub const CODE_EXTENSIONS: &[&str] = &["rs", "py", "ts", "js", "go", "c", "h", "sh"];
pub const DOC_EXTENSIONS: &[&str] = &["pdf", "docx", "odt", "xlsx", "pptx"];
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "png", "webp", "heic", "svg"];
pub const MEDIA_EXTENSIONS: &[&str] = &["mp4", "mkv", "mp3", "flac", "wav"];
pub const BINARY_EXTENSIONS: &[&str] = &["bin", "dat", "iso", "zip", "tar"];
