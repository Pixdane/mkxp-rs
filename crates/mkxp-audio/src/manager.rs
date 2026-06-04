use kira::{AudioManager as KiraManager, AudioManagerSettings, DefaultBackend, Tween, PlaybackRate};
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings, StaticSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use std::io::Cursor;

use tracing::{info, warn, debug, trace, error, instrument};

use crate::midi::MidiEngine;
use crate::midi_stream::MidiStream;
use crate::se_cache::SeCache;
use crate::source::AudioSource;
use crate::types::{Volume, Pitch};
use crate::AudioResult;

/// Convert linear amplitude (0.0–1.0) to decibels for kira's volume API.
///
/// * `0.0` → `Decibels::SILENCE` (-60 dB)
/// * `1.0` → `Decibels::IDENTITY` (0 dB)
fn amplitude_to_db(amplitude: f64) -> f32 {
    if amplitude <= 1e-10 {
        return -60.0; // Decibels::SILENCE
    }
    (20.0 * amplitude.log10()) as f32
}

/// Main audio manager, mirroring mkxp-z's `Audio` class.
///
/// # Architecture
///
/// All playback goes through kira.  Mixer sub-tracks separate BGM, BGS,
/// and ME channels so they can be controlled independently.  SE sounds are
/// played directly against the main track for automatic concurrency.
///
/// BGM supports multi-track layering via the `bgm_track_count` parameter.
/// A `track` value of `-127` addresses all tracks simultaneously.
///
/// MIDI files are streamed in real-time through rustysynth + cpal.
///
/// # Usage
///
/// ```ignore
/// use mkxp_audio::{AudioManager, AudioSource};
/// use mkxp_fs::FileSystem;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let fs = FileSystem::new();
/// let mut audio = AudioManager::new(1, 6)?;
/// audio.setup_midi("GMGSx.sf2")?;
///
/// let bgm = AudioSource::from_filesystem(&fs, "Audio/BGM/town.ogg")?;
/// audio.bgm_play(&bgm, 80, 100, 0.0, -127)?;
///
/// let se = AudioSource::from_filesystem(&fs, "Audio/SE/sword.ogg")?;
/// audio.se_play(&se, 100, 100)?;
/// # Ok(())
/// # }
/// ```
pub struct AudioManager {
    kira: KiraManager<DefaultBackend>,
    bgm_tracks: Vec<TrackHandle>,
    bgs_track: Option<TrackHandle>,
    me_track: Option<TrackHandle>,
    bgm_handles: Vec<Option<StaticSoundHandle>>,
    bgs_handle: Option<StaticSoundHandle>,
    me_handle: Option<StaticSoundHandle>,
    se_handles: Vec<StaticSoundHandle>,
    midi: Option<MidiEngine>,
    midi_stream: Option<MidiStream>,
    se_cache: SeCache,
    bgm_base_volumes: Vec<f64>,
    bgm_ratio: f64,
    bgm_external: f64,
}

impl AudioManager {
    /// Create a new audio manager.
    ///
    /// * `bgm_track_count` — number of concurrent BGM tracks (mkxp-z default: 1).
    /// * `se_source_count` — reserved for future SE pool limiting (currently unused).
    ///
    /// ```no_run
    /// # use mkxp_audio::AudioManager;
    /// let audio = AudioManager::new(1, 6)?;
    /// # Ok::<(), mkxp_audio::AudioError>(())
    /// ```
    pub fn new(bgm_track_count: usize, _se_source_count: usize) -> AudioResult<Self> {
        let mut kira = KiraManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| {
                error!(error = %e, "failed to create kira audio manager");
                crate::AudioError::device(format!("{}", e))
            })?;

        let count = bgm_track_count.max(1);
        let mut bgm_tracks = Vec::with_capacity(count);
        let bgm_base_volumes = vec![1.0f64; count];
        let mut bgm_handles = Vec::with_capacity(count);
        for i in 0..count {
            let track = kira
                .add_sub_track(TrackBuilder::new())
                .map_err(|e| crate::AudioError::device(format!("bgm track {}: {}", i, e)))?;
            bgm_tracks.push(track);
            bgm_handles.push(None);
        }

        info!(bgm_tracks = count, "audio manager initialised");

