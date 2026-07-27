// src/track.rs

use core::fmt;
use lofty::file::FileType;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use std::path::{Path, PathBuf};

pub fn read_tags(path: &Path) -> Result<TrackInfo, Box<dyn std::error::Error + Send + Sync>> {
    let reader = Probe::open(path)?;
    let file = reader.read()?;
    let file_type = file.file_type();
    let tag = file.primary_tag();
    let props = file.properties();

    // closure to parse tags from ItemKey enum
    let get = |key: ItemKey| -> Option<String> {
        tag.and_then(|t| t.get(&key))
            .and_then(|i| i.value().text())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    Ok(TrackInfo {
        id: None,
        file_path: path.to_path_buf(),

        // track info
        title: tag.and_then(|t| t.title().map(|s| s.to_string())),
        artist: tag.and_then(|t| t.artist().map(|s| s.to_string())),
        album: tag.and_then(|t| t.album().map(|s| s.to_string())),
        album_artist: get(ItemKey::AlbumArtist),
        // All album-artist values joined, in case the file lists several.
        album_artists: tag.and_then(|t| {
            let joined: Vec<&str> = t.get_strings(&ItemKey::AlbumArtist).collect();
            (!joined.is_empty()).then(|| joined.join("; "))
        }),
        composer: get(ItemKey::Composer),
        label: get(ItemKey::Label),
        genre: tag.and_then(|t| t.genre().map(|s| s.to_string())),
        comment: tag.and_then(|t| t.comment().map(|s| s.to_string())),
        lyrics: get(ItemKey::Lyrics),
        track: tag.and_then(|t| t.track()),
        track_total: tag.and_then(|t| t.track_total()),
        disc: tag.and_then(|t| t.disk()),
        disc_total: tag.and_then(|t: &lofty::tag::Tag| t.disk_total()),
        release_year: tag.and_then(|t| t.year()),
        recording_date: get(ItemKey::RecordingDate),
        original_release_date: get(ItemKey::OriginalReleaseDate),
        release_type: None, // value populated during enrichment
        compilation: tag
            .and_then(|t| t.get(&ItemKey::FlagCompilation))
            .and_then(|i| i.value().text())
            .map(|s| s == "1" || s.to_lowercase() == "true"),
        isrc: get(ItemKey::Isrc),
        barcode: get(ItemKey::Barcode),
        catalog_number: get(ItemKey::CatalogNumber),
        bpm: tag
            .and_then(|t| t.get(&ItemKey::IntegerBpm))
            .and_then(|i| i.value().text())
            .and_then(|s| s.parse::<u32>().ok()),
        language: get(ItemKey::Language),
        script: get(ItemKey::Script),
        mood: get(ItemKey::Mood),
        replay_gain_track_gain: get(ItemKey::ReplayGainTrackGain),
        replay_gain_track_peak: get(ItemKey::ReplayGainTrackPeak),
        replay_gain_album_gain: get(ItemKey::ReplayGainAlbumGain),
        replay_gain_album_peak: get(ItemKey::ReplayGainAlbumPeak),

        // tech properties
        file_format: Some(
            match file_type {
                FileType::Aac => "AAC",
                FileType::Aiff => "AIFF",
                FileType::Ape => "APE",
                FileType::Flac => "FLAC",
                FileType::Mpeg => "MP3",
                FileType::Mp4 => "MP4",
                FileType::Mpc => "MPC",
                FileType::Opus => "OPUS",
                FileType::Vorbis => "OGG",
                FileType::Speex => "SPEEX",
                FileType::Wav => "WAV",
                FileType::WavPack => "WAVPACK",
                _ => "UNKNOWN",
            }
            .to_string(),
        ),
        file_size: std::fs::metadata(path).ok().map(|m| m.len() as i64),
        duration: Some(props.duration().as_millis() as u32),
        bitrate: props.audio_bitrate(),
        sample_rate: props.sample_rate(),
        bit_depth: props.bit_depth().map(|b| b as u32),
        channels: props.channels().map(|c| c as u32),

        // dumb stuff
        acoustid: None,
        musicbrainz_recording_id: get(ItemKey::MusicBrainzRecordingId),
        musicbrainz_track_id: get(ItemKey::MusicBrainzTrackId),
        musicbrainz_release_id: get(ItemKey::MusicBrainzReleaseId),
        musicbrainz_release_group_id: get(ItemKey::MusicBrainzReleaseGroupId),
        musicbrainz_artist_id: get(ItemKey::MusicBrainzArtistId),
        musicbrainz_release_artist_id: get(ItemKey::MusicBrainzReleaseArtistId),
        musicbrainz_work_id: get(ItemKey::MusicBrainzWorkId),

        // pipeline state
        status: "pending".to_string(),

        // file hash
        file_hash: None,
    })
}

pub fn hash_file(path: &PathBuf) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    Some(blake3::hash(&data).to_hex().to_string())
}

/// Editable tag fields, as raw editor strings. An empty string clears the tag.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TagEdits {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub comment: String,
    pub track: String,
    pub disc: String,
    pub year: String,
}

