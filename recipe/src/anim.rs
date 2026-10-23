//! Animations stored as value lists per channel (`Step::ChannelAnimation`).

use clockwork_core::model::{Animation, Track};

/// The layout of one stored animation, from the recipe step.
pub struct Layout {
    pub index: usize,
    pub values: usize,
    pub frames: u32,
    pub bones: usize,
    pub root_translation: bool,
}

/// Decodes the channels into one track per bone. `None` when the tables run past the data.
pub fn decode(data: &[u8], layout: &Layout) -> Option<Vec<Track>> {
    let u16_at = |at: usize| Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?));
    let value = |i: usize| {
        let at = layout.values + i * 2;
        Some(i16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
    };
    // Channel `c`'s value at every frame; a short channel holds its last value.
    let channel = |c: usize| -> Option<Vec<f32>> {
        let count = u16_at(layout.index + c * 4)? as usize;
        let first = u16_at(layout.index + c * 4 + 2)? as usize;
        let n = count.min(layout.frames as usize).max(1);
        (0..n)
            .map(|f| value(first + f.min(count.saturating_sub(1))).map(f32::from))
            .collect()
    };

    let mut next = 0;
    let mut take3 = || {
        let c = [channel(next)?, channel(next + 1)?, channel(next + 2)?];
        next += 3;
        Some(c)
    };
    let root = if layout.root_translation {
        Some(take3()?)
    } else {
        None
    };
    let mut tracks = Vec::with_capacity(layout.bones);
    for bone in 0..layout.bones {
        let [x, y, z] = take3()?;
        let frames = x.len().max(y.len()).max(z.len());
        let rotations = (0..frames)
            .map(|f| euler_xyz(held(&x, f), held(&y, f), held(&z, f)))
            .collect();
        let translations = match (&root, bone) {
            (Some([tx, ty, tz]), 0) => {
                let frames = tx.len().max(ty.len()).max(tz.len());
                (0..frames)
                    .map(|f| [held(tx, f), held(ty, f), held(tz, f)])
                    .collect()
            }
            _ => Vec::new(),
        };
        tracks.push(Track {
            bone,
            translations,
            rotations,
        });
    }
    Some(tracks)
}

pub fn animation(
    name: &str,
    frames: u32,
    fps: f32,
    loop_start: Option<u32>,
    tracks: Vec<Track>,
) -> Animation {
    Animation {
        name: name.to_string(),
        frames,
        fps,
        loop_start,
        tracks,
    }
}

fn held(values: &[f32], frame: usize) -> f32 {
    values[frame.min(values.len() - 1)]
}

/// Rotation about x, then y, then z, with angles in 1/65536 turns, as a quaternion (x, y, z, w).
fn euler_xyz(x: f32, y: f32, z: f32) -> [f32; 4] {
    let half = std::f32::consts::PI / 65536.0;
    let (sx, cx) = (x * half).sin_cos();
    let (sy, cy) = (y * half).sin_cos();
    let (sz, cz) = (z * half).sin_cos();
    // qz * qy * qx: x is applied first.
    [
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Index table (count, first) for 3 translation + 3 rotation channels, then values.
    fn data() -> Vec<u8> {
        let index: [(u16, u16); 6] = [(2, 0), (1, 2), (1, 2), (1, 2), (1, 3), (1, 2)];
        let values: [i16; 4] = [10, 20, 0, 0x4000];
        let mut out = Vec::new();
        for (count, first) in index {
            out.extend_from_slice(&count.to_be_bytes());
            out.extend_from_slice(&first.to_be_bytes());
        }
        for v in values {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out
    }

    #[test]
    fn channels_cover_only_the_bones_they_were_made_for() {
        // The table holds one bone's channels. A model with a billboard bone hung after its
        // parts must be decoded for its one animated bone, not read past the table.
        let layout = |bones| Layout {
            index: 0,
            values: 24,
            frames: 3,
            bones,
            root_translation: true,
        };
        assert_eq!(decode(&data(), &layout(1)).unwrap().len(), 1);
        assert!(decode(&data(), &layout(2)).is_none());
    }

    #[test]
    fn decodes_root_translation_and_rotation() {
        let layout = Layout {
            index: 0,
            values: 24,
            frames: 3,
            bones: 1,
            root_translation: true,
        };
        let tracks = decode(&data(), &layout).unwrap();
        let track = &tracks[0];
        assert_eq!(track.translation(0), Some([10.0, 0.0, 0.0]));
        assert_eq!(track.translation(2), Some([20.0, 0.0, 0.0]));
        // A quarter turn about y.
        let q = track.rotation(0).unwrap();
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!((q[1] - s).abs() < 1e-5 && (q[3] - s).abs() < 1e-5, "{q:?}");
    }

    #[test]
    fn euler_order_is_x_then_y_then_z() {
        // Rotating x by a quarter turn about z then y: x first means about x does nothing to
        // the x axis. Check against qz * qy * qx built by hand for z = quarter, x = quarter.
        let q = euler_xyz(16384.0, 0.0, 16384.0);
        let h = 0.5f32;
        // qz = (0,0,s,s), qx = (s,0,0,s) with s = sqrt(1/2); qz*qx = (h, h, h, h).
        for c in q {
            assert!((c - h).abs() < 1e-5, "{q:?}");
        }
    }

    #[test]
    fn short_data_is_rejected() {
        let layout = Layout {
            index: 0,
            values: 24,
            frames: 3,
            bones: 2,
            root_translation: true,
        };
        assert!(decode(&data(), &layout).is_none());
    }
}
