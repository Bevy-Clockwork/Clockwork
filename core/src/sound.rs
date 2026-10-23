//! Sounds: samples a recipe reads from the ROM (many GBA games keep theirs as plain signed 8-bit
//! PCM), played when the game asks for them: a trigger's effect, a character's hurt or pickup.

use serde::{Deserialize, Serialize};

/// How a sound's samples are stored in the ROM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SampleFormat {
    /// Signed 8-bit.
    #[default]
    S8,
    /// Unsigned 8-bit.
    U8,
    /// Signed 16-bit, little-endian.
    S16,
}

/// A sound ready to play: mono samples at a rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Reads samples stored as `format`.
    pub fn decode(bytes: &[u8], format: SampleFormat, rate: u32) -> Self {
        let samples = match format {
            SampleFormat::S8 => bytes.iter().map(|b| (*b as i8 as i16) << 8).collect(),
            SampleFormat::U8 => bytes.iter().map(|b| ((*b as i16) - 128) << 8).collect(),
            SampleFormat::S16 => bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes(*c))
                .collect(),
        };
        Self { rate, samples }
    }

    /// The sound as a WAV file, for players that read files.
    pub fn to_wav(&self) -> Vec<u8> {
        let data = self.samples.len() as u32 * 2;
        let mut out = Vec::with_capacity(44 + data as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&(self.rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for s in &self.samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_bit_samples_become_sixteen_and_a_wav() {
        let pcm = Pcm::decode(&[0x00, 0x7F, 0x80], SampleFormat::S8, 8000);
        assert_eq!(pcm.samples, vec![0, 0x7F00, -0x8000]);
        let unsigned = Pcm::decode(&[0x80, 0xFF], SampleFormat::U8, 8000);
        assert_eq!(unsigned.samples, vec![0, 0x7F00]);
        let wav = pcm.to_wav();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 6);
    }
}
