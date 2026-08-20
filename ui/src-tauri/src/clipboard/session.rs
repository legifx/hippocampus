//! The half of the clipboard that needs a display.
//!
//! Everything here talks to GTK, which means everything here must run on the main thread —
//! GTK's clipboard is not thread-safe and gtk-rs asserts as much on the way in. The commands
//! that call these functions hop threads first; see [`crate::clipboard::on_main_thread`].
//!
//! It is deliberately the thinnest possible layer over `gtk::Clipboard`: the formats, the
//! encoding and every decision about what a payload means live in the parent module, where they
//! can be tested without a compositor. What is left here is ownership and two round trips,
//! which cannot be tested without one.
//!
//! **Why GTK rather than `wl-copy`.** `wl-copy` would be four lines, and it works by forking a
//! process that stays alive to serve the data. That is a second process per copy, outliving the
//! window, holding paths that may have been moved since — and it is a dependency on a package
//! that is not installed everywhere. The overlay is already a resident GTK process with a
//! Wayland connection. Owning the selection is what it is for.

use super::{
    parse_gnome, parse_uri_list, Mode, Selection, GNOME_COPIED_FILES, KDE_CUT_SELECTION, URI_LIST,
    UTF8_TEXT,
};

use gtk::{TargetEntry, TargetFlags};

/// Which target the receiver asked for. `set_with_data` hands this back as a number rather than
/// a name, so the list below and this enum have to agree — hence one `const` per entry and no
/// literal indices anywhere else.
const TARGET_GNOME: u32 = 0;
const TARGET_URI_LIST: u32 = 1;
const TARGET_KDE_CUT: u32 = 2;
const TARGET_TEXT: u32 = 3;

/// Put files on the system clipboard, in every format anything reads.
///
/// Returns false when the compositor refused the selection. On Wayland that happens when the
/// window has no keyboard focus — a client may only take the selection off the back of a recent
/// input event — which is worth reporting rather than swallowing: a copy that silently did not
/// happen is discovered later, in another application, by pasting the wrong thing.
pub fn set(selection: &Selection) -> bool {
    let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);

    let mut targets = vec![
        TargetEntry::new(GNOME_COPIED_FILES, TargetFlags::empty(), TARGET_GNOME),
        TargetEntry::new(URI_LIST, TargetFlags::empty(), TARGET_URI_LIST),
    ];
    // Advertised only for a cut. KDE's convention is the presence of the target, and some
    // readers never look at the value, so offering it for a copy would turn every copy into a
    // move in Dolphin.
    if selection.mode == Mode::Move {
        targets.push(TargetEntry::new(
            KDE_CUT_SELECTION,
            TargetFlags::empty(),
            TARGET_KDE_CUT,
        ));
    }
    targets.push(TargetEntry::new(UTF8_TEXT, TargetFlags::empty(), TARGET_TEXT));

    let owned = selection.clone();
    clipboard.set_with_data(&targets, move |_, data, info| {
        let payload = match info {
            TARGET_GNOME => super::gnome_payload(&owned),
            TARGET_URI_LIST => super::uri_list_payload(&owned),
            TARGET_KDE_CUT => b"1".to_vec(),
            TARGET_TEXT => super::text_payload(&owned),
            // Nothing else was advertised, so nothing else can be asked for. Answering with an
            // empty payload is better than answering with the wrong one.
            _ => Vec::new(),
        };
        // The type is the target that was asked for, and the format is 8 because these are
        // bytes rather than words — the two ways to get this call wrong.
        data.set(&data.target(), 8, &payload);
    })
}

/// Read whatever the last application to copy something put there.
///
/// Most specific first: only `x-special/gnome-copied-files` carries a verb, so a cut from
/// Nautilus stays a cut. A plain `text/uri-list` becomes a copy, because a mover that guessed
/// would be a mover that deletes somebody's originals.
pub fn get() -> Option<Selection> {
    let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);

    if let Some(data) = clipboard.wait_for_contents(&gdk::Atom::intern(GNOME_COPIED_FILES)) {
        if let Some(selection) = parse_gnome(&bytes_of(&data)) {
            if !selection.is_empty() {
                return Some(selection);
            }
        }
    }

    let data = clipboard.wait_for_contents(&gdk::Atom::intern(URI_LIST))?;
    let paths = parse_uri_list(&bytes_of(&data));
    if paths.is_empty() {
        return None;
    }

    let cut = clipboard
        .wait_for_contents(&gdk::Atom::intern(KDE_CUT_SELECTION))
        .map(|flag| bytes_of(&flag) == b"1")
        .unwrap_or(false);

    Some(Selection {
        mode: if cut { Mode::Move } else { Mode::Copy },
        paths,
    })
}

/// Give up the selection.
///
/// Called after a cut has been carried out, the way every file manager does: the paths on the
/// clipboard no longer exist, and leaving them there means the next paste fails with a list of
/// files that are not there any more.
///
/// Safe to call when something else owns the selection: `gtk_clipboard_clear` gives up
/// ownership only if there is ownership to give up, so a cut carried out from *another* file
/// manager's clipboard leaves that manager's clipboard alone. Which is right — clearing
/// somebody else's selection would be rude and, worse, invisible.
pub fn clear() {
    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).clear();
}

/// The raw bytes of a selection.
///
/// `SelectionData` binds `text()` and `uris()` and no accessor for bytes, which is a gap rather
/// than a policy: `x-special/gnome-copied-files` is neither text nor a URI list as far as GTK is
/// concerned, so reading it means going through the C function that gtk-rs did not wrap.
fn bytes_of(data: &gtk::SelectionData) -> Vec<u8> {
    use glib::translate::ToGlibPtr;

    unsafe {
        let mut length: i32 = 0;
        let ptr = gtk::ffi::gtk_selection_data_get_data_with_length(
            data.to_glib_none().0,
            &mut length,
        );
        if ptr.is_null() || length <= 0 {
            return Vec::new();
        }
        std::slice::from_raw_parts(ptr, length as usize).to_vec()
    }
}