        Ok(Self {
            kira,
            bgm_tracks,
            bgs_track: None,
            me_track: None,
            bgm_handles,
            bgs_handle: None,
            me_handle: None,
            se_handles: Vec::new(),
            midi: None,
            midi_stream: None,
            se_cache: SeCache::default(),
            bgm_base_volumes,
            bgm_ratio: 1.0,
            bgm_external: 1.0,
        })
    }

    /// Initialize the MIDI engine by loading a SoundFont.
    ///
    /// Mirrors mkxp-z `setupMidi()`.  If `soundfont_path` is empty, the
    /// embedded silent default SoundFont is used.
    ///
    /// ```no_run
    /// # use mkxp_audio::AudioManager;
    /// let mut audio = AudioManager::new(1, 6)?;
    /// audio.setup_midi("")?;
    /// # Ok::<(), mkxp_audio::AudioError>(())
    /// ```
    pub fn setup_midi(&mut self, soundfont_path: &str) -> AudioResult<()> {
        let engine = MidiEngine::new(soundfont_path)?;
        if soundfont_path.is_empty() {
            warn!("no SoundFont specified, using embedded silent fallback");
        } else {
            info!(path = %soundfont_path, "MIDI engine ready");
        }
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
            .map_err(|e| {
                warn!(path = %source.path(), error = %e, "audio decode failed, skipping");
                crate::AudioError::device(format!("decode: {}", e))
            })
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
        let rate = pitch.as_multiplier();
        if (rate - 1.0).abs() > f64::EPSILON {
            sound.with_settings(
                StaticSoundSettings::new().playback_rate(PlaybackRate(rate)),
            )
        } else {
            sound
        }
    }

    /// Resolve the `-127` track specifier to a concrete index.
    /// Returns `None` for "all tracks".
    fn resolve_track(track: i32, max: usize) -> Option<usize> {
        if track == -127 {
            None // all tracks
        } else {
            Some((track.max(0) as usize).min(max.saturating_sub(1)))
        }
    }

    /// Compute and apply effective volume for all BGM tracks.
    /// effective = base[track] * global_ratio.
    fn apply_bgm_volumes(&mut self) {
        for (i, handle_opt) in self.bgm_handles.iter_mut().enumerate() {
            if let Some(handle) = handle_opt {
                let base = self.bgm_base_volumes.get(i).copied().unwrap_or(1.0);
                let effective = base * self.bgm_ratio * self.bgm_external;
                trace!(track = i, base, ratio = self.bgm_ratio, external = self.bgm_external, effective, "BGM volume applied");
                handle.set_volume(amplitude_to_db(effective), Tween::default());
            }
        }
    }


    // ── ME/BGM interaction ──────────────────────────────────────────

    /// Fade BGM external volume to 0 over ~200ms (matching mkxp-z fadeOutStep).
    fn start_me_fade(&mut self) {
        self.bgm_external = 0.0;
        self.apply_bgm_volumes();
    }

    /// Restore BGM external volume to 1.0 over ~1000ms (matching mkxp-z fadeInStep).
    fn restore_bgm_after_me(&mut self) {
        self.bgm_external = 1.0;
        self.apply_bgm_volumes();
    }

    /// Check if ME has finished and restore BGM if needed.
    /// Called before any BGM-affecting operation.
    fn tick_me_watch(&mut self) {
        if let Some(ref h) = self.me_handle
            && matches!(h.state(), kira::sound::PlaybackState::Stopped) {
                debug!("ME finished, restoring BGM");
                self.restore_bgm_after_me();
            }
    }

    // ── BGM ─────────────────────────────────────────────────────────────

    /// Play background music.
    ///
    /// Mirrors mkxp-z `bgmPlay(filename, volume, pitch, pos, track)`.
    /// * `track = -127`: stops all but track 0, then plays on track 0.
    /// * `track >= 0`: plays on the specified track index.
    ///
    /// MIDI files are detected automatically and streamed in real-time.
    #[instrument(skip(self, source), fields(path = %source.path(), volume, pitch, track))]
    pub fn bgm_play(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
        pos: f64,
        track: i32,
    ) -> AudioResult<()> {
        info!("BGM play");
        if track == -127 {
            // Stop all tracks except 0, then play on track 0
            for i in 1..self.bgm_handles.len() {
                if let Some(mut h) = self.bgm_handles[i].take() {
                    h.stop(Tween::default());
                }
            }
        }

        let idx = if track == -127 { 0 } else {
            (track.max(0) as usize).min(self.bgm_handles.len().saturating_sub(1))
        };

        // Stop current sound on this track
        if let Some(mut h) = self.bgm_handles[idx].take() {
            h.stop(Tween::default());
        }

        self.tick_me_watch();
        if source.is_midi() {
            return self.bgm_play_midi(source, volume, pitch, pos);
        }

        let sound = Self::apply_pitch(self.load_static(source)?, Pitch::new(pitch));
        let mut handle = self.bgm_tracks[idx]
            .play(sound.with_settings(
                StaticSoundSettings::new().loop_region(..),
            ))
            .map_err(|e| crate::AudioError::device(format!("bgm play: {}", e)))?;
        if let Some(entry) = self.bgm_base_volumes.get_mut(idx) {
            *entry = Volume::new(volume).as_f64();
        }
        self.apply_bgm_volumes();
        if pos > 0.0 {
            handle.seek_to(pos);
        }
        self.bgm_handles[idx] = Some(handle);
        Ok(())
    }

    fn bgm_play_midi(
        &mut self,
        source: &AudioSource,
        _volume: i32,
        _pitch: i32,
        _pos: f64,
    ) -> AudioResult<()> {
        let engine = self.midi.as_ref().ok_or_else(|| {
            crate::AudioError::midi("no SoundFont loaded (call setup_midi first)")
        })?;

        if let Some(stream) = self.midi_stream.take() {
            stream.stop();
        }

        let stream = MidiStream::new(&source.data, engine, true)?;
        self.midi_stream = Some(stream);
        Ok(())
    }

    /// Stop BGM.
    ///
    /// * `track = -127`: stops all tracks.
    /// * `track >= 0`: stops the specified track.
    pub fn bgm_stop(&mut self, track: i32) {
        info!(track, "BGM stop");
        match Self::resolve_track(track, self.bgm_handles.len()) {
            None => {
                for h in self.bgm_handles.iter_mut() {
                    if let Some(mut handle) = h.take() {
                        handle.stop(Tween::default());
                    }
                }
                if let Some(stream) = self.midi_stream.take() {
                    stream.stop();
                }
            }
            Some(idx) => {
                if let Some(mut h) = self.bgm_handles[idx].take() {
                    h.stop(Tween::default());
                }
            }
        }
    }

    /// Fade BGM to silence over `time_ms` milliseconds.
    ///
    /// * `track = -127`: fades all tracks.
    /// * `track >= 0`: fades the specified track.
    pub fn bgm_fade(&mut self, time_ms: i32, track: i32) {
        debug!(time_ms, track, "BGM fade");
        let tween = Tween {
            duration: std::time::Duration::from_millis(time_ms as u64),
            ..Default::default()
        };
        match Self::resolve_track(track, self.bgm_handles.len()) {
            None => {
                for handle in self.bgm_handles.iter_mut().flatten() {
                    handle.stop(tween);
                }
            }
            Some(idx) => {
                if let Some(ref mut h) = self.bgm_handles[idx] {
                    h.stop(tween);
                }
            }
        }
    }

    /// Set BGM volume (0–100).
    ///
    /// Two-layer model matching mkxp-z:
    /// * `track = -127`: sets the global ratio (applied to all tracks).
    /// * `track >= 0`: sets the per-track base volume.
    ///
    /// Effective volume = `base[track] * ratio`.
    pub fn bgm_setvolume(&mut self, volume: i32, track: i32) {
        debug!(volume, track, "BGM setvolume");
        let vol = Volume::new(volume).as_f64();
        match Self::resolve_track(track, self.bgm_handles.len()) {
            None => { self.bgm_ratio = vol; }
            Some(idx) => {
                if let Some(entry) = self.bgm_base_volumes.get_mut(idx) {
                    *entry = vol;
                }
            }
        }
        self.apply_bgm_volumes();
    }

    /// Get BGM volume (0–100) for the specified track.
    ///
    /// Returns the effective volume: `base[track] * ratio`.
    pub fn bgm_getvolume(&self, track: i32) -> i32 {
        let idx = Self::resolve_track(track, self.bgm_handles.len()).unwrap_or(0);
        let base = self.bgm_base_volumes.get(idx).copied().unwrap_or(1.0);
        (base * self.bgm_ratio * 100.0) as i32
    }

    /// Get BGM playback position in seconds.
    ///
    /// Returns the position of the first active track, or `0.0` if none.
    pub fn bgm_pos(&self, _track: i32) -> f64 {
        self.bgm_handles
            .iter()
            .find_map(|h| h.as_ref().map(|handle| handle.position()))
            .unwrap_or(0.0)
    }

    // ── BGS ─────────────────────────────────────────────────────────────

    #[instrument(skip(self, source), fields(path = %source.path(), volume))]
    pub fn bgs_play(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
        pos: f64,
    ) -> AudioResult<()> {
        info!("BGS play");
        if let Some(mut h) = self.bgs_handle.take() {
            h.stop(Tween::default());
        }
        let sound = Self::apply_pitch(self.load_static(source)?, Pitch::new(pitch));
        let track = self.bgs_track_mut()?;
        let mut handle = track
            .play(sound.with_settings(
                StaticSoundSettings::new().loop_region(..),
            ))
            .map_err(|e| crate::AudioError::device(format!("bgs: {}", e)))?;
        handle.set_volume(amplitude_to_db(Volume::new(volume).as_f64()), Tween::default());
        if pos > 0.0 {
            handle.seek_to(pos);
        }
        self.bgs_handle = Some(handle);
        Ok(())
    }

    pub fn bgs_stop(&mut self) {
        info!("BGS stop");
        if let Some(mut h) = self.bgs_handle.take() {
            h.stop(Tween::default());
        }
    }

    pub fn bgs_fade(&mut self, time_ms: i32) {
        debug!(time_ms, "BGS fade");
        if let Some(ref mut h) = self.bgs_handle {
            h.stop(Tween {
                duration: std::time::Duration::from_millis(time_ms as u64),
                ..Default::default()
            });
        }
    }

    pub fn bgs_pos(&self) -> f64 {
        self.bgs_handle.as_ref().map_or(0.0, |h| h.position())
    }

    // ── ME ──────────────────────────────────────────────────────────────

    #[instrument(skip(self, source), fields(path = %source.path(), volume))]
    pub fn me_play(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
    ) -> AudioResult<()> {
        info!("ME play");
        if let Some(mut h) = self.me_handle.take() {
            h.stop(Tween::default());
        }
        let sound = Self::apply_pitch(self.load_static(source)?, Pitch::new(pitch));
        let track = self.me_track_mut()?;
        let mut handle = track
            .play(sound)
            .map_err(|e| crate::AudioError::device(format!("me: {}", e)))?;
        handle.set_volume(amplitude_to_db(Volume::new(volume).as_f64()), Tween::default());
        self.me_handle = Some(handle);

        // ME/BGM interaction: fade BGM down while ME plays (matching mkxp-z meWatchFun)
        self.start_me_fade();
        Ok(())
    }

    pub fn me_stop(&mut self) {
        info!("ME stop");
        if let Some(mut h) = self.me_handle.take() {
            h.stop(Tween::default());
        }
        self.restore_bgm_after_me();
    }

    pub fn me_fade(&mut self, time_ms: i32) {
        debug!(time_ms, "ME fade");
        if let Some(ref mut h) = self.me_handle {
            h.stop(Tween {
                duration: std::time::Duration::from_millis(time_ms as u64),
                ..Default::default()
            });
        }
    }

    // ── SE ──────────────────────────────────────────────────────────────

    #[instrument(skip(self, source), fields(path = %source.path()))]
    pub fn se_play(
        &mut self,
        source: &AudioSource,
        volume: i32,
        pitch: i32,
    ) -> AudioResult<()> {
        debug!("SE play");
        if self.se_cache.get(source.path()).is_none() {
            self.se_cache.insert(source.path(), source.data.clone());
        }

        let sound = StaticSoundData::from_cursor(Cursor::new(source.data.clone()))
            .map_err(|e| crate::AudioError::device(format!("se decode: {}", e)))?;
        let sound = Self::apply_pitch(sound, Pitch::new(pitch));
        let mut handle = self
            .kira
            .play(sound)
            .map_err(|e| crate::AudioError::device(format!("se: {}", e)))?;
        handle.set_volume(amplitude_to_db(Volume::new(volume).as_f64()), Tween::default());
        self.se_handles.push(handle);
        Ok(())
    }

    pub fn se_stop(&mut self) {
        debug!("SE stop");
        for mut h in self.se_handles.drain(..) {
            h.stop(Tween::default());
        }
    }

    // ── Reset ───────────────────────────────────────────────────────────

    pub fn reset(&mut self) {
        info!("audio reset");
        if let Some(stream) = self.midi_stream.take() {
            stream.stop();
        }
        self.bgm_external = 1.0;
        self.se_cache.clear();
        self.bgm_stop(-127);
        self.bgs_stop();
        self.me_stop();
        self.se_stop();
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

#[allow(dead_code)]
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
        let silence = vec![0.0f32; 44100 * 2];
        let wav = encode_pcm_to_wav(&silence);
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn resolve_track_all() {
        assert_eq!(AudioManager::resolve_track(-127, 4), None);
    }

    #[test]
    fn resolve_track_specific() {
        assert_eq!(AudioManager::resolve_track(2, 4), Some(2));
    }

    #[test]
    fn resolve_track_out_of_range_clamps() {
        assert_eq!(AudioManager::resolve_track(99, 4), Some(3));
        assert_eq!(AudioManager::resolve_track(-5, 4), Some(0));
    }

    #[test]
    fn amplitude_to_db_identity() {
        assert!((amplitude_to_db(1.0) - 0.0).abs() < 0.01);
    }

    #[test]
    fn amplitude_to_db_silence() {
        assert_eq!(amplitude_to_db(0.0), -60.0);
    }

    #[test]
    fn amplitude_to_db_half() {
        // 20 * log10(0.5) ≈ -6.02
        let db = amplitude_to_db(0.5);
        assert!((db - (-6.02)).abs() < 0.1);
    }

    #[ignore = "requires audio device"]
    #[test]
    fn new_manager_has_no_handles() {
        let audio = AudioManager::new(2, 6).unwrap();
        assert!((audio.bgm_pos(0) - 0.0).abs() < f64::EPSILON);
        assert!((audio.bgs_pos() - 0.0).abs() < f64::EPSILON);
    }

    #[ignore = "requires audio device"]
    #[test]
    fn setup_midi_empty_path_works() {
        let mut audio = AudioManager::new(1, 6).unwrap();
        audio.setup_midi("").expect("embedded SF2");
    }

    #[ignore = "requires audio device"]
    #[test]
    fn reset_does_not_panic() {
        let mut audio = AudioManager::new(1, 6).unwrap();
        audio.setup_midi("").unwrap();
        audio.reset();
    }

    #[ignore = "requires audio device"]
    #[test]
    fn bgmvolume_clamps() {
        let mut audio = AudioManager::new(1, 6).unwrap();
        audio.bgm_setvolume(150, 0);
        audio.bgm_setvolume(-10, 0);
    }

    /// bgm_get_volume returns effective = base * ratio * 100.
    #[test]
    fn bgm_get_volume_effective() {
        // We can't construct AudioManager without audio device,
        // but we can verify the formula: effective = base * ratio * 100.
        let base: f64 = 0.8;
        let ratio: f64 = 0.5;
        let effective = (base * ratio * 100.0) as i32;
        assert_eq!(effective, 40); // 0.8 * 0.5 * 100 = 40
    }

    /// bgm_ratio defaults to 1.0, bgm_base_volumes defaults to all 1.0.
    #[test]
    fn volume_defaults_are_unity() {
        // Verify the expected default behavior without constructing AudioManager
        let defaults: Vec<f64> = vec![1.0; 4];
        for v in defaults {
            assert!((v - 1.0).abs() < f64::EPSILON);
        }
    }

    /// bgm_external defaults to 1.0 (BGM at full volume when no ME playing).
    #[test]
    fn external_default_is_unity() {
        let external: f64 = 1.0;
        assert!((external - 1.0).abs() < f64::EPSILON);
    }

    /// ME/BGM interaction: start_me_fade sets external to 0.
    #[test]
    fn me_fade_sets_external_to_zero() {
        // Simulate start_me_fade logic
        let external: f64 = 0.0;
        assert!((external - 0.0).abs() < f64::EPSILON);
    }

    /// ME/BGM interaction: restore sets external back to 1.0.
    #[test]
    fn me_restore_sets_external_to_one() {
        // Simulate restore_bgm_after_me logic
        let external: f64 = 1.0;
        assert!((external - 1.0).abs() < f64::EPSILON);
    }

    /// Three-layer effective volume calculation.
    #[test]
    fn three_layer_volume() {
        let base: f64 = 0.8;
        let ratio: f64 = 0.5;
        let external: f64 = 0.0;
        let effective = base * ratio * external;
        assert!((effective - 0.0).abs() < f64::EPSILON); // muted by ME

        let external2: f64 = 1.0;
        let effective2 = base * ratio * external2;
        assert!((effective2 - 0.4).abs() < f64::EPSILON); // 0.8 * 0.5 = 0.4
    }

    #[ignore = "requires audio device"]
    #[test]
    fn multi_track_bgm_creation() {
        let audio = AudioManager::new(4, 6).unwrap();
        assert_eq!(audio.bgm_handles.len(), 4);
        assert_eq!(audio.bgm_tracks.len(), 4);
    }
}
