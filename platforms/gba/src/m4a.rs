//! The m4a sound engine (MusicPlayer2000, "Sappy"), which most GBA games play their music with.
//! A song is a header naming its voice group (the instruments) and its tracks; a track is a
//! stream of commands: notes, waits, voice, volume, pan and pitch changes, and jumps. A voice
//! plays a sample, a square or wave channel or noise, or picks another voice by key (key splits
//! and drum kits). This reads a song and renders it to stereo samples, finding where it loops.
//! No game offsets: a recipe names a song's address, and [`find_song_table`] looks for the
//! table of them.

use std::collections::HashMap;

use clockwork_core::sound::Music;

type Result<T> = std::result::Result<T, String>;

/// Where the ROM starts in the GBA's memory: pointers in the ROM count from here.
const ROM: u32 = 0x0800_0000;
/// Ticks in a beat (a quarter note).
const TICKS_PER_BEAT: f64 = 24.0;
/// Envelopes step once a frame of the GBA's screen.
const FRAME_RATE: f64 = 59.7275;
/// The engine's tempo until a song sets one, in beats a minute.
const TEMPO: f64 = 150.0;
/// The notes a song can sound at once from samples (the square, wave and noise channels are one
/// note each).
const SAMPLE_CHANNELS: usize = 12;
/// Songs end here even if they never do.
const LONGEST: f64 = 600.0;

/// Ticks of the wait commands 0x80..=0xB0 and the notes 0xD0..=0xFF (from index 1).
const LENGTHS: [u8; 49] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 28,
    30, 32, 36, 40, 42, 44, 48, 52, 54, 56, 60, 64, 66, 68, 72, 76, 78, 80, 84, 88, 90, 92, 96,
];

fn u32_at(rom: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(rom.get(at..at + 4)?.try_into().ok()?))
}

/// The ROM offset a GBA pointer at `at` points to, if it points into the ROM.
fn pointer(rom: &[u8], at: usize) -> Option<usize> {
    let p = u32_at(rom, at)?;
    let offset = p.checked_sub(ROM)? as usize;
    (offset < rom.len()).then_some(offset)
}

/// A song's header: its voice group and where its tracks start, as ROM offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub voices: usize,
    pub tracks: Vec<usize>,
}

impl Header {
    /// The header at ROM offset `at`.
    pub fn read(rom: &[u8], at: usize) -> Result<Self> {
        let count = *rom
            .get(at)
            .ok_or("song header is past the end of the ROM")? as usize;
        if count > 16 {
            return Err(format!("a song has at most 16 tracks, not {count}"));
        }
        let voices = pointer(rom, at + 4).ok_or("song's voice group pointer is not in the ROM")?;
        let tracks = (0..count)
            .map(|i| {
                pointer(rom, at + 8 + i * 4)
                    .ok_or_else(|| format!("song's track {i} pointer is not in the ROM"))
            })
            .collect::<Result<_>>()?;
        Ok(Self { voices, tracks })
    }
}

/// The song table: where it is and the song header each entry points to, by ROM offset. Its
/// entries are 8 bytes, a pointer to a song and the player and priority that play it; the
/// longest run of such entries in the ROM is taken to be the table.
pub fn find_song_table(rom: &[u8]) -> Option<(usize, Vec<usize>)> {
    let entry = |at: usize| -> Option<usize> {
        let song = pointer(rom, at)?;
        let player = u16::from_le_bytes(rom.get(at + 4..at + 6)?.try_into().ok()?);
        let other = u16::from_le_bytes(rom.get(at + 6..at + 8)?.try_into().ok()?);
        if player >= 32 || other >= 32 || song % 4 != 0 {
            return None;
        }
        Header::read(rom, song).ok()?;
        Some(song)
    };
    let mut best: Option<(usize, Vec<usize>)> = None;
    for start in (0..rom.len().saturating_sub(8)).step_by(4) {
        // Only where a run begins.
        if start >= 8 && entry(start - 8).is_some() {
            continue;
        }
        let mut songs = Vec::new();
        let mut at = start;
        while let Some(song) = entry(at) {
            songs.push(song);
            at += 8;
        }
        let real = songs
            .iter()
            .filter(|s| rom.get(**s).is_some_and(|n| *n > 0))
            .count();
        if real >= 2 && best.as_ref().is_none_or(|(_, b)| songs.len() > b.len()) {
            best = Some((start, songs));
        }
    }
    best
}

