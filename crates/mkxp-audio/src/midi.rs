use crate::AudioResult;
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::io::Cursor;
use std::sync::Arc;

/// A minimal valid SoundFont 2.04 binary, embedded as a fallback default.
///
/// Contains 1 preset (Piano, silent), 1 instrument, and 48 samples of
/// silence.  All synthesizer operations work normally but produce no
/// audible output unless a real SoundFont is loaded — matching mkxp-z's
/// behaviour when `midiSoundFont` is empty.
const DEFAULT_SOUNDFONT: &[u8] = include_bytes!("default.sf2");

/// MIDI synthesis engine backed by rustysynth (pure Rust SoundFont synthesizer).
///
/// Loads a SoundFont once and creates [`Synthesizer`] instances for
/// individual MIDI tracks on demand.
///
/// # Examples
///
/// ```no_run
/// use mkxp_audio::MidiEngine;
///
/// // Load a real SoundFont:
/// let engine = MidiEngine::new("GMGSx.sf2")?;
/// let synth = engine.create_synthesizer()?;
/// assert_eq!(engine.sample_rate(), 44100);
/// # Ok::<(), mkxp_audio::AudioError>(())
/// ```
///
/// ```no_run
/// use mkxp_audio::MidiEngine;
///
/// // Empty path → embedded silent default:
/// let engine = MidiEngine::new("")?;
/// println!("Using fallback SF2 ({} Hz, block={})", engine.sample_rate(), engine.block_size());
/// # Ok::<(), mkxp_audio::AudioError>(())
/// ```
pub struct MidiEngine {
    sound_font: Arc<SoundFont>,
    settings: SynthesizerSettings,
}

impl MidiEngine {
    /// Create a MIDI engine from a SoundFont file path.
    ///
    /// If `path` is an empty string, the embedded silent default SoundFont
    /// is used instead, ensuring MIDI playback never crashes due to a
    /// missing SoundFont.
    pub fn new(path: &str) -> AudioResult<Self> {
        let sf = if path.is_empty() {
            Self::load_embedded()
        } else {
            Self::load_file(path)
        }?;
        let settings = SynthesizerSettings::new(44100);
        Ok(Self {
            sound_font: Arc::new(sf),
            settings,
        })
    }

    fn load_file(path: &str) -> AudioResult<SoundFont> {
        let file = std::fs::File::open(path)
            .map_err(|e| crate::AudioError::soundfont(format!("cannot open {}: {}", path, e)))?;
        let mut reader = std::io::BufReader::new(file);
        SoundFont::new(&mut reader)
            .map_err(|e| crate::AudioError::soundfont(format!("parse error: {:?}", e)))
    }

    fn load_embedded() -> AudioResult<SoundFont> {
        SoundFont::new(&mut Cursor::new(DEFAULT_SOUNDFONT))
            .map_err(|e| crate::AudioError::soundfont(format!("embedded SF2: {:?}", e)))
    }

    /// Create a new [`Synthesizer`] from the loaded SoundFont.
    ///
    /// Each MIDI track should get its own synthesizer instance.
    pub fn create_synthesizer(&self) -> AudioResult<Synthesizer> {
        Synthesizer::new(&self.sound_font, &self.settings)
            .map_err(|e| crate::AudioError::midi(format!("synth error: {:?}", e)))
    }

    /// The sample rate used for synthesis (44100 Hz).
    pub fn sample_rate(&self) -> i32 {
        self.settings.sample_rate
    }

    /// The block size for waveform rendering (default: 64 samples).
    pub fn block_size(&self) -> usize {
        self.settings.block_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_sf2_loads() {
        let engine = MidiEngine::new("").expect("embedded default SF2");
        assert_eq!(engine.sample_rate(), 44100);
        assert_eq!(engine.block_size(), 64);
    }

    #[test]
    fn embedded_sf2_creates_synthesizer() {
        let engine = MidiEngine::new("").unwrap();
        let synth = engine.create_synthesizer().expect("synthesizer");
        // Just verify it doesn't panic — the synthesizer is functional.
        drop(synth);
    }
}
