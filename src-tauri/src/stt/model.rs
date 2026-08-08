//! Which speech models exist, and getting one onto disk.
//!
//! Split so that the parts that can be wrong are testable without the network: the
//! catalogue, the resume arithmetic, and the checksum comparison are pure, and the
//! only untested remainder is a `get().send()` in a loop.
//!
//! The checksum is the thing to be careful about here. HuggingFace's `resolve` URL
//! returns an `ETag` that is sixty-four hex characters and is **not** the SHA-256 of
//! the file — see [`Model::sha256`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Which model Magi transcribes with.
///
/// Both multilingual and English-only variants, and the **default is multilingual**.
/// The first version of this offered only the `.en` models, on the reasoning that they
/// are better per byte at English — which is true, and was the wrong default for a
/// project aimed at a global contributor base. An `.en` model given Spanish does not
/// report that it cannot: it writes the English words that sound closest, so the
/// failure arrives as a confident wrong transcript.
///
/// `medium` is deliberately absent. `large-v3-turbo` is better *and* faster at a similar
/// size, and a menu where one option is strictly worse than another is a worse menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    /// 148 MB, ~99 languages. The default: understands whatever you speak, and a
    /// download someone will actually wait for on first launch.
    #[default]
    Base,
    /// 141 MB, English only. Slightly better at English than `Base` for the same size.
    BaseEn,
    /// 488 MB, multilingual. Noticeably better on accents and technical vocabulary.
    Small,
    /// 466 MB, English only.
    SmallEn,
    /// 1.6 GB, multilingual. The most accurate, and faster than the older `medium` it
    /// replaces.
    LargeV3Turbo,
}

impl Model {
    pub const ALL: [Model; 5] = [
        Model::Base,
        Model::BaseEn,
        Model::Small,
        Model::SmallEn,
        Model::LargeV3Turbo,
    ];

    /// The file name, which is also the name whisper.cpp knows it by.
    pub fn file_name(self) -> &'static str {
        match self {
            Model::Base => "ggml-base.bin",
            Model::BaseEn => "ggml-base.en.bin",
            Model::Small => "ggml-small.bin",
            Model::SmallEn => "ggml-small.en.bin",
            Model::LargeV3Turbo => "ggml-large-v3-turbo.bin",
        }
    }

    /// Whether this model understands languages other than English.
    ///
    /// The single most consequential fact about a model here, because getting it wrong
    /// is silent: an English-only model handed Spanish produces fluent nonsense rather
    /// than an error.
    pub fn is_multilingual(self) -> bool {
        match self {
            Model::Base | Model::Small | Model::LargeV3Turbo => true,
            Model::BaseEn | Model::SmallEn => false,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::Base => "Base",
            Model::BaseEn => "Base (English only)",
            Model::Small => "Small",
            Model::SmallEn => "Small (English only)",
            Model::LargeV3Turbo => "Large Turbo",
        }
    }

    /// What the user is trading by picking this one.
    pub fn description(self) -> &'static str {
        match self {
            Model::Base => {
                "Understands about 99 languages. Fast, and accurate enough for dictating \
                 a question — the right choice unless you find it mishearing you."
            }
            Model::BaseEn => {
                "English only, and a little better at it than Base for the same size. \
                 Speak anything else and it will write English words that sound similar."
            }
            Model::Small => {
                "Multilingual, and noticeably better with accents and technical words. A \
                 few times slower than Base."
            }
            Model::SmallEn => {
                "English only. Better with accents and technical words than Base, for \
                 English."
            }
            Model::LargeV3Turbo => {
                "The most accurate option, multilingual, and faster than its size \
                 suggests. Large enough that the download is a commitment."
            }
        }
    }

    /// Roughly how large the download is, for telling the user before it starts.
    ///
    /// An **estimate**. Two of these were wrong by about twelve thousand bytes when
    /// first written from memory, and that would have been a real bug: [`resume_from`]
    /// compares what is on disk against a total, and a total smaller than the real file
    /// makes a *completed* download look over-long — which discards it and starts again,
    /// forever. The server's `content-length` is the authority; this only renders
    /// "about 488 MB" before a request has been made.
    pub fn approximate_bytes(self) -> u64 {
        match self {
            Model::Base => 147_951_465,
            Model::BaseEn => 147_964_211,
            Model::Small => 487_601_967,
            Model::SmallEn => 487_614_201,
            Model::LargeV3Turbo => 1_624_555_275,
        }
    }

    /// The SHA-256 of the file's contents.
    ///
    /// From HuggingFace's API — `siblings[].lfs.sha256` — and **not** the `ETag` on the
    /// download URL. The ETag is also sixty-four hex characters, which is exactly what
    /// makes it dangerous: it looks like the value you want and is not. Verified for
    /// `base.en` by downloading and hashing it; the content hash matches the API and does
    /// not match the ETag.
    pub fn sha256(self) -> &'static str {
        match self {
            Model::Base => "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
            Model::BaseEn => "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
            Model::Small => "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
            Model::SmallEn => "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
            Model::LargeV3Turbo => {
                "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"
            }
        }
    }

    pub fn url(self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file_name()
        )
    }

    /// Where the model lives, given the app's data directory.
    pub fn path_in(self, dir: &Path) -> PathBuf {
        dir.join(self.file_name())
    }

    /// Where a partial download lives.
    ///
    /// A separate name so an interrupted download is never mistaken for a usable model.
    /// Renaming into place only after the checksum passes is what makes a half-written
    /// file impossible to load.
    pub fn partial_path_in(self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.partial", self.file_name()))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum DownloadError {
    #[error("the download could not reach {url}: {reason}")]
    Unreachable { url: String, reason: String },

    #[error("{url} returned HTTP {status}")]
    Http { url: String, status: u16 },

    #[error("the download could not be written to {path}: {reason}")]
    Io { path: String, reason: String },

    /// The bytes arrived but are not the model.
    ///
    /// Its own variant because the recovery is specific: delete the partial file and
    /// start again. A resumed download that was corrupted before the interruption
    /// would otherwise resume forever onto bad data.
    #[error(
        "the downloaded model is corrupt — expected checksum {expected}, got {actual}. \
         The partial file has been discarded; try again."
    )]
    ChecksumMismatch { expected: String, actual: String },

    #[error("the download was cancelled")]
    Cancelled,
}