/// What a voice makes sound.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Source {
    /// Signed 8-bit samples at ROM offset `data`, `rate` per second at middle C.
    Sample {
        data: usize,
        len: usize,
        looped: Option<usize>,
        rate: f64,
        fixed: bool,
    },
    /// A square wave: duty 0..=3 is 12.5%, 25%, 50% or 75% high.
    Square { duty: u8, second: bool },
    /// 32 steps of 4 bits, at ROM offset `table`.
    Wave { table: usize },
    /// The noise channel: a short (7-bit) or long (15-bit) sequence.
    Noise { short: bool },
}

impl Source {
    /// The sound channels a GBA has one of: a new note there stops the one playing.
    fn psg(&self) -> Option<u8> {
        match self {
            Source::Sample { .. } => None,
            Source::Square { second, .. } => Some(*second as u8),
            Source::Wave { .. } => Some(2),
            Source::Noise { .. } => Some(3),
        }
    }
}

/// A playable instrument: its sound, envelope and pan, at the key it plays.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Voice {
    source: Source,
    /// Attack, decay, sustain, release.
    adsr: [u8; 4],
    /// The key a drum plays at, instead of the note's.
    key: Option<u8>,
    pan: Option<i8>,
}

/// The voice of group `group` number `index` for `key`, following key splits and drum kits.
fn voice(rom: &[u8], group: usize, index: u8, key: u8, depth: u8) -> Result<Voice> {
    let at = group + index as usize * 12;
    let v = rom
        .get(at..at + 12)
        .ok_or_else(|| format!("voice {index} is past the end of the ROM"))?;
    let kind = v[0];
    let adsr = [v[8], v[9], v[10], v[11]];
    let pan = (v[3] & 0x80 != 0).then(|| (v[3] & 0x7F) as i8 - 64);
    let source = match kind {
        0x40 | 0x80 if depth > 0 => return Err("a key split or drum kit inside another".into()),
        0x40 => {
            let sub = pointer(rom, at + 4).ok_or("key split's voice group is not in the ROM")?;
            let table = pointer(rom, at + 8).ok_or("key split's table is not in the ROM")?;
            let pick = *rom
                .get(table + key as usize)
                .ok_or("key split's table is past the end of the ROM")?;
            return voice(rom, sub, pick, key, depth + 1);
        }
        0x80 => {
            let sub = pointer(rom, at + 4).ok_or("drum kit's voice group is not in the ROM")?;
            let mut drum = voice(rom, sub, key, key, depth + 1)?;
            let sub_at = sub + key as usize * 12;
            drum.key = Some(rom[sub_at + 1]);
            return Ok(drum);
        }
        _ => match kind & 0x07 {
            0 => {
                let header = pointer(rom, at + 4).ok_or("sample is not in the ROM")?;
                let flags = u32_at(rom, header).ok_or("sample header is past the end")?;
                let rate = u32_at(rom, header + 4).ok_or("sample header is past the end")?;
                let loop_start = u32_at(rom, header + 8).ok_or("sample header is past the end")?;
                let len = u32_at(rom, header + 12).ok_or("sample header is past the end")? as usize;
                let data = header + 16;
                let len = len.min(rom.len().saturating_sub(data));
                Source::Sample {
                    data,
                    len,
                    looped: (flags & 0x4000_0000 != 0 && (loop_start as usize) < len)
                        .then_some(loop_start as usize),
                    rate: rate as f64 / 1024.0,
                    fixed: kind & 0x08 != 0,
                }
            }
            1 | 2 => Source::Square {
                duty: (u32_at(rom, at + 4).unwrap_or(2) & 3) as u8,
                second: kind & 0x07 == 2,
            },
            3 => Source::Wave {
                table: pointer(rom, at + 4).ok_or("wave table is not in the ROM")?,
            },
            4 => Source::Noise {
                short: u32_at(rom, at + 4).unwrap_or(0) & 1 != 0,
            },
            _ => return Err(format!("voice {index} is of unknown kind 0x{kind:02X}")),
        },
    };
    Ok(Voice {
        source,
        adsr,
        key: None,
        pan,
    })
}

/// Where a channel's envelope is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}

/// A note sounding.
struct Channel {
    track: usize,
    /// The key the note was played with (an end of tie stops notes of it).
    note: u8,
    /// The key it sounds at.
    key: u8,
    voice: Voice,
    velocity: f64,
    /// Ticks until it is let go, if it isn't tied.
    left: Option<u32>,
    stage: Stage,
    /// 0..=1.
    level: f64,
    pos: f64,
    step: f64,
    lfsr: u16,
    started: u64,
}

