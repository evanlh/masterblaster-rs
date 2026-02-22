//! WAV encoding and decoding for PCM audio.

use crate::FormatError;
use mb_engine::Frame;
use mb_ir::{Sample, SampleData};
use std::io::Write;

// --- Writing ---

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

// --- Reading ---

/// Load a WAV file from raw bytes into a Sample.
pub fn load_wav(data: &[u8], name: &str) -> Result<Sample, FormatError> {
    let header = parse_header(data)?;
    let sample_data = read_pcm_data(data, &header)?;

    let mut sample = Sample::new(name);
    sample.data = sample_data;
    sample.c4_speed = header.sample_rate;
    Ok(sample)
}

struct WavHeader {
    num_channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    data_offset: usize,
    data_size: usize,
}

fn parse_header(data: &[u8]) -> Result<WavHeader, FormatError> {
    if data.len() < 44 {
        return Err(FormatError::UnexpectedEof);
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(FormatError::InvalidHeader);
    }

    let mut pos = 12;
    let mut fmt: Option<(u16, u32, u16)> = None;
    let mut data_chunk: Option<(usize, usize)> = None;

    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = read_u32_le(data, pos + 4) as usize;

        if chunk_id == b"fmt " && chunk_size >= 16 {
            let format = read_u16_le(data, pos + 8);
            if format != 1 {
                return Err(FormatError::UnsupportedVersion);
            }
            let channels = read_u16_le(data, pos + 10);
            let rate = read_u32_le(data, pos + 12);
            let bits = read_u16_le(data, pos + 22);
            fmt = Some((channels, rate, bits));
        } else if chunk_id == b"data" {
            data_chunk = Some((pos + 8, chunk_size));
        }

        pos += 8 + chunk_size;
        if pos % 2 != 0 { pos += 1; }
    }

    let (num_channels, sample_rate, bits_per_sample) = fmt.ok_or(FormatError::InvalidHeader)?;
    let (data_offset, data_size) = data_chunk.ok_or(FormatError::InvalidHeader)?;

    if bits_per_sample != 8 && bits_per_sample != 16 {
        return Err(FormatError::UnsupportedVersion);
    }
    if !(1..=2).contains(&num_channels) {
        return Err(FormatError::UnsupportedVersion);
    }

    Ok(WavHeader { num_channels, sample_rate, bits_per_sample, data_offset, data_size })
}

fn read_pcm_data(data: &[u8], header: &WavHeader) -> Result<SampleData, FormatError> {
    let end = (header.data_offset + header.data_size).min(data.len());
    let raw = &data[header.data_offset..end];

    match (header.bits_per_sample, header.num_channels) {
        (8, 1) => Ok(SampleData::Mono8(read_8bit_mono(raw))),
        (8, 2) => {
            let (l, r) = read_8bit_stereo(raw);
            Ok(SampleData::Stereo8(l, r))
        }
        (16, 1) => Ok(SampleData::Mono16(read_16bit_mono(raw))),
        (16, 2) => {
            let (l, r) = read_16bit_stereo(raw);
            Ok(SampleData::Stereo16(l, r))
        }
        _ => Err(FormatError::UnsupportedVersion),
    }
}

/// Read 8-bit unsigned PCM → signed i8 (WAV 8-bit is unsigned 0-255, center=128).
fn read_8bit_mono(raw: &[u8]) -> Vec<i8> {
    raw.iter().map(|&b| (b as i16 - 128) as i8).collect()
}

fn read_8bit_stereo(raw: &[u8]) -> (Vec<i8>, Vec<i8>) {
    let mut left = Vec::with_capacity(raw.len() / 2);
    let mut right = Vec::with_capacity(raw.len() / 2);
    for chunk in raw.chunks_exact(2) {
        left.push((chunk[0] as i16 - 128) as i8);
        right.push((chunk[1] as i16 - 128) as i8);
    }
    (left, right)
}

fn read_16bit_mono(raw: &[u8]) -> Vec<i16> {
    raw.chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn read_16bit_stereo(raw: &[u8]) -> (Vec<i16>, Vec<i16>) {
    let mut left = Vec::with_capacity(raw.len() / 4);
    let mut right = Vec::with_capacity(raw.len() / 4);
    for chunk in raw.chunks_exact(4) {
        left.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        right.push(i16::from_le_bytes([chunk[2], chunk[3]]));
    }
    (left, right)
}

fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid WAV file from raw parameters.
    fn make_wav(channels: u16, sample_rate: u32, bits: u16, pcm_data: &[u8]) -> Vec<u8> {
        let block_align = channels * (bits / 8);
        let byte_rate = sample_rate * block_align as u32;
        let data_size = pcm_data.len() as u32;
        let file_size = 36 + data_size;

        let mut buf = Vec::new();
        buf.extend(b"RIFF");
        buf.extend(&file_size.to_le_bytes());
        buf.extend(b"WAVE");
        buf.extend(b"fmt ");
        buf.extend(&16u32.to_le_bytes());
        buf.extend(&1u16.to_le_bytes());
        buf.extend(&channels.to_le_bytes());
        buf.extend(&sample_rate.to_le_bytes());
        buf.extend(&byte_rate.to_le_bytes());
        buf.extend(&block_align.to_le_bytes());
        buf.extend(&bits.to_le_bytes());
        buf.extend(b"data");
        buf.extend(&data_size.to_le_bytes());
        buf.extend(pcm_data);
        buf
    }

    #[test]
    fn load_8bit_mono() {
        let wav = make_wav(1, 22050, 8, &[128, 255, 0, 192]);
        let sample = load_wav(&wav, "test").unwrap();
        assert_eq!(sample.c4_speed, 22050);
        match &sample.data {
            SampleData::Mono8(data) => {
                assert_eq!(data, &[0, 127, -128, 64]);
            }
            other => panic!("expected Mono8, got {:?}", other),
        }
    }

    #[test]
    fn load_16bit_mono() {
        let pcm: Vec<u8> = [0i16, 1000, -1000, 32767]
            .iter()
            .flat_map(|&v| v.to_le_bytes())
            .collect();
        let wav = make_wav(1, 44100, 16, &pcm);
        let sample = load_wav(&wav, "test16").unwrap();
        match &sample.data {
            SampleData::Mono16(data) => {
                assert_eq!(data, &[0, 1000, -1000, 32767]);
            }
            other => panic!("expected Mono16, got {:?}", other),
        }
    }

    #[test]
    fn load_16bit_stereo() {
        let pcm: Vec<u8> = [100i16, 200, -100, -200]
            .iter()
            .flat_map(|&v| v.to_le_bytes())
            .collect();
        let wav = make_wav(2, 44100, 16, &pcm);
        let sample = load_wav(&wav, "stereo").unwrap();
        match &sample.data {
            SampleData::Stereo16(l, r) => {
                assert_eq!(l, &[100, -100]);
                assert_eq!(r, &[200, -200]);
            }
            other => panic!("expected Stereo16, got {:?}", other),
        }
    }

    #[test]
    fn invalid_header_rejected() {
        assert!(load_wav(b"not a wav", "bad").is_err());
    }

    #[test]
    fn too_short_rejected() {
        assert!(load_wav(&[0; 10], "short").is_err());
    }
}