/// How far along a download is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
}

impl Progress {
    pub fn percent(self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        ((self.downloaded.min(self.total) * 100) / self.total) as u8
    }
}

/// What a partial file on disk means for the next attempt.
///
/// Pure, and separated from the request because this is the part that can be wrong in
/// a way no test of the happy path would catch: resuming from the wrong offset
/// produces a file of exactly the right length whose contents are garbage, and the
/// only thing that catches it is the checksum at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// Nothing on disk. Request the whole file.
    FromStart,
    /// `bytes` already present. Request the remainder with a range header.
    From(u64),
    /// The partial file is already the full length. Skip the transfer and verify.
    AlreadyComplete,
    /// The partial file is longer than the model. It cannot be a prefix of anything
    /// useful, so it is discarded rather than resumed from.
    Discard,
}

/// Decides how to continue, given what is on disk and how big the model is.
pub fn resume_from(existing: u64, total: u64) -> Resume {
    if existing == 0 {
        Resume::FromStart
    } else if existing == total {
        Resume::AlreadyComplete
    } else if existing > total {
        // Longer than the model it claims to be. Something else wrote here, or the
        // size constant changed; either way the bytes cannot be trusted as a prefix.
        Resume::Discard
    } else {
        Resume::From(existing)
    }
}

/// Whether a file's hash matches what the model should be.
pub fn checksum_matches(digest: &str, expected: &str) -> bool {
    // Case-insensitive: the API returns lowercase, but a hash pasted from elsewhere
    // may not be, and failing over letter case would be an infuriating bug.
    digest.eq_ignore_ascii_case(expected)
}

