//! mkxp-audio — Audio playback for RPG Maker games (BGM/BGS/ME/SE + MIDI).
//!
//! Maps mkxp-z's four-channel audio model to kira for mixing/output and
//! rustysynth for SoundFont-based MIDI synthesis.  Zero C dependencies.
//!
//! # Quick start
//!
//! ```no_run
//! # use mkxp_audio::AudioManager;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut audio = AudioManager::new()?;
//! audio.setup_midi("")?; // empty = embedded silent SoundFont
//! # Ok(())
//! # }
//! ```
//!
//! # Channels
//!
//! | Channel | Role | Loops | Backing |
//! |---------|------|-------|---------|
//! | BGM | Background Music (OGG/MP3/MIDI) | Yes | kira sub-track |
//! | BGS | Background Sound | Yes | kira sub-track |
//! | ME  | Music Effect (one-shot) | No | kira sub-track |
//! | SE  | Sound Effect (concurrent) | No | kira main track |
//!
//! All volume values are in the range 0–100 (mkxp-z convention).
//! Pitch values are in the range 50–150 where 100 is normal.
//!
//! # MIDI
//!
//! MIDI files are rendered through rustysynth (SoundFont synthesis) then
//! played as standard audio.  A 556-byte embedded SoundFont serves as the
//! silent fallback when no `midi_soundfont` path is configured — matching
//! mkxp-z's behaviour of playing silently without a SoundFont.

mod error;
mod types;
mod source;
mod midi;
mod manager;

pub use error::AudioError;
pub use types::{Volume, Pitch, AudioResult};
pub use source::{AudioSource, AudioFormat};
pub use midi::MidiEngine;
pub use manager::AudioManager;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires audio device"]
    fn audio_manager_creates_and_resets() {
        let mut audio = AudioManager::new().expect("create AudioManager");
        audio.reset(); // should not panic
    }

    #[test]
    #[ignore = "requires audio device"]
    fn audio_manager_setup_midi_empty() {
        let mut audio = AudioManager::new().expect("create AudioManager");
        audio.setup_midi("").expect("embedded SF2 should load");
    }

    #[test]
    fn midi_engine_embedded_default() {
        let engine = MidiEngine::new("").expect("embedded SF2");
        assert_eq!(engine.sample_rate(), 44100);
        assert_eq!(engine.block_size(), 64);
        engine.create_synthesizer().expect("synth should work");
    }

    #[test]
    fn volume_clamps() {
        assert_eq!(Volume::new(150).as_i32(), 100);
        assert_eq!(Volume::new(-10).as_i32(), 0);
        assert_eq!(Volume::new(50).as_i32(), 50);
        assert!((Volume::new(100).as_f64() - 1.0).abs() < f64::EPSILON);
        assert!((Volume::new(0).as_f64() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn pitch_clamps_and_converts() {
        assert_eq!(Pitch::new(200).as_multiplier(), Pitch::new(150).as_multiplier());
        assert_eq!(Pitch::new(0).as_multiplier(), Pitch::new(50).as_multiplier());
        assert!((Pitch::new(100).as_multiplier() - 1.0).abs() < f64::EPSILON);
        assert!((Pitch::new(150).as_multiplier() - 1.5).abs() < f64::EPSILON);
        assert!((Pitch::new(50).as_multiplier() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn audio_format_detection() {
        assert_eq!(AudioFormat::from_extension("ogg"), Some(AudioFormat::Ogg));
        assert_eq!(AudioFormat::from_extension("MP3"), Some(AudioFormat::Mp3));
        assert_eq!(AudioFormat::from_extension("WAV"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::from_extension("flac"), Some(AudioFormat::Flac));
        assert_eq!(AudioFormat::from_extension("mid"), Some(AudioFormat::Midi));
        assert_eq!(AudioFormat::from_extension("MIDI"), Some(AudioFormat::Midi));
        assert_eq!(AudioFormat::from_extension("smf"), Some(AudioFormat::Midi));
        assert_eq!(AudioFormat::from_extension("exe"), None);
        assert_eq!(AudioFormat::from_extension(""), None);
    }
}