impl TagEdits {
    pub fn opt(s: &str) -> Option<&str> {
        let s = s.trim();
        (!s.is_empty()).then_some(s)
    }
}

/// Writes the edited tags back into the audio file via lofty. Empty fields
/// remove the corresponding tag; numeric fields that don't parse are ignored.
pub fn write_tags(
    path: &Path,
    e: &TagEdits,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use lofty::config::WriteOptions;
    use lofty::tag::Tag;

    let mut tagged = lofty::read_from_path(path)?;
    if tagged.primary_tag().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged.primary_tag_mut().expect("tag inserted above");

    // Text fields: set when non-empty, otherwise clear.
    let mut text = |key: ItemKey, value: &str| {
        let value = value.trim();
        if value.is_empty() {
            tag.remove_key(&key);
        } else {
            tag.insert_text(key, value.to_string());
        }
    };
    text(ItemKey::TrackTitle, &e.title);
    text(ItemKey::TrackArtist, &e.artist);
    text(ItemKey::AlbumTitle, &e.album);
    text(ItemKey::AlbumArtist, &e.album_artist);
    text(ItemKey::Genre, &e.genre);
    text(ItemKey::Comment, &e.comment);

    // Numeric fields.
    let mut num = |key: ItemKey, value: &str| {
        let value = value.trim();
        match value.parse::<u32>() {
            Ok(n) => {
                tag.insert_text(key, n.to_string());
            }
            Err(_) if value.is_empty() => {
                tag.remove_key(&key);
            }
            Err(_) => {}
        }
    };
    num(ItemKey::TrackNumber, &e.track);
    num(ItemKey::DiscNumber, &e.disc);
    num(ItemKey::Year, &e.year);

    tagged.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct TrackInfo {
    pub id: Option<i64>,
    pub file_path: PathBuf,

    // core tags
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub album_artists: Option<String>,
    pub composer: Option<String>,
    pub label: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    pub lyrics: Option<String>,

    // numbering
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub disc: Option<u32>,
    pub disc_total: Option<u32>,

    // dates
    pub release_year: Option<u32>,
    pub recording_date: Option<String>,
    pub original_release_date: Option<String>,

    // release metadata
    pub release_type: Option<String>,
    pub compilation: Option<bool>,
    pub isrc: Option<String>,
    pub barcode: Option<String>,
    pub catalog_number: Option<String>,
    pub bpm: Option<u32>,
    pub language: Option<String>,
    pub script: Option<String>,
    pub mood: Option<String>,

    // replay gain
    pub replay_gain_track_gain: Option<String>,
    pub replay_gain_track_peak: Option<String>,
    pub replay_gain_album_gain: Option<String>,
    pub replay_gain_album_peak: Option<String>,

    // tech properties
    pub file_format: Option<String>,
    pub file_size: Option<i64>,
    pub duration: Option<u32>,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub channels: Option<u32>,

    // external IDs
    pub acoustid: Option<String>,
    pub musicbrainz_recording_id: Option<String>,
    pub musicbrainz_track_id: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    pub musicbrainz_release_group_id: Option<String>,
    pub musicbrainz_artist_id: Option<String>,
    pub musicbrainz_release_artist_id: Option<String>,
    pub musicbrainz_work_id: Option<String>,

    // pipeline state
    pub status: String,

    // file hash
    pub file_hash: Option<String>,
}

impl fmt::Display for TrackInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Artist: {:?}\tAlbum: {:?}\nTitle: {:?}\tYear: {:?}\nGenre: {:?}\nDuration: {:?}\tBitrate: {:?}\n",
            self.artist,
            self.album,
            self.title,
            self.release_year,
            self.genre,
            self.duration,
            self.bitrate
        )
    }
}

#[derive(serde::Serialize)]
pub struct TrackSummary {
    #[serde(skip)]
    pub is_selected: bool,
    pub id: Option<i64>,
    pub isrc: Option<String>,
    pub file_path: PathBuf,

    // core tags
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,

    // tech props
    pub file_format: Option<String>,
    pub file_size: Option<i64>,
    pub duration: Option<u32>,
    pub bitrate: Option<u32>,

    // pipeline state
    pub status: String,
    pub file_hash: Option<String>,
    pub marked_for_deletion: bool,
    pub health_issue: Option<String>,
}

/// How a duplicate group's members were matched together.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DuplicateKind {
    /// Byte-for-byte identical files (same BLAKE3 hash).
    Hash,
    /// Same recording across different files/formats (same ISRC).
    Isrc,
}

impl DuplicateKind {
    pub fn label(self) -> &'static str {
        match self {
            DuplicateKind::Hash => "hash",
            DuplicateKind::Isrc => "isrc",
        }
    }
}

#[derive(Clone)]
pub struct DuplicateGroupSummary {
    pub kind: DuplicateKind,
    /// The shared key: a file hash or an ISRC depending on `kind`.
    pub key: String,
    pub count: u32,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}