/// Hashes a file on disk.
///
/// Streamed in chunks rather than read whole: `medium.en` is 1.4 GB, and loading it
/// into memory to hash it would use more than transcribing with it does.
pub fn hash_file(path: &Path) -> Result<String, DownloadError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|e| DownloadError::Io {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        let read = file.read(&mut buffer).map_err(|e| DownloadError::Io {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_has_a_distinct_file_and_checksum() {
        // A copy-paste in the table would give two models the same hash, and the
        // second one would fail verification with a message about corruption.
        let names: std::collections::HashSet<_> =
            Model::ALL.iter().map(|m| m.file_name()).collect();
        let hashes: std::collections::HashSet<_> = Model::ALL.iter().map(|m| m.sha256()).collect();

        assert_eq!(names.len(), Model::ALL.len());
        assert_eq!(hashes.len(), Model::ALL.len());
    }

    #[test]
    fn every_checksum_is_a_sha256() {
        for model in Model::ALL {
            let hash = model.sha256();
            assert_eq!(hash.len(), 64, "{:?} has a {}-char hash", model, hash.len());
            assert!(
                hash.chars().all(|c| c.is_ascii_hexdigit()),
                "{model:?} has a non-hex checksum"
            );
        }
    }

    #[test]
    fn the_base_model_checksum_is_the_one_that_was_verified_by_hashing() {
        // Downloaded and hashed rather than copied from a header. The ETag on the same
        // URL is also 64 hex characters and is a different value — verifying against
        // it would fail every download on every machine.
        assert_eq!(
            Model::BaseEn.sha256(),
            "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"
        );
        assert_ne!(
            Model::BaseEn.sha256(),
            "ff7d10f8526045d48149699b43aeaa014e4b337239bc5a35251116fc179aabcf",
            "that is the ETag, not the content hash"
        );
    }

    #[test]
    fn the_sizes_match_what_the_api_reported_when_they_were_taken() {
        // Not load-bearing — the server's content-length is the authority — but wrong by
        // enough to matter for a progress bar. Two of these were about twelve thousand
        // bytes out when written from memory, which would have made a completed download
        // look longer than the model and be discarded. Instructively, the wrong number I
        // wrote for `small.en` turned out to be the real size of multilingual `small`:
        // recall had mixed up the two variants, which is the same mistake as shipping
        // English-only models by default.
        assert_eq!(Model::Base.approximate_bytes(), 147_951_465);
        assert_eq!(Model::BaseEn.approximate_bytes(), 147_964_211);
        assert_eq!(Model::Small.approximate_bytes(), 487_601_967);
        assert_eq!(Model::SmallEn.approximate_bytes(), 487_614_201);
        assert_eq!(Model::LargeV3Turbo.approximate_bytes(), 1_624_555_275);
    }

    #[test]
    fn the_default_is_multilingual_and_the_smallest_of_those() {
        // The first version defaulted to English-only. For a project aimed at a global
        // contributor base that was the wrong choice, and it fails silently: an `.en`
        // model handed Spanish writes English words that sound similar rather than
        // reporting that it cannot.
        assert_eq!(Model::default(), Model::Base);
        assert!(Model::default().is_multilingual());

        let smallest_multilingual = Model::ALL
            .iter()
            .filter(|m| m.is_multilingual())
            .min_by_key(|m| m.approximate_bytes())
            .copied();
        assert_eq!(smallest_multilingual, Some(Model::default()));
    }

    #[test]
    fn every_english_only_model_says_so_in_its_label_and_description() {
        // The trap this whole change exists to close. Someone scanning a list must be able
        // to see which options will not understand them.
        for model in Model::ALL.iter().filter(|m| !m.is_multilingual()) {
            assert!(
                model.label().contains("English only"),
                "{model:?} does not say so in its label"
            );
            assert!(
                model.description().contains("English only"),
                "{model:?} does not say so in its description"
            );
        }

        for model in Model::ALL.iter().filter(|m| m.is_multilingual()) {
            assert!(
                !model.label().contains("English only"),
                "{model:?} is multilingual but labelled otherwise"
            );
        }
    }

    #[test]
    fn the_file_name_tells_the_truth_about_multilingual_support() {
        // whisper.cpp's own convention: `.en` in the name means English-only. A variant
        // added later with the flag set wrongly would transcribe silently badly.
        for model in Model::ALL {
            let english_only_name = model.file_name().contains(".en.");
            assert_eq!(
                english_only_name,
                !model.is_multilingual(),
                "{model:?}: file name {} disagrees with is_multilingual()",
                model.file_name()
            );
        }
    }

    #[test]
    fn the_url_points_at_the_file_name() {
        let url = Model::SmallEn.url();
        assert!(url.starts_with("https://huggingface.co/"));
        assert!(url.ends_with("ggml-small.en.bin"));
    }

    #[test]
    fn a_partial_download_is_never_mistaken_for_a_model() {
        // The reason the partial has its own name: a half-written file at the model's
        // path would be loaded, and whisper.cpp's failure on it reads as a corrupt
        // model rather than an interrupted download.
        let dir = Path::new("/tmp/magi-test");
        assert_ne!(
            Model::BaseEn.path_in(dir),
            Model::BaseEn.partial_path_in(dir)
        );
        assert!(Model::BaseEn
            .partial_path_in(dir)
            .to_string_lossy()
            .ends_with(".partial"));
    }

    // ---- resume arithmetic -------------------------------------------------

    #[test]
    fn nothing_on_disk_downloads_the_whole_file() {
        assert_eq!(resume_from(0, 1_000), Resume::FromStart);
    }

    #[test]
    fn a_partial_file_resumes_from_its_length() {
        // The whole point: 465 MB restarting from zero on a dropped connection is a
        // download that never finishes on a bad one.
        assert_eq!(resume_from(400, 1_000), Resume::From(400));
    }

    #[test]
    fn a_complete_file_skips_the_transfer_and_verifies() {
        // The case where the connection dropped after the last byte but before the
        // rename. Re-downloading 465 MB to discover it was already there would be a
        // poor way to find out.
        assert_eq!(resume_from(1_000, 1_000), Resume::AlreadyComplete);
    }

    #[test]
    fn a_file_longer_than_the_model_is_discarded_rather_than_resumed() {
        // It cannot be a prefix of the model, so resuming would append onto bytes that
        // are already wrong and produce a file of the right length and wrong contents.
        assert_eq!(resume_from(2_000, 1_000), Resume::Discard);
    }

    #[test]
    fn resume_handles_a_zero_total_without_dividing_by_it() {
        // A defensive case: a server that reports no length.
        assert_eq!(resume_from(0, 0), Resume::FromStart);
        assert_eq!(resume_from(10, 0), Resume::Discard);
    }

    // ---- checksum comparison ----------------------------------------------

    #[test]
    fn checksums_compare_case_insensitively() {
        // The API returns lowercase; a hash pasted from elsewhere may not be. Failing
        // over letter case would be an infuriating bug to diagnose.
        let lower = "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002";
        assert!(checksum_matches(lower, &lower.to_uppercase()));
        assert!(checksum_matches(&lower.to_uppercase(), lower));
    }

    #[test]
    fn a_different_checksum_does_not_match() {
        assert!(!checksum_matches(
            Model::BaseEn.sha256(),
            Model::SmallEn.sha256()
        ));
        assert!(!checksum_matches("", Model::BaseEn.sha256()));
    }

    // ---- progress ----------------------------------------------------------

    #[test]
    fn progress_reports_a_percentage() {
        assert_eq!(
            Progress {
                downloaded: 0,
                total: 100
            }
            .percent(),
            0
        );
        assert_eq!(
            Progress {
                downloaded: 50,
                total: 100
            }
            .percent(),
            50
        );
        assert_eq!(
            Progress {
                downloaded: 100,
                total: 100
            }
            .percent(),
            100
        );
    }

    #[test]
    fn progress_does_not_divide_by_zero_or_exceed_a_hundred() {
        assert_eq!(
            Progress {
                downloaded: 10,
                total: 0
            }
            .percent(),
            0
        );
        // A server that sends more than it promised must not produce 103%.
        assert_eq!(
            Progress {
                downloaded: 150,
                total: 100
            }
            .percent(),
            100
        );
    }

    #[test]
    fn progress_survives_a_gigabyte_scale_download() {
        // The concern is the `* 100`: at 1.4 GB that is 153 billion, which needs u64.
        // In u32 it would wrap and report a nonsense percentage.
        //
        // Asserted as a range rather than an exact 50, because the total is odd — my
        // first version of this test asserted 50 and got 49, which was the test being
        // wrong about integer division rather than the code being wrong.
        let total = Model::LargeV3Turbo.approximate_bytes();
        let progress = Progress {
            downloaded: total / 2,
            total,
        };
        assert!(
            (49..=50).contains(&progress.percent()),
            "got {}%",
            progress.percent()
        );

        // And the endpoints are exact whatever the size.
        assert_eq!(
            Progress {
                downloaded: 0,
                total
            }
            .percent(),
            0
        );
        assert_eq!(
            Progress {
                downloaded: total,
                total
            }
            .percent(),
            100
        );
    }

    // ---- hashing -----------------------------------------------------------

    #[test]
    fn hashing_a_file_matches_a_known_digest() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("known");
        std::fs::write(&path, b"abc").expect("writable");

        // The SHA-256 of "abc", which is the canonical test vector.
        assert_eq!(
            hash_file(&path).expect("hashable"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hashing_streams_rather_than_reading_the_whole_file() {
        // A file larger than the 1 MB read buffer, to exercise the loop rather than a
        // single read. medium.en is 1.4 GB, and hashing it by loading it would use more
        // memory than transcribing with it.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("big");
        std::fs::write(&path, vec![7u8; 3 * 1024 * 1024]).expect("writable");

        let digest = hash_file(&path).expect("hashable");
        assert_eq!(digest.len(), 64);
        // Deterministic.
        assert_eq!(digest, hash_file(&path).expect("hashable"));
    }

    #[test]
    fn hashing_a_missing_file_names_the_path() {
        let error = hash_file(Path::new("/nonexistent/model.bin")).expect_err("must fail");
        assert!(error.to_string().contains("/nonexistent/model.bin"));
    }

    #[test]
    fn a_checksum_mismatch_says_what_to_do_about_it() {
        // The recovery is specific: the partial file is gone and a retry will work. A
        // resumed download onto already-corrupt bytes would otherwise resume forever.
        let error = DownloadError::ChecksumMismatch {
            expected: "aaa".into(),
            actual: "bbb".into(),
        };
        let message = error.to_string();
        assert!(message.contains("discarded"), "got: {message}");
        assert!(message.contains("try again"), "got: {message}");
    }
}

/// Downloads a model, resuming a partial file if one is there.
///
/// The only part of this module that touches the network, and deliberately the
/// thinnest: every decision it makes — where to resume from, whether the bytes are
/// right — is one of the pure functions above.
///
/// `on_progress` is called as bytes arrive. Returning `false` from it cancels, which
/// is how the UI stops a 1.4 GB download the user changed their mind about; the
/// partial file is left in place so the next attempt resumes rather than restarts.
///
/// Blocking, and called from `spawn_blocking`. A download that streams to disk has
/// nothing to gain from being async here and would need a runtime to test.
pub fn download(
    client: &reqwest::blocking::Client,
    model: Model,
    dir: &Path,
    mut on_progress: impl FnMut(Progress) -> bool,
) -> Result<PathBuf, DownloadError> {
    use std::io::{Read, Seek, SeekFrom, Write};

    let final_path = model.path_in(dir);
    if final_path.exists() {
        return Ok(final_path);
    }

    std::fs::create_dir_all(dir).map_err(|e| DownloadError::Io {
        path: dir.display().to_string(),
        reason: e.to_string(),
    })?;

    let partial = model.partial_path_in(dir);
    let url = model.url();

    // The server's length is the authority, not `approximate_bytes` — a constant that
    // disagreed with reality by a few thousand bytes would make a completed download
    // look over-long and be discarded on every attempt.
    let head = client
        .head(&url)
        .send()
        .map_err(|e| DownloadError::Unreachable {
            url: url.clone(),
            reason: e.to_string(),
        })?;

    let total = head
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or_else(|| model.approximate_bytes());

    let existing = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);

    let mut downloaded = match resume_from(existing, total) {
        Resume::AlreadyComplete => {
            tracing::info!("the partial download is already complete; verifying");
            existing
        }
        Resume::Discard => {
            tracing::warn!(
                existing,
                total,
                "the partial file is longer than the model; discarding it"
            );
            let _ = std::fs::remove_file(&partial);
            0
        }
        Resume::FromStart => 0,
        Resume::From(offset) => {
            tracing::info!(offset, total, "resuming the model download");
            offset
        }
    };

    // Skip the transfer entirely when the file is already whole; the checksum below is
    // what decides whether it is usable.
    if downloaded < total {
        let mut request = client.get(&url);
        if downloaded > 0 {
            // The whole point of the partial file. 465 MB restarting from zero on a
            // dropped connection is a download that never finishes on a bad one.
            request = request.header(reqwest::header::RANGE, format!("bytes={downloaded}-"));
        }

        let mut response = request.send().map_err(|e| DownloadError::Unreachable {
            url: url.clone(),
            reason: e.to_string(),
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(DownloadError::Http {
                url: url.clone(),
                status: status.as_u16(),
            });
        }

        // A server that ignores the range header answers 200 with the whole file
        // rather than 206 with the remainder. Appending that to a partial file would
        // produce a corrupt result of the right-ish length, so the offset is reset
        // rather than trusted.
        if downloaded > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
            tracing::warn!(
                %status,
                "the server ignored the range request; restarting the download"
            );
            downloaded = 0;
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(downloaded == 0)
            .open(&partial)
            .map_err(|e| DownloadError::Io {
                path: partial.display().to_string(),
                reason: e.to_string(),
            })?;

        file.seek(SeekFrom::Start(downloaded))
            .map_err(|e| DownloadError::Io {
                path: partial.display().to_string(),
                reason: e.to_string(),
            })?;

        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let read = response
                .read(&mut buffer)
                .map_err(|e| DownloadError::Unreachable {
                    url: url.clone(),
                    reason: e.to_string(),
                })?;
            if read == 0 {
                break;
            }

            file.write_all(&buffer[..read])
                .map_err(|e| DownloadError::Io {
                    path: partial.display().to_string(),
                    reason: e.to_string(),
                })?;
            downloaded += read as u64;

            if !on_progress(Progress { downloaded, total }) {
                // Cancelled. The partial file stays, so the next attempt resumes from
                // here instead of starting again.
                let _ = file.flush();
                return Err(DownloadError::Cancelled);
            }
        }

        file.flush().map_err(|e| DownloadError::Io {
            path: partial.display().to_string(),
            reason: e.to_string(),
        })?;
    }

    // Verified before the rename, never after. A file at the model's path is a file
    // whisper.cpp will load, and its failure on a corrupt one reads as a broken model
    // rather than a bad download.
    let digest = hash_file(&partial)?;
    if !checksum_matches(&digest, model.sha256()) {
        // Discarded rather than kept. Resuming onto bytes that are already wrong would
        // append forever and fail the checksum every time.
        let _ = std::fs::remove_file(&partial);
        return Err(DownloadError::ChecksumMismatch {
            expected: model.sha256().to_string(),
            actual: digest,
        });
    }

    std::fs::rename(&partial, &final_path).map_err(|e| DownloadError::Io {
        path: final_path.display().to_string(),
        reason: e.to_string(),
    })?;

    tracing::info!(model = ?model, path = %final_path.display(), "speech model downloaded");
    Ok(final_path)
}

#[cfg(test)]
mod download_tests {
    use super::*;

    #[test]
    fn an_already_downloaded_model_is_returned_without_a_request() {
        // No client is used, which is the assertion: a present model must not cost a
        // network round trip on every launch.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = Model::BaseEn.path_in(dir.path());
        std::fs::write(&path, b"pretend model").expect("writable");

        let client = reqwest::blocking::Client::new();
        let found = download(&client, Model::BaseEn, dir.path(), |_| true)
            .expect("an existing model is found");
        assert_eq!(found, path);
    }

    #[test]
    fn the_partial_and_final_paths_never_collide() {
        let dir = tempfile::tempdir().expect("temp dir");
        for model in Model::ALL {
            assert_ne!(model.path_in(dir.path()), model.partial_path_in(dir.path()));
        }
    }
}
