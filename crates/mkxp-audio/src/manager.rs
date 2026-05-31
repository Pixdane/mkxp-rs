use kira::manager::{AudioManager as KiraManager, AudioManagerSettings, DefaultBackend};
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings, StaticSoundHandle};
use kira::sound::PlaybackRate;
use kira::track::{TrackBuilder, TrackHandle};
use kira::tween::Tween;
use std::io::Cursor;

use crate::midi::MidiEngine;
use crate::source::AudioSource;
use crate::types::{Volume, Pitch};
use crate::AudioResult;

/// Main audio manager, mirroring mkxp-z's `Audio` class.
///
/// # Architecture
///
/// All playback goes through kira.  Mixer sub-tracks separate BGM, BGS,
/// and ME channels so they can be controlled independently.  SE sounds are
/// played directly against the main track for automatic concurrency.
///
/// MIDI files are pre-rendered through rustysynth, encoded as WAV, and
/// played as [`StaticSoundData`].
///
/// # Usage
///
/// ```ignore
/// use mkxp_audio::{AudioManager, AudioSource};
/// use mkxp_fs::FileSystem;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let fs = FileSystem::new();
/// let mut audio = AudioManager::new()?;
/// audio.setup_midi("GMGSx.sf2")?;
///
/// let bgm = AudioSource::from_filesystem(&fs, "Audio/BGM/town.ogg")?;
/// audio.bgm_play(&bgm, 80, 100, 0.0)?;
///
/// let se = AudioSource::from_filesystem(&fs, "Audio/SE/sword.ogg")?;
/// audio.se_play(&se, 100, 100)?;
/// # Ok(())
/// # }
/// ```
pub struct AudioManager {
    kira: KiraManager<DefaultBackend>,
    bgm_track: Option<TrackHandle>,
    bgs_track: Option<TrackHandle>,
    me_track: Option<TrackHandle>,
    bgm_handle: Option<StaticSoundHandle>,
    bgs_handle: Option<StaticSoundHandle>,
    me_handle: Option<StaticSoundHandle>,
    se_handles: Vec<StaticSoundHandle>,
    midi: Option<MidiEngine>,
    bgm_volume: i32,
}

impl AudioManager {
    /// Create a new audio manager with the system default audio device.
    ///
    /// ```no_run
    /// use mkxp_audio::AudioManager;
    ///
    /// let audio = AudioManager::new()?;
    /// # Ok::<(), mkxp_audio::AudioError>(())
    /// ```
    pub fn new() -> AudioResult<Self> {
        let kira = KiraManager::new(AudioManagerSettings::default())
            .map_err(|e| crate::AudioError::device(format!("{}", e)))?;
        Ok(Self {
            kira,
            bgm_track: None,
            bgs_track: None,
            me_track: None,
            bgm_handle: None,
            bgs_handle: None,
            me_handle: None,
            se_handles: Vec::new(),
            midi: None,
            bgm_volume: 100,
        })
    }

    /// Initialize the MIDI engine by loading a SoundFont.
    ///
    /// Mirrors mkxp-z `setupMidi()`.  Call once, before playing any MIDI
    /// files.  If `soundfont_path` is empty, the embedded silent default
    /// SoundFont is used — MIDI will play silently but without errors.
    ///
    /// ```no_run
    /// # use mkxp_audio::AudioManager;
    /// let mut audio = AudioManager::new()?;
    /// audio.setup_midi("")?; // empty = embedded fallback
    /// # Ok::<(), mkxp_audio::AudioError>(())
    /// ```
    pub fn setup_midi(&mut self, soundfont_path: &str) -> AudioResult<()> {
        let engine = MidiEngine::new(soundfont_path)?;
        self.midi = Some(engine);
        Ok(())
    }

    // ── internal helpers ───────────────────────────────────────────────

    fn load_static(&self, source: &AudioSource) -> AudioResult<StaticSoundData> {
        if source.is_midi() {
            return Err(crate::AudioError::midi(
                "use bgm_play for MIDI files (they are automatically detected)",
            ));
        }
        StaticSoundData::from_cursor(Cursor::new(source.data.clone()))
            .map_err(|e| crate::AudioError::device(format!("decode: {}", e)))
    }

