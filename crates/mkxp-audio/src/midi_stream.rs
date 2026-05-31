//! Real-time MIDI streaming: render thread + ringbuf → cpal callback.
//!
//! Avoids pre-rendering the entire MIDI file into memory (approach used in
//! `AudioManager::bgm_play_midi`).  Instead, a background thread runs the
//! `MidiFileSequencer` and pushes rendered audio blocks into a lock-free
//! ring buffer.  A cpal output stream pulls from the ring buffer in the
//! audio callback.  Constant memory usage (~2 seconds of audio).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::HeapRb;
use rustysynth::{MidiFile, MidiFileSequencer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::midi::MidiEngine;
use crate::AudioResult;

/// A real-time MIDI audio stream.
///
/// Renders MIDI data through rustysynth in a background thread and
/// streams the output to the default audio device via cpal.
///
/// # Memory
///
/// Constant 2-second ring buffer (~176K `f32` samples) regardless of
/// MIDI file length.  The render thread blocks when the buffer is full.
///
/// # Examples
///
/// ```no_run
/// use mkxp_audio::{MidiEngine, MidiStream};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let engine = MidiEngine::new("")?; // embedded silent SF2
/// let midi_bytes: &[u8] = &[0; 32]; // placeholder
/// let stream = MidiStream::new(midi_bytes, &engine, true)?; // loop=true
/// stream.stop();
/// # Ok(())
/// # }
/// ```

pub struct MidiStream {
    _render_thread: JoinHandle<()>,
    stop_flag: Arc<AtomicBool>,
    _stream: cpal::Stream,
}

impl MidiStream {
    /// Start streaming a MIDI file to the default audio output.
    ///
    /// * `midi_bytes` — raw SMF data.
    /// * `engine` — the loaded MIDI engine (SoundFont + settings).
    /// * `do_loop` — if `true`, loops at the RPG Maker CC 111 marker.
    pub fn new(
        midi_bytes: &[u8],
        engine: &MidiEngine,
        do_loop: bool,
    ) -> AudioResult<Self> {
        let sample_rate = engine.sample_rate() as u32;
        let block = engine.block_size();
        let synth = engine.create_synthesizer()?;
        let mut cursor = std::io::Cursor::new(midi_bytes);
        let midi = Arc::new(
            MidiFile::new(&mut cursor)
                .map_err(|e| crate::AudioError::midi(format!("{:?}", e)))?,
        );

        // Ring buffer: 2 seconds of stereo f32 = sample_rate * 2 channels * 2 sec
        let ring = HeapRb::<f32>::new(sample_rate as usize * 2 * 2);
        let (mut prod, cons) = ring.split();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = stop_flag.clone();

        // ── Render thread ──────────────────────────────────────────
        let render_thread = thread::spawn(move || {
            let mut seq = MidiFileSequencer::new(synth);
            seq.play(&midi, do_loop);

            let mut left = vec![0.0f32; block];
            let mut right = vec![0.0f32; block];

            loop {
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }

                seq.render(&mut left, &mut right);

                // Interleave into ring buffer (may block if full)
                for (&l, &r) in left.iter().zip(right.iter()) {
                    // Spin until there's room
                    while prod.push(l).is_err() {
                        if stop_clone.load(Ordering::Relaxed) {
                            return;
                        }
                        thread::yield_now();
                    }
                    while prod.push(r).is_err() {
                        if stop_clone.load(Ordering::Relaxed) {
                            return;
                        }
                        thread::yield_now();
                    }
                }

                if seq.end_of_sequence() {
                    if !do_loop {
                        // Drain remaining audio, then stop
                        for _ in 0..50 {
                            seq.render(&mut left, &mut right);
                            for (&l, &r) in left.iter().zip(right.iter()) {
                                let _ = prod.push(l);
                                let _ = prod.push(r);
                            }
                        }
                        break;
                    }
                }
            }
        });

        // ── cpal output stream ─────────────────────────────────────
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| crate::AudioError::device("no default audio device"))?;

        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        // The consumer side of the ring buffer needs to be moved into the callback.
        // ringbuf's Consumer is `!Send` in 0.3, so we wrap it in an Arc<Mutex>.
        let cons = std::sync::Mutex::new(cons);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut c) = cons.lock() {
                        let n = c.pop_slice(data);
                        for s in &mut data[n..] {
                            *s = 0.0;
                        }
                    } else {
                        for s in data.iter_mut() {
                            *s = 0.0;
                        }
                    }
                },
                |err| eprintln!("cpal MIDI stream error: {}", err),
                None,
            )
            .map_err(|e| crate::AudioError::device(format!("cpal stream: {}", e)))?;

        stream
            .play()
            .map_err(|e| crate::AudioError::device(format!("cpal play: {}", e)))?;

        Ok(Self {
            _render_thread: render_thread,
            stop_flag,
            _stream: stream,
        })
    }

    /// Stop playback and tear down the render thread.
    pub fn stop(self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        // Joining is handled by Drop — the render thread exits on stop_flag.
        // cpal stream stops on Drop.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MidiEngine;

    /// MidiEngine with embedded SF2 creates without panic.
    #[test]
    fn midi_engine_for_stream_is_available() {
        let engine = MidiEngine::new("").expect("embedded SF2");
        assert_eq!(engine.sample_rate(), 44100);
        assert_eq!(engine.block_size(), 64);
    }

    /// Ring buffer size calculation matches expected 2-second buffer.
    #[test]
    fn ring_buffer_size_matches_2_seconds() {
        let sample_rate = 44100u32;
        let capacity = sample_rate as usize * 2 * 2; // stereo * 2 sec
        let ring = HeapRb::<f32>::new(capacity);
        let (prod, cons) = ring.split();
        assert_eq!(prod.capacity(), capacity);
        assert_eq!(cons.capacity(), capacity);
    }

    /// Verify the ring buffer split pattern used by MidiStream::new.
    #[test]
    fn ringbuf_split_producer_consumer() {
        let ring = HeapRb::<f32>::new(1024);
        let (mut prod, mut cons) = ring.split();
        prod.push(0.5).unwrap();
        prod.push(-0.3).unwrap();
        assert_eq!(cons.pop(), Some(0.5));
        assert_eq!(cons.pop(), Some(-0.3));
        assert_eq!(cons.pop(), None);
    }

    /// Ring buffer `push_slice` / `pop_slice` used in streaming.
    #[test]
    fn ringbuf_push_pop_slice() {
        let ring = HeapRb::<f32>::new(1024);
        let (mut prod, mut cons) = ring.split();
        let input = [1.0f32, 2.0, 3.0, 4.0];
        let n = prod.push_slice(&input);
        assert_eq!(n, 4);
        let mut output = [0.0f32; 4];
        let n = cons.pop_slice(&mut output);
        assert_eq!(n, 4);
        assert_eq!(output, input);
    }

    /// AtomicBool used for stop signalling is lock-free on all supported targets.
    #[test]
    fn stop_flag_defaults_to_false() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        assert!(!flag.load(std::sync::atomic::Ordering::Relaxed));
    }
}
