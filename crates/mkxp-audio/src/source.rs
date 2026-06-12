use crate::AudioResult;

/// Recognized audio formats, detected from file extension.
///
/// # Examples
///
/// ```
/// use mkxp_audio::AudioFormat;
///
/// assert_eq!(AudioFormat::from_extension("ogg"), Some(AudioFormat::Ogg));
/// assert_eq!(AudioFormat::from_extension("MID"), Some(AudioFormat::Midi));
/// assert_eq!(AudioFormat::from_extension("exe"), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// OGG Vorbis (`.ogg`).
    Ogg,
    /// MPEG audio (`.mp3`).
    Mp3,
    /// WAV / RIFF (`.wav`, `.wave`).
    Wav,
    /// FLAC (`.flac`).
    Flac,
    /// Standard MIDI file (`.mid`, `.midi`, `.smf`).
    Midi,
}

impl AudioFormat {
    /// Detect the audio format from a lowercase file extension.
    ///
    /// Returns `None` if the extension is not recognised.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "ogg" => Some(Self::Ogg),
            "mp3" => Some(Self::Mp3),
            "wav" | "wave" => Some(Self::Wav),
            "flac" => Some(Self::Flac),
            "mid" | "midi" | "smf" => Some(Self::Midi),
            _ => None,
        }
    }
}

/// Loaded audio data with format metadata.
///
/// Wraps raw file bytes together with the detected [`AudioFormat`] and
/// the original virtual path for error messages.
///
/// # Examples
///
/// ```no_run
/// use mkxp_fs::FileSystem;
/// use mkxp_audio::AudioSource;
///
/// let fs = FileSystem::new();
/// // Load an OGG file from the virtual filesystem:
/// let source = AudioSource::from_filesystem(&fs, "Audio/BGM/town.ogg")?;
/// assert!(matches!(source.format(), mkxp_audio::AudioFormat::Ogg));
/// # Ok::<(), mkxp_audio::AudioError>(())
/// ```
pub struct AudioSource {
    pub(crate) data: Vec<u8>,
    pub(crate) format: AudioFormat,
    pub(crate) path: String,
}

impl AudioSource {
    /// Load audio data from the virtual [`mkxp_fs::FileSystem`].
    ///
    /// The format is automatically detected from the file extension.
    /// Returns [`crate::AudioError::FileNotFound`] if the path does not exist,
    /// or [`crate::AudioError::UnsupportedFormat`] if the extension is unknown.
    pub fn from_filesystem(fs: &mkxp_fs::FileSystem, path: &str) -> AudioResult<Self> {
        let data = fs
            .read(path)
            .map_err(|_| crate::AudioError::file_not_found(path))?;
        let ext = path.rsplit('.').next().unwrap_or("");
        let format =
            AudioFormat::from_extension(ext).ok_or_else(|| crate::AudioError::unsupported(path))?;
        Ok(Self {
            data,
            format,
            path: path.to_string(),
        })
    }

    /// Returns `true` if this source is a MIDI file.
    ///
    /// MIDI files must be played through the MIDI engine, not as raw audio.
    pub fn is_midi(&self) -> bool {
        matches!(self.format, AudioFormat::Midi)
    }

    /// The detected audio format.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// The original file path (for error reporting).
    pub fn path(&self) -> &str {
        &self.path
    }
}