    fn bgm_track_mut(&mut self) -> AudioResult<&mut TrackHandle> {
        if self.bgm_track.is_none() {
            let track = self
                .kira
                .add_sub_track(TrackBuilder::new())
                .map_err(|e| crate::AudioError::device(format!("bgm track: {}", e)))?;
            self.bgm_track = Some(track);
        }
        Ok(self.bgm_track.as_mut().unwrap())
    }

    fn bgm_track_id(&mut self) -> AudioResult<kira::track::TrackId> {
        Ok(self.bgm_track_mut()?.id())
    }

    fn bgs_track_mut(&mut self) -> AudioResult<&mut TrackHandle> {
        if self.bgs_track.is_none() {
            let track = self
                .kira
                .add_sub_track(TrackBuilder::new())
                .map_err(|e| crate::AudioError::device(format!("bgs track: {}", e)))?;
            self.bgs_track = Some(track);
        }
        Ok(self.bgs_track.as_mut().unwrap())
    }

    fn me_track_mut(&mut self) -> AudioResult<&mut TrackHandle> {
        if self.me_track.is_none() {
            let track = self
                .kira
                .add_sub_track(TrackBuilder::new())
                .map_err(|e| crate::AudioError::device(format!("me track: {}", e)))?;
            self.me_track = Some(track);
        }
        Ok(self.me_track.as_mut().unwrap())
    }

    fn apply_pitch(sound: StaticSoundData, pitch: Pitch) -> StaticSoundData {
        let semitones = pitch.as_multiplier();
        if (semitones - 0.0).abs() > f64::EPSILON {
            sound.with_settings(
                StaticSoundSettings::new().playback_rate(PlaybackRate::Semitones(semitones)),
            )
        } else {
            sound
        }
    }

    // ── BGM ─────────────────────────────────────────────────────────────

    /// Play background music.
    ///
    /// Mirrors mkxp-z `bgmPlay(filename, volume, pitch, pos)`.  If the
    /// source is a MIDI file, it is pre-rendered through the rustysynth
    /// pipeline automatically.
    ///
    /// Stops any currently playing BGM before starting the new track.
    /// Volume and pitch use mkxp-z conventions: 0–100 and 50–150.
    pub fn bgm_play(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
        pos: f64,
    ) -> AudioResult<()> {
        if let Some(mut h) = self.bgm_handle.take() {
            h.stop(Tween::default());
        }

        if source.is_midi() {
            return self.bgm_play_midi(source, volume, pitch, pos);
        }

        let track_id = self.bgm_track_id()?;
        let sound = Self::apply_pitch(self.load_static(source)?, Pitch::new(pitch));
        let settings = StaticSoundSettings::new()
            .output_destination(track_id)
            .loop_region(..);

        let mut handle = self
            .kira
            .play(sound.with_settings(settings))
            .map_err(|e| crate::AudioError::device(format!("bgm play: {}", e)))?;
        handle.set_volume(Volume::new(volume).as_f64(), Tween::default());
        if pos > 0.0 {
            handle.seek_to(pos);
        }
        self.bgm_volume = volume;
        self.bgm_handle = Some(handle);
        Ok(())
    }

    fn bgm_play_midi(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
        pos: f64,
    ) -> AudioResult<()> {
        let engine = self.midi.as_ref().ok_or_else(|| {
            crate::AudioError::midi("no SoundFont loaded (call setup_midi first)")
        })?;
        let synth = engine.create_synthesizer()?;
        let mut cursor = Cursor::new(&source.data);
        let midi = std::sync::Arc::new(
            rustysynth::MidiFile::new(&mut cursor)
                .map_err(|e| crate::AudioError::midi(format!("{:?}", e)))?,
        );
        let mut seq = rustysynth::MidiFileSequencer::new(synth);
        seq.play(&midi, true);

        let block = engine.block_size();
        let mut left = vec![0.0f32; block];
        let mut right = vec![0.0f32; block];
        let mut pcm: Vec<f32> = Vec::new();
        let max_samples = engine.sample_rate() as usize * 300 * 2;

        while !seq.end_of_sequence() && pcm.len() < max_samples {
            seq.render(&mut left, &mut right);
            for (&l, &r) in left.iter().zip(right.iter()) {
                pcm.push(l);
                pcm.push(r);
            }
        }

        let wav = encode_pcm_to_wav(&pcm);
        let sound = StaticSoundData::from_cursor(Cursor::new(wav))
            .map_err(|e| crate::AudioError::device(format!("midi decode: {}", e)))?;
        let sound = Self::apply_pitch(sound, Pitch::new(pitch));

        let track_id = self.bgm_track_id()?;
        let settings = StaticSoundSettings::new()
            .output_destination(track_id)
            .loop_region(..);

        let mut handle = self
            .kira
            .play(sound.with_settings(settings))
            .map_err(|e| crate::AudioError::device(format!("bgm midi: {}", e)))?;
        handle.set_volume(Volume::new(volume).as_f64(), Tween::default());
        if pos > 0.0 {
            handle.seek_to(pos);
        }
        self.bgm_handle = Some(handle);
        Ok(())
    }

