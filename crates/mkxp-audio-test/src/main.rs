//! mkxp-audio integration test: MIDI synthesis → cpal → speakers.
//!
//! To use a real SoundFont, place a .sf2 file next to this crate and run:
//! ```bash
//! cargo run -p mkxp-audio-test
//! ```
//!
//! With no external SF2, the embedded silent default is used.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use mkxp_audio::MidiEngine;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Try to use a real SF2 if present, otherwise fall back to embedded
    let sf_path = if std::path::Path::new("GMGSx.sf2").exists() {
        std::borrow::Cow::Borrowed("GMGSx.sf2")
    } else {
        println!("No GMGSx.sf2 found — using embedded silent SoundFont");
        std::borrow::Cow::Borrowed("")
    };

    let engine = MidiEngine::new(&sf_path)?;
    println!("SoundFont: {}Hz, block={}", engine.sample_rate(), engine.block_size());

    // Build C major scale MIDI in memory
    let midi_bytes = build_midi_c_major_scale();
    let mut cursor = std::io::Cursor::new(&midi_bytes);
    let midi = Arc::new(rustysynth::MidiFile::new(&mut cursor)?);

    let synth = engine.create_synthesizer()?;
    let mut seq = rustysynth::MidiFileSequencer::new(synth);
    seq.play(&midi, false);

    let block = engine.block_size();
    let mut left = vec![0.0f32; block];
    let mut right = vec![0.0f32; block];
    let mut audio: Vec<f32> = Vec::new();
    loop {
        seq.render(&mut left, &mut right);
        for (&l, &r) in left.iter().zip(right.iter()) { audio.push(l); audio.push(r); }
        if seq.end_of_sequence() { break; }
    }
    // Release tail
    for _ in 0..50 {
        seq.render(&mut left, &mut right);
        for (&l, &r) in left.iter().zip(right.iter()) { audio.push(l); audio.push(r); }
    }

    let dur = audio.len() as f64 / 2.0 / 44100.0;
    println!("Rendered {:.1}s", dur);

    // Play via cpal
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio device")?;
    let config = cpal::StreamConfig {
        channels: 2,
        sample_rate: cpal::SampleRate(44100),
        buffer_size: cpal::BufferSize::Default,
    };
    let audio = Arc::new(audio);
    let pos = Arc::new(Mutex::new(0usize));
    let pos_c = pos.clone();
    let audio_c = audio.clone();
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut p = pos_c.lock().unwrap();
            let n = audio_c.len().saturating_sub(*p).min(data.len());
            data[..n].copy_from_slice(&audio_c[*p..*p + n]);
            for s in &mut data[n..] { *s = 0.0; }
            *p += n;
        },
        |e| eprintln!("cpal: {}", e),
        None,
    )?;
    println!("Playing...");
    stream.play()?;
    while *pos.lock().unwrap() < audio.len() { thread::sleep(Duration::from_millis(50)); }
    thread::sleep(Duration::from_millis(500));
    drop(stream);
    println!("Done.");
    Ok(())
}

fn build_midi_c_major_scale() -> Vec<u8> {
    let mut m = Vec::new();
    let div: u16 = 480;
    m.extend(b"MThd"); m.extend(&[0,0,0,6]); m.extend(&[0,0]); m.extend(&[0,1]); m.extend(&div.to_be_bytes());
    let notes: &[u8] = &[60,62,64,65,67,69,71,72];
    let gate = (div * 3 / 4) as u16;
    let rest = div - gate;
    let mut t = Vec::new();
    for &k in notes {
        write_varint(&mut t, 0); t.extend(&[0x90, k, 100]);
        write_varint(&mut t, gate); t.extend(&[0x80, k, 0]);
    }
    write_varint(&mut t, rest); t.extend(&[0xFF, 0x2F, 0x00]);
    m.extend(b"MTrk"); m.extend(&(t.len() as u32).to_be_bytes()); m.extend(&t);
    m
}
fn write_varint(b: &mut Vec<u8>, mut v: u16) {
    let mut bytes = vec![(v & 0x7F) as u8]; v >>= 7;
    while v > 0 { bytes.push((v & 0x7F | 0x80) as u8); v >>= 7; }
    bytes.reverse(); b.extend(&bytes);
}
