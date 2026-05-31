use mkxp_types::MkxpError;

/// Errors specific to the audio subsystem.
///
/// All variants follow the three-layer error model:
/// shared vocabulary (`MkxpError`) → crate-specific enum → `anyhow` at the binary layer.
///
/// # Examples
///
/// ```
/// use mkxp_audio::AudioError;
///
/// let err = AudioError::file_not_found("Audio/BGM/battle.ogg");
/// assert!(err.to_string().contains("battle.ogg"));
/// ```
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// The requested audio file was not found in the virtual file system.
    #[error("audio file not found: {path}")]
    FileNotFound { path: String },

    /// The file extension is not a recognized audio format.
    #[error("unsupported audio format: {path}")]
    UnsupportedFormat { path: String },

    /// MIDI parsing or synthesis failed.
    #[error("MIDI error: {reason}")]
    Midi { reason: String },

    /// SoundFont loading or parsing failed.
    #[error("SoundFont error: {reason}")]
    SoundFont { reason: String },

    /// The audio device (kira/cpal backend) reported an error.
    #[error("audio device error: {reason}")]
    Device { reason: String },

    /// Transparent pass-through for shared error vocabulary.
    #[error(transparent)]
    Mkxp(#[from] MkxpError),
}

impl AudioError {
    /// Shorthand for a file-not-found error.
    pub fn file_not_found(path: impl Into<String>) -> Self {
        Self::FileNotFound { path: path.into() }
    }

    /// Shorthand for an unsupported format error.
    pub fn unsupported(path: impl Into<String>) -> Self {
        Self::UnsupportedFormat { path: path.into() }
    }

    /// Shorthand for a MIDI processing error.
    pub fn midi(reason: impl Into<String>) -> Self {
        Self::Midi { reason: reason.into() }
    }

    /// Shorthand for a SoundFont loading error.
    pub fn soundfont(reason: impl Into<String>) -> Self {
        Self::SoundFont { reason: reason.into() }
    }

    /// Shorthand for an audio device error.
    pub fn device(reason: impl Into<String>) -> Self {
        Self::Device { reason: reason.into() }
    }
}