    /// Stop background music immediately.
    ///
    /// Mirrors mkxp-z `bgmStop()`.
    pub fn bgm_stop(&mut self) {
        if let Some(mut h) = self.bgm_handle.take() {
            h.stop(Tween::default());
        }
    }

    /// Fade BGM to silence over `time_ms` milliseconds, then stop.
    ///
    /// Mirrors mkxp-z `bgmFade(time)`.
    pub fn bgm_fade(&mut self, time_ms: i32) {
        if let Some(ref mut h) = self.bgm_handle {
            h.stop(Tween {
                duration: std::time::Duration::from_millis(time_ms as u64),
                ..Default::default()
            });
        }
    }

    /// Set BGM volume (0–100).
    ///
    /// Mirrors mkxp-z `bgmSetVolume`.
    pub fn bgm_set_volume(&mut self, volume: i32) {
        self.bgm_volume = volume;
        if let Some(ref mut h) = self.bgm_handle {
            h.set_volume(Volume::new(volume).as_f64(), Tween::default());
        }
    }

    /// Get BGM volume (0–100).
    ///
    /// Mirrors mkxp-z `bgmGetVolume`.
    pub fn bgm_get_volume(&self) -> i32 {
        self.bgm_volume
    }

    /// Get BGM playback position in seconds.
    ///
    /// Mirrors mkxp-z `bgmPos`.  Returns `0.0` if no BGM is playing.
    pub fn bgm_pos(&self) -> f64 {
        self.bgm_handle.as_ref().map_or(0.0, |h| h.position())
    }

    // ── BGS ─────────────────────────────────────────────────────────────

    /// Play background sound (ambient, looping).
    ///
    /// Mirrors mkxp-z `bgsPlay(filename, volume, pitch, pos)`.
    pub fn bgs_play(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
        pos: f64,
    ) -> AudioResult<()> {
        if let Some(mut h) = self.bgs_handle.take() {
            h.stop(Tween::default());
        }
        let track_id = self.bgs_track_mut()?.id();
        let sound = Self::apply_pitch(self.load_static(source)?, Pitch::new(pitch));
        let settings = StaticSoundSettings::new()
            .output_destination(track_id)
            .loop_region(..);
        let mut handle = self
            .kira
            .play(sound.with_settings(settings))
            .map_err(|e| crate::AudioError::device(format!("bgs: {}", e)))?;
        handle.set_volume(Volume::new(volume).as_f64(), Tween::default());
        if pos > 0.0 {
            handle.seek_to(pos);
        }
        self.bgs_handle = Some(handle);
        Ok(())
    }

    /// Stop background sound.
    ///
    /// Mirrors mkxp-z `bgsStop()`.
    pub fn bgs_stop(&mut self) {
        if let Some(mut h) = self.bgs_handle.take() {
            h.stop(Tween::default());
        }
    }

    /// Fade BGS to silence, then stop.
    ///
    /// Mirrors mkxp-z `bgsFade(time)`.
    pub fn bgs_fade(&mut self, time_ms: i32) {
        if let Some(ref mut h) = self.bgs_handle {
            h.stop(Tween {
                duration: std::time::Duration::from_millis(time_ms as u64),
                ..Default::default()
            });
        }
    }

    /// Get BGS playback position in seconds.
    ///
    /// Mirrors mkxp-z `bgsPos`.
    pub fn bgs_pos(&self) -> f64 {
        self.bgs_handle.as_ref().map_or(0.0, |h| h.position())
    }

    // ── ME ──────────────────────────────────────────────────────────────