impl Channel {
    /// One frame of the envelope: false once it is silent.
    fn envelope(&mut self) -> bool {
        let [attack, decay, sustain, release] = self.voice.adsr;
        if self.voice.source.psg().is_some() {
            // The square, wave and noise channels count in 16 levels, each step taking a
            // number of frames.
            let per = |frames: u8| 1.0 / (15.0 * frames.max(1) as f64);
            let sustain = (sustain & 0x0F) as f64 / 15.0;
            match self.stage {
                Stage::Attack if attack == 0 => self.level = 1.0,
                Stage::Attack => self.level += per(attack),
                Stage::Decay if decay == 0 => self.level = sustain,
                Stage::Decay => self.level -= per(decay),
                Stage::Sustain => {}
                Stage::Release if release == 0 => self.level = 0.0,
                Stage::Release => self.level -= per(release),
            }
            match self.stage {
                Stage::Attack if self.level >= 1.0 => {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
                Stage::Decay if self.level <= sustain => {
                    self.level = sustain;
                    self.stage = Stage::Sustain;
                }
                _ => {}
            }
            return self.level > 0.0 || self.stage != Stage::Release;
        }
        let sustain = sustain as f64 / 255.0;
        match self.stage {
            Stage::Attack => {
                self.level += attack as f64 / 255.0;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level *= decay as f64 / 256.0;
                if self.level <= sustain {
                    self.level = sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {}
            Stage::Release => self.level *= release as f64 / 256.0,
        }
        self.level > 1.0 / 256.0 || self.stage != Stage::Release
    }

    /// Its next sample, -1..=1; None once a sample that doesn't loop has ended.
    fn sample(&mut self, rom: &[u8]) -> Option<f64> {
        let value = match self.voice.source {
            Source::Sample {
                data, len, looped, ..
            } => {
                if self.pos >= len as f64 {
                    let start = looped? as f64;
                    let span = len as f64 - start;
                    if span <= 0.0 {
                        return None;
                    }
                    self.pos = start + (self.pos - len as f64) % span;
                }
                let i = self.pos as usize;
                let at = |i: usize| rom.get(data + i).map_or(0.0, |b| *b as i8 as f64 / 128.0);
                let next = if i + 1 < len {
                    i + 1
                } else {
                    looped.unwrap_or(i)
                };
                let t = self.pos.fract();
                at(i) * (1.0 - t) + at(next) * t
            }
            Source::Square { duty, .. } => {
                let high = [0.125, 0.25, 0.5, 0.75][duty as usize];
                if self.pos.fract() < high { 0.5 } else { -0.5 }
            }
            Source::Wave { table } => {
                let i = (self.pos.fract() * 32.0) as usize;
                let byte = rom.get(table + i / 2).copied().unwrap_or(0x88);
                let nibble = if i.is_multiple_of(2) {
                    byte >> 4
                } else {
                    byte & 0x0F
                };
                (nibble as f64 - 7.5) / 15.0
            }
            Source::Noise { short } => {
                let before = self.pos as u64;
                self.pos += self.step;
                for _ in before..self.pos as u64 {
                    let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
                    self.lfsr = (self.lfsr >> 1) | (bit << 14);
                    if short {
                        self.lfsr = (self.lfsr & !0x40) | (bit << 6);
                    }
                }
                return Some(if self.lfsr & 1 == 0 { 0.5 } else { -0.5 });
            }
        };
        self.pos += self.step;
        Some(value)
    }
}

/// A track's place in its commands and the settings they made.
struct Track {
    pc: usize,
    back: Option<usize>,
    wait: u32,
    done: bool,
    command: u8,
    key: u8,
    velocity: u8,
    voice: u8,
    volume: u8,
    pan: i8,
    bend: i8,
    bend_range: u8,
    shift: i8,
    tune: i8,
    repeats: HashMap<usize, u8>,
}

impl Track {
    fn new(pc: usize) -> Self {
        Self {
            pc,
            back: None,
            wait: 0,
            done: false,
            command: 0,
            key: 60,
            velocity: 100,
            voice: 0,
            volume: 100,
            pan: 0,
            bend: 0,
            bend_range: 2,
            shift: 0,
            tune: 0,
            repeats: HashMap::new(),
        }
    }

    /// Semitones notes are moved by: the bend and tuning.
    fn pitch(&self) -> f64 {
        self.bend as f64 * self.bend_range as f64 / 64.0 + self.tune as f64 / 64.0
    }
}

/// How fast a channel steps through its sound, per output sample.
fn step(channel: &Channel, pitch: f64, rate: f64) -> f64 {
    let key = channel.key as f64
        + if channel.voice.key.is_some() {
            0.0
        } else {
            pitch
        };
    let freq = |base: f64| base * 2f64.powf((key - 60.0) / 12.0);
    match channel.voice.source {
        Source::Sample {
            rate: at_c, fixed, ..
        } => {
            if fixed {
                at_c / rate
            } else {
                freq(at_c) / rate
            }
        }
        // Middle C, 261.6 Hz.
        Source::Square { .. } | Source::Wave { .. } => freq(261.625_565) / rate,
        Source::Noise { .. } => freq(4096.0) / rate,
    }
}

/// Renders the song whose header is at ROM offset `at`, `rate` samples a second. Its loop is
/// where a track first jumps back to; at most `seconds` long.
pub fn render(rom: &[u8], at: usize, rate: u32, seconds: Option<f32>) -> Result<Music> {
    let header = Header::read(rom, at)?;
    let out_rate = rate as f64;
    let longest = seconds.map_or(LONGEST, |s| s as f64).min(LONGEST);
    let mut tracks: Vec<Track> = header.tracks.iter().map(|pc| Track::new(*pc)).collect();
    let mut channels: Vec<Channel> = Vec::new();
    let mut samples: Vec<i16> = Vec::new();
    // Which tick each track first reached each command at, and where each tick began.
    let mut seen: HashMap<(usize, usize), u64> = HashMap::new();
    let mut tick_starts: Vec<usize> = Vec::new();
    let mut loop_start = None;
    let mut tempo = TEMPO;
    let mut tick: u64 = 0;
    let mut next_frame = 0.0;
    let mut tick_time = 0.0;
    'song: loop {
        tick_starts.push(samples.len() / 2);
        // The commands of this tick.
        for (t, track) in tracks.iter_mut().enumerate() {
            if track.done {
                continue;
            }
            if track.wait > 0 {
                track.wait -= 1;
                if track.wait > 0 {
                    continue;
                }
            }
            let mut commands = 0;
            while track.wait == 0 && !track.done {
                commands += 1;
                if commands > 10_000 {
                    return Err(format!("track {t} never waits (at 0x{:X})", track.pc));
                }
                seen.entry((t, track.pc)).or_insert(tick);
                let byte = *rom
                    .get(track.pc)
                    .ok_or("track runs past the end of the ROM")?;
                let command = if byte < 0x80 {
                    if track.command == 0 {
                        // An argument with no command to repeat.
                        track.pc += 1;
                        continue;
                    }
                    // Running status: the last command again, this byte its first argument.
                    track.command
                } else {
                    track.pc += 1;
                    if byte >= 0xBD {
                        track.command = byte;
                    }
                    byte
                };
                let arg = |track: &mut Track| -> Result<u8> {
                    let b = *rom
                        .get(track.pc)
                        .ok_or("track runs past the end of the ROM")?;
                    track.pc += 1;
                    Ok(b)
                };
                let optional = |track: &mut Track| -> Option<u8> {
                    let b = *rom.get(track.pc)?;
                    (b < 0x80).then(|| {
                        track.pc += 1;
                        b
                    })
                };
                match command {
                    0x80..=0xB0 => track.wait = LENGTHS[(command - 0x80) as usize] as u32,
                    0xB1 => track.done = true,
                    0xB2 => {
                        let to = pointer(rom, track.pc).ok_or("a jump goes outside the ROM")?;
                        // The first jump back is where the song loops.
                        if to < track.pc
                            && let Some(start) = seen.get(&(t, to))
                        {
                            loop_start = Some(tick_starts[*start as usize]);
                            tick_starts.pop();
                            break 'song;
                        }
                        track.pc = to;
                    }
                    0xB3 => {
                        let to = pointer(rom, track.pc).ok_or("a pattern is outside the ROM")?;
                        track.back = Some(track.pc + 4);
                        track.pc = to;
                    }
                    0xB4 => {
                        if let Some(back) = track.back.take() {
                            track.pc = back;
                        }
                    }
                    0xB5 => {
                        let count = arg(track)?;
                        let to = pointer(rom, track.pc).ok_or("a repeat is outside the ROM")?;
                        let here = track.pc - 2;
                        let done = track.repeats.entry(here).or_insert(0);
                        *done += 1;
                        if count == 0 || *done < count {
                            track.pc = to;
                        } else {
                            track.repeats.remove(&here);
                            track.pc += 4;
                        }
                    }
                    0xB9 => track.pc += 3,
                    0xBA | 0xC2..=0xC5 | 0xC6 | 0xC7 | 0xC9..=0xCC => {
                        arg(track)?;
                    }
                    0xBB => tempo = arg(track)? as f64 * 2.0,
                    0xBC => track.shift = arg(track)? as i8,
                    0xBD => track.voice = arg(track)?,
                    0xBE => track.volume = arg(track)?,
                    0xBF => track.pan = arg(track)? as i8 - 64,
                    0xC0 => track.bend = arg(track)? as i8 - 64,
                    0xC1 => track.bend_range = arg(track)?,
                    0xC8 => track.tune = arg(track)? as i8 - 64,
                    0xCD => {
                        arg(track)?;
                        arg(track)?;
                    }
                    0xCE => {
                        let key = optional(track).unwrap_or(track.key);
                        for c in channels.iter_mut() {
                            if c.track == t && c.note == key && c.left.is_none() {
                                c.stage = Stage::Release;
                            }
                        }
                    }
                    0xCF..=0xFF => {
                        if let Some(key) = optional(track) {
                            track.key = key;
                            if let Some(velocity) = optional(track) {
                                track.velocity = velocity;
                            }
                        }
                        let length = (command > 0xCF).then(|| {
                            let gate = optional(track).unwrap_or(0).min(3);
                            LENGTHS[(command - 0xCF) as usize] as u32 + gate as u32
                        });
                        let note = track.key;
                        let key = (note as i16 + track.shift as i16).clamp(0, 127) as u8;
                        // A voice that can't play is silent, as in the game.
                        if let Ok(voice) = voice(rom, header.voices, track.voice, key, 0) {
                            let mut channel = Channel {
                                track: t,
                                note,
                                key: voice.key.unwrap_or(key),
                                voice,
                                velocity: track.velocity as f64 / 127.0,
                                left: length,
                                stage: Stage::Attack,
                                level: 0.0,
                                pos: 0.0,
                                step: 0.0,
                                lfsr: 0x7FFF,
                                started: tick,
                            };
                            channel.step = step(&channel, track.pitch(), out_rate);
                            if let Some(psg) = voice.source.psg() {
                                channels.retain(|c| c.voice.source.psg() != Some(psg));
                            } else if channels
                                .iter()
                                .filter(|c| c.voice.source.psg().is_none())
                                .count()
                                >= SAMPLE_CHANNELS
                            {
                                // The quietest note makes room, the oldest of those.
                                if let Some(i) = channels
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, c)| c.voice.source.psg().is_none())
                                    .min_by(|(_, a), (_, b)| {
                                        let rank = |c: &Channel| {
                                            (c.stage != Stage::Release, c.level, c.started)
                                        };
                                        rank(a).partial_cmp(&rank(b)).unwrap()
                                    })
                                    .map(|(i, _)| i)
                                {
                                    channels.remove(i);
                                }
                            }
                            channels.push(channel);
                        }
                    }
                    _ => {}
                }
            }
        }
        // Notes end, and bends and volumes take hold.
        for c in channels.iter_mut() {
            if let Some(left) = c.left.as_mut() {
                *left = left.saturating_sub(1);
                if *left == 0 {
                    c.left = None;
                    c.stage = Stage::Release;
                }
            }
            c.step = step(c, tracks[c.track].pitch(), out_rate);
        }
        let all_done = tracks.iter().all(|t| t.done);
        if all_done && channels.is_empty() {
            break;
        }
        tick += 1;
        // The samples of this tick.
        tick_time += 60.0 / (tempo * TICKS_PER_BEAT);
        let until = (tick_time * out_rate).round() as usize;
        while samples.len() / 2 < until {
            if (samples.len() / 2) as f64 / out_rate >= next_frame {
                next_frame += 1.0 / FRAME_RATE;
                channels.retain_mut(|c| c.envelope());
            }
            let (mut left, mut right) = (0.0, 0.0);
            channels.retain_mut(|c| {
                let Some(value) = c.sample(rom) else {
                    return false;
                };
                let track = &tracks[c.track];
                let loud = value * c.level * c.velocity * track.volume as f64 / 127.0;
                let pan = (c.voice.pan.unwrap_or(0) as f64 + track.pan as f64) / 64.0;
                let pan = pan.clamp(-1.0, 1.0);
                left += loud * (1.0 - pan).min(1.0);
                right += loud * (1.0 + pan).min(1.0);
                true
            });
            for v in [left, right] {
                samples.push((v * 0.25 * i16::MAX as f64).clamp(-32768.0, 32767.0) as i16);
            }
        }
        if tick_time >= longest {
            break;
        }
    }
    Ok(Music {
        rate,
        samples,
        loop_start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM with a song at 0x100: one track playing a looping sample voice and a square voice.
    fn rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x400];
        let put = |rom: &mut Vec<u8>, at: usize, bytes: &[u8]| {
            rom[at..at + bytes.len()].copy_from_slice(bytes)
        };
        let ptr = |offset: usize| (ROM + offset as u32).to_le_bytes();
        // Song header: 1 track, voice group at 0x200, track at 0x140.
        put(&mut rom, 0x100, &[1, 0, 0, 0]);
        put(&mut rom, 0x104, &ptr(0x200));
        put(&mut rom, 0x108, &ptr(0x140));
        // Track: tempo 120, voice 0, a note C4 for 24 ticks; then the loop: voice 1, two
        // quarter notes of A4 and a jump back to the loop.
        let mut track = vec![0xBB, 60, 0xBD, 0, 0xE7, 60, 127, 0x98];
        let loop_at = 0x140 + track.len();
        track.extend([0xBD, 1, 0xE7, 69, 100, 0x98, 0xE7, 0x98, 0xB2]);
        track.extend(ptr(loop_at));
        put(&mut rom, 0x140, &track);
        // Voice 0: a sample at 0x300; voice 1: a square wave at 50%.
        put(&mut rom, 0x200, &[0x00, 60, 0, 0]);
        put(&mut rom, 0x204, &ptr(0x300));
        put(&mut rom, 0x208, &[255, 0, 255, 0]);
        put(&mut rom, 0x20C, &[0x01, 60, 0, 0, 2, 0, 0, 0, 0, 0, 15, 0]);
        // The sample: looped from its start, 32 bytes of a saw, 8 kHz at middle C.
        put(&mut rom, 0x300, &0x4000_0000u32.to_le_bytes());
        put(&mut rom, 0x304, &(8000u32 * 1024).to_le_bytes());
        put(&mut rom, 0x308, &0u32.to_le_bytes());
        put(&mut rom, 0x30C, &32u32.to_le_bytes());
        for i in 0..32 {
            rom[0x310 + i] = (i as u8 * 8) ^ 0x80;
        }
        // A song table at 0x80: two songs.
        put(&mut rom, 0x80, &ptr(0x100));
        put(&mut rom, 0x88, &ptr(0x100));
        rom
    }

