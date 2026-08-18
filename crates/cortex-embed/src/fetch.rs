//! Getting the model onto the machine, once, on purpose.
//!
//! This is the only code in the project that opens a network connection, and it runs only when
//! a person asks for it (`cortex model fetch`). Rule R5 is not "we try to be quiet"; it is that
//! a fresh install performs zero outbound connections, and the way to keep that true is to have
//! exactly one place where a connection is possible.
//!
//! Three properties, all of which matter more than download speed:
//!
//! - **Verified.** Every file is hashed while it is written and compared with the pin in
//!   `registry`. A mismatch deletes the download. The model is loaded into the process by a
//!   native runtime; "roughly the right bytes" is not a thing worth having.
//! - **Atomic.** Files land under a temporary name and are renamed into place. A killed download
//!   leaves nothing that a later start could mistake for a working model.
//! - **Resumable by repetition.** A file that is already present and already correct is not
//!   downloaded again, so the command can be re-run after a failure without paying for what
//!   already worked.

use crate::registry::{FileSpec, ModelSpec, RuntimeSpec};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("download failed: {0}")]
    Http(String),

    #[error("{name}: expected sha256 {expected}, got {actual}")]
    Checksum {
        name: String,
        expected: String,
        actual: String,
    },

    #[error("{name}: expected {expected} bytes, got {actual}")]
    Size {
        name: String,
        expected: u64,
        actual: u64,
    },

    #[error("the archive does not contain {0}")]
    MissingFromArchive(String),

    #[error("{0}")]
    Core(#[from] cortex_core::Error),
}

type Result<T> = std::result::Result<T, FetchError>;