    /// Play a one-shot music effect (auto-stops, no loop).
    ///
    /// Mirrors mkxp-z `mePlay(filename, volume, pitch)`.
    pub fn me_play(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
    ) -> AudioResult<()> {
        if let Some(mut h) = self.me_handle.take() {
            h.stop(Tween::default());
        }
        let track_id = self.me_track_mut()?.id();
        let sound = Self::apply_pitch(self.load_static(source)?, Pitch::new(pitch));
        let settings = StaticSoundSettings::new().output_destination(track_id);
        let mut handle = self
            .kira
            .play(sound.with_settings(settings))
            .map_err(|e| crate::AudioError::device(format!("me: {}", e)))?;
        handle.set_volume(Volume::new(volume).as_f64(), Tween::default());
        self.me_handle = Some(handle);
        Ok(())
    }

    /// Stop the music effect.
    ///
    /// Mirrors mkxp-z `meStop()`.
    pub fn me_stop(&mut self) {
        if let Some(mut h) = self.me_handle.take() {
            h.stop(Tween::default());
        }
    }

    /// Fade ME to silence, then stop.
    ///
    /// Mirrors mkxp-z `meFade(time)`.
    pub fn me_fade(&mut self, time_ms: i32) {
        if let Some(ref mut h) = self.me_handle {
            h.stop(Tween {
                duration: std::time::Duration::from_millis(time_ms as u64),
                ..Default::default()
            });
        }
    }

    // ── SE ──────────────────────────────────────────────────────────────

    /// Play a sound effect (concurrent with other SEs).
    ///
    /// Mirrors mkxp-z `sePlay(filename, volume, pitch)`.  Multiple SEs
    /// can overlap; kira handles automatic mixing.
    pub fn se_play(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
    ) -> AudioResult<()> {
        let sound = Self::apply_pitch(self.load_static(source)?, Pitch::new(pitch));
        let mut handle = self
            .kira
            .play(sound)
            .map_err(|e| crate::AudioError::device(format!("se: {}", e)))?;
        handle.set_volume(Volume::new(volume).as_f64(), Tween::default());
        self.se_handles.push(handle);
        Ok(())
    }

    /// Stop all sound effects.
    ///
    /// Mirrors mkxp-z `seStop()`.
    pub fn se_stop(&mut self) {
        for mut h in self.se_handles.drain(..) {
            h.stop(Tween::default());
        }
    }

    // ── Reset ───────────────────────────────────────────────────────────

    /// Stop all audio (BGM, BGS, ME, and SE).
    ///
    /// Mirrors mkxp-z `reset()`.
    pub fn reset(&mut self) {
        self.bgm_stop();
        self.bgs_stop();
        self.me_stop();
        self.se_stop();
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Encode interleaved stereo `f32` PCM to a 16-bit WAV in memory.
fn encode_pcm_to_wav(samples: &[f32]) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_size as usize);
    wav.extend(b"RIFF");
    wav.extend(&(36 + data_size).to_le_bytes());
    wav.extend(b"WAVE");
    wav.extend(b"fmt ");
    wav.extend(&16u32.to_le_bytes());
    wav.extend(&1u16.to_le_bytes());
    wav.extend(&2u16.to_le_bytes());
    wav.extend(&44100u32.to_le_bytes());
    wav.extend(&(44100u32 * 4).to_le_bytes());
    wav.extend(&4u16.to_le_bytes());
    wav.extend(&16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(&data_size.to_le_bytes());
    for &s in samples {
        let sample = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        wav.extend(&sample.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_wav_roundtrip_silence() {
        let silence = vec![0.0f32; 44100 * 2]; // 1s stereo silence
        let wav = encode_pcm_to_wav(&silence);
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    #[ignore = "requires audio device"]
    fn new_manager_has_no_handles() {
        let audio = AudioManager::new().unwrap();
        assert!((audio.bgm_pos() - 0.0).abs() < f64::EPSILON);
        assert!((audio.bgs_pos() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    #[ignore = "requires audio device"]
    fn setup_midi_empty_path_works() {
        let mut audio = AudioManager::new().unwrap();
        audio.setup_midi("").expect("embedded SF2");
    }

    #[test]
    #[ignore = "requires audio device"]
    fn reset_does_not_panic() {
        let mut audio = AudioManager::new().unwrap();
        audio.setup_midi("").unwrap();
        audio.reset();
    }

    #[test]
    #[ignore = "requires audio device"]
    fn bgm_volume_clamps() {
        let mut audio = AudioManager::new().unwrap();
        audio.bgm_set_volume(150); // should not panic, just clamp
        audio.bgm_set_volume(-10);
    }
}