    #[test]
    fn a_song_renders_its_intro_then_loops() {
        let rom = rom();
        let music = render(&rom, 0x100, 8000, None).unwrap();
        // 120 beats a minute: the intro is one beat (half a second), the loop two.
        assert_eq!(music.loop_start, Some(4000));
        assert_eq!(music.frames(), 12000);
        let (intro, looped) = music.parts();
        assert_eq!(intro, 0..4000);
        assert_eq!(looped, Some(4000..12000));
        let loud = |range: std::ops::Range<usize>| {
            music.samples[range.start * 2..range.end * 2]
                .iter()
                .map(|s| s.unsigned_abs() as u64)
                .sum::<u64>()
        };
        assert!(loud(100..3900) > 0, "the sample sounds");
        assert!(loud(4100..7900) > 0, "the square sounds");
    }

    #[test]
    fn a_song_that_ends_stops() {
        let mut rom = rom();
        // End the track instead of jumping back.
        rom[0x140 + 16] = 0xB1;
        let music = render(&rom, 0x100, 8000, None).unwrap();
        assert_eq!(music.loop_start, None);
        assert!(music.frames() >= 12000);
    }

    #[test]
    fn the_song_table_is_found() {
        let rom = rom();
        assert_eq!(find_song_table(&rom), Some((0x80, vec![0x100, 0x100])));
    }

    #[test]
    fn bad_headers_say_why() {
        let rom = rom();
        assert!(Header::read(&rom, 0x3F0).is_err());
        assert!(render(&rom, 0x10, 8000, None).is_err());
    }
}