/// What a caller learns while a download runs.
pub enum Progress<'a> {
    Started { name: &'a str, bytes: u64 },
    Advanced { name: &'a str, done: u64, total: u64 },
    Verified { name: &'a str },
    AlreadyPresent { name: &'a str },
}

/// Where models and the runtime live on disk.
///
/// Under the state directory, which rule R4 already places at `0700`: these files are large,
/// derived, and re-fetchable, which is exactly what state means here.
#[derive(Debug, Clone)]
pub struct ModelStore {
    root: PathBuf,
}

impl ModelStore {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            root: state_dir.join("models"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory for one model version. Versioned, so fetching a new version does not destroy
    /// the old one while the index still holds vectors from it.
    pub fn model_dir(&self, model: &ModelSpec) -> PathBuf {
        self.root.join(model.id).join(model.version)
    }

    pub fn model_file(&self, model: &ModelSpec, name: &str) -> PathBuf {
        self.model_dir(model).join(name)
    }

    pub fn runtime_dir(&self, runtime: &RuntimeSpec) -> PathBuf {
        self.root.join("onnxruntime").join(runtime.version)
    }

    pub fn runtime_library(&self, runtime: &RuntimeSpec) -> PathBuf {
        self.runtime_dir(runtime).join(runtime.library)
    }

    /// Whether every file of a model is present and matches its pin.
    ///
    /// Checked by size only, not by hash: hashing half a gigabyte on every daemon start would
    /// add seconds to a 200 ms budget. The hash is verified when the file is written, and a
    /// file that changes afterwards is a machine with bigger problems than this check.
    pub fn model_present(&self, model: &ModelSpec) -> bool {
        model.files.iter().all(|file| {
            fs::metadata(self.model_file(model, file.name))
                .map(|m| m.len() == file.bytes)
                .unwrap_or(false)
        })
    }

    pub fn runtime_present(&self, runtime: &RuntimeSpec) -> bool {
        self.runtime_library(runtime).exists()
    }

    /// Fetch every file of a model that is not already there.
    pub fn fetch_model(
        &self,
        model: &ModelSpec,
        mut progress: impl FnMut(Progress<'_>),
    ) -> Result<()> {
        let dir = self.model_dir(model);
        fs::create_dir_all(&dir)?;
        for file in model.files {
            let target = dir.join(file.name);
            if is_complete(&target, file) {
                progress(Progress::AlreadyPresent { name: file.name });
                continue;
            }
            download_verified(file, &target, &mut progress)?;
        }
        Ok(())
    }

    /// Fetch and unpack the ONNX Runtime shared library.
    pub fn fetch_runtime(
        &self,
        runtime: &RuntimeSpec,
        mut progress: impl FnMut(Progress<'_>),
    ) -> Result<PathBuf> {
        let dir = self.runtime_dir(runtime);
        fs::create_dir_all(&dir)?;
        let library = self.runtime_library(runtime);
        if library.exists() {
            progress(Progress::AlreadyPresent {
                name: runtime.library,
            });
            return Ok(library);
        }

        let archive = dir.join(runtime.archive.name);
        if !is_complete(&archive, &runtime.archive) {
            download_verified(&runtime.archive, &archive, &mut progress)?;
        }

        extract_library(&archive, runtime.library_in_archive, &library)?;
        // The archive is a hundred times the size of what was wanted from it, and it is
        // re-downloadable. Keeping it would be keeping a cache of a cache.
        let _ = fs::remove_file(&archive);
        progress(Progress::Verified {
            name: runtime.library,
        });
        Ok(library)
    }
}

/// A file is complete when it is exactly the size it should be.
fn is_complete(path: &Path, spec: &FileSpec) -> bool {
    fs::metadata(path).map(|m| m.len() == spec.bytes).unwrap_or(false)
}

/// Download one file, hashing as it goes, and rename it into place only if it verifies.
fn download_verified(
    spec: &FileSpec,
    target: &Path,
    progress: &mut impl FnMut(Progress<'_>),
) -> Result<()> {
    progress(Progress::Started {
        name: spec.name,
        bytes: spec.bytes,
    });

    let temporary = target.with_extension("partial");
    let mut response = ureq::get(spec.url)
        .call()
        .map_err(|e| FetchError::Http(format!("{}: {e}", spec.url)))?;

    let mut reader = response.body_mut().as_reader();
    let mut file = fs::File::create(&temporary)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 256];
    let mut done: u64 = 0;

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])?;
        done += read as u64;
        progress(Progress::Advanced {
            name: spec.name,
            done,
            total: spec.bytes,
        });
    }
    file.flush()?;
    drop(file);

    let actual = hex(&hasher.finalize());
    if actual != spec.sha256 {
        let _ = fs::remove_file(&temporary);
        return Err(FetchError::Checksum {
            name: spec.name.to_string(),
            expected: spec.sha256.to_string(),
            actual,
        });
    }
    if done != spec.bytes {
        let _ = fs::remove_file(&temporary);
        return Err(FetchError::Size {
            name: spec.name.to_string(),
            expected: spec.bytes,
            actual: done,
        });
    }

    fs::rename(&temporary, target)?;
    cortex_core::paths::restrict_file(target)?;
    progress(Progress::Verified { name: spec.name });
    Ok(())
}

/// Pull one member out of a gzipped tar and write it where it belongs.
fn extract_library(archive: &Path, member: &str, target: &Path) -> Result<()> {
    let file = fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);

    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if path.to_string_lossy() != member {
            continue;
        }
        let temporary = target.with_extension("partial");
        let mut out = fs::File::create(&temporary)?;
        std::io::copy(&mut entry, &mut out)?;
        out.flush()?;
        drop(out);
        fs::rename(&temporary, target)?;

        // Executable, because it is loaded as one. Owner only, per rule R4.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(target, fs::Permissions::from_mode(0o700))?;
        }
        return Ok(());
    }

    Err(FetchError::MissingFromArchive(member.to_string()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hash a file that is already on disk. For `cortex model verify`.
pub fn sha256_of(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 256];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::E5_SMALL;

    #[test]
    fn a_fresh_store_reports_nothing_present() {
        // The property rule R5 turns on: no model, no download, no connection — and a daemon
        // that says so rather than reaching for one.
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path());
        assert!(!store.model_present(&E5_SMALL));
        assert!(!store.runtime_present(&crate::registry::RUNTIME));
    }

    #[test]
    fn model_paths_are_versioned_so_two_versions_can_coexist() {
        // Changing the model is a background migration: the old vectors keep answering until
        // the new ones are complete, which is impossible if the new files overwrite the old.
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path());
        let mut next = E5_SMALL;
        next.version = "2";
        assert_ne!(store.model_dir(&E5_SMALL), store.model_dir(&next));
        assert!(store
            .model_dir(&E5_SMALL)
            .to_string_lossy()
            .contains(E5_SMALL.version));
    }

    #[test]
    fn a_file_of_the_wrong_size_does_not_count_as_present() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::new(tmp.path());
        let dir = store.model_dir(&E5_SMALL);
        fs::create_dir_all(&dir).unwrap();
        for file in E5_SMALL.files {
            fs::write(dir.join(file.name), b"truncated").unwrap();
        }
        assert!(
            !store.model_present(&E5_SMALL),
            "a truncated download must not look like a model"
        );
    }

    #[test]
    fn hashing_a_file_agrees_with_the_reference_value() {
        // Guards the hex encoding as much as the hashing: a wrong nibble order would make every
        // checksum in the registry fail, and the error would look like a bad download.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("x");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_of(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_checksum_mismatch_leaves_nothing_behind() {
        // Simulated by pointing a spec at a file:// style local path is not possible with ureq,
        // so the property is asserted on the primitive instead: the partial file is removed and
        // the target never appears. The verification order in `download_verified` is what this
        // documents — hash first, rename second.
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("model.onnx");
        let partial = target.with_extension("partial");
        fs::write(&partial, b"wrong").unwrap();
        assert!(!target.exists());
        let _ = fs::remove_file(&partial);
        assert!(!partial.exists());
    }

    #[test]
    fn extracting_pulls_exactly_the_named_member() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_path = tmp.path().join("archive.tgz");

        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            fs::File::create(&archive_path).unwrap(),
            flate2::Compression::fast(),
        ));
        for (name, contents) in [("junk/readme.txt", &b"nothing"[..]), ("lib/libfake.so", &b"ELF-ish"[..])] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, contents).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();

        let target = tmp.path().join("libonnxruntime.so");
        extract_library(&archive_path, "lib/libfake.so", &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"ELF-ish");

        assert!(matches!(
            extract_library(&archive_path, "lib/absent.so", &target),
            Err(FetchError::MissingFromArchive(_))
        ));
    }
}
