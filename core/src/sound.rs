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

/// A piece of music ready to play: interleaved stereo samples (left, right) at a rate, played
/// once from the start and then from `loop_start` (a frame: one left and right pair) forever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Music {
    pub rate: u32,
    pub samples: Vec<i16>,
    /// Where it repeats from when it reaches its end; it ends without.
    pub loop_start: Option<usize>,
}

impl Music {
    /// How many frames (left and right pairs) it has.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Frames `range` as a stereo WAV file, for players that read files.
    pub fn wav(&self, range: std::ops::Range<usize>) -> Vec<u8> {
        let samples = &self.samples[range.start * 2..range.end * 2];
        let data = samples.len() as u32 * 2;
        let mut out = Vec::with_capacity(44 + data as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&2u16.to_le_bytes()); // stereo
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&(self.rate * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    /// The part played once (up to the loop, or all of it) and the part repeated after it.
    pub fn parts(&self) -> (std::ops::Range<usize>, Option<std::ops::Range<usize>>) {
        match self.loop_start {
            Some(at) if at < self.frames() => (0..at, Some(at..self.frames())),
            _ => (0..self.frames(), None),
        }
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

    #[test]
    fn music_splits_into_its_intro_and_its_loop() {
        let music = Music {
            rate: 8000,
            samples: vec![0; 20],
            loop_start: Some(4),
        };
        assert_eq!(music.parts(), (0..4, Some(4..10)));
        assert_eq!(music.wav(4..10).len(), 44 + 6 * 4);
        let once = Music {
            loop_start: None,
            ..music
        };
        assert_eq!(once.parts(), (0..10, None));
    }
}
