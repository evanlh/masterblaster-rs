//! WAV encoding for 16-bit stereo PCM.

use mb_engine::Frame;
use std::io::Write;

pub fn write_wav(w: &mut impl Write, frames: &[Frame], sample_rate: u32) -> std::io::Result<()> {
    let num_channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let block_align = num_channels * (bits_per_sample / 8);
    let data_size = frames.len() as u32 * block_align as u32;

    write_riff_header(w, data_size)?;
    write_fmt_chunk(w, num_channels, sample_rate, block_align, bits_per_sample)?;
    write_data_chunk(w, frames, data_size)
}

pub fn frames_to_wav(frames: &[Frame], sample_rate: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    write_wav(&mut buf, frames, sample_rate).expect("Vec<u8> write cannot fail");
    buf
}

fn write_riff_header(w: &mut impl Write, data_size: u32) -> std::io::Result<()> {
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_size).to_le_bytes())?;
    w.write_all(b"WAVE")
}

fn write_fmt_chunk(
    w: &mut impl Write,
    num_channels: u16,
    sample_rate: u32,
    block_align: u16,
    bits_per_sample: u16,
) -> std::io::Result<()> {
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&num_channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&(sample_rate * block_align as u32).to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&bits_per_sample.to_le_bytes())
}

fn write_data_chunk(
    w: &mut impl Write,
    frames: &[Frame],
    data_size: u32,
) -> std::io::Result<()> {
    w.write_all(b"data")?;
    w.write_all(&data_size.to_le_bytes())?;
    for frame in frames {
        w.write_all(&frame.left.to_le_bytes())?;
        w.write_all(&frame.right.to_le_bytes())?;
    }
    Ok(())
}
