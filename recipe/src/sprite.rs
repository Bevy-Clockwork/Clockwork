//! Sprite steps: frames composed from hardware objects, and keyframe animations over them.

use std::collections::HashMap;

use clockwork_core::recipe::{
    AfterEnd, AnimationAt, FrameTables, KeyframeLayout, ObjectLayout, Table,
};
use clockwork_core::sprite::{AnimationStep, Sprite, SpriteAnimation, SpriteFrame};
use clockwork_gba::gfx::PaletteBank;
use clockwork_gba::obj::{Object, compose};

use crate::RecipeError;

/// The most keyframes read for one animation; a list without an end marker stops here.
const MAX_KEYFRAMES: usize = 4096;

type Buffers = HashMap<String, Vec<u8>>;

fn buffer<'b>(step: usize, buffers: &'b Buffers, name: &str) -> Result<&'b [u8], RecipeError> {
    buffers
        .get(name)
        .map(Vec::as_slice)
        .ok_or_else(|| RecipeError::Unknown {
            step,
            kind: "buffer",
            name: name.to_string(),
        })
}

fn decode(step: usize, message: String) -> RecipeError {
    RecipeError::Decode { step, message }
}

fn table_values(step: usize, buffers: &Buffers, table: &Table) -> Result<Vec<i64>, RecipeError> {
    let data = buffer(step, buffers, &table.buffer)?;
    table.values(data).ok_or_else(|| {
        decode(
            step,
            format!(
                "table in \"{}\" runs past the end of the buffer",
                table.buffer
            ),
        )
    })
}

/// Builds a sprite whose frames are drawn from hardware objects.
#[allow(clippy::too_many_arguments)] // The step's parameters, as they are.
pub(crate) fn object_frames(
    step: usize,
    buffers: &Buffers,
    banks: &HashMap<String, PaletteBank>,
    tiles: &str,
    bank: &str,
    objects: &str,
    layout: &ObjectLayout,
    frames: &FrameTables,
) -> Result<Sprite, RecipeError> {
    let tiles = buffer(step, buffers, tiles)?;
    let objects = buffer(step, buffers, objects)?;
    let palettes = banks.get(bank).ok_or_else(|| RecipeError::Unknown {
        step,
        kind: "palette bank",
        name: bank.to_string(),
    })?;
    let fields = layout.fields();
    let lists = table_values(step, buffers, &frames.lists)?;
    let tile_bases = frames
        .tile_base
        .as_ref()
        .map(|t| table_values(step, buffers, t))
        .transpose()?;
    if tile_bases.as_ref().is_some_and(|t| t.len() != lists.len()) {
        return Err(decode(
            step,
            "the tile table and the list table need the same count".into(),
        ));
    }

    let mut sprite = Sprite::default();
    for (index, list) in lists.iter().enumerate() {
        let at = |what: &str| {
            decode(
                step,
                format!("frame {index}: {what} runs past the end of the objects buffer"),
            )
        };
        let start = usize::try_from(*list + frames.list_base as i64)
            .map_err(|_| decode(step, format!("frame {index}: negative list offset")))?;
        let count = clockwork_core::recipe::Field::new(0, frames.count_size)
            .read(objects.get(start..).ok_or_else(|| at("the list"))?)
            .ok_or_else(|| at("the object count"))?;
        let mut list_objects = Vec::new();
        for j in 0..count as usize {
            let record = start + frames.count_size.bytes() + j * fields.stride as usize;
            let record = objects.get(record..).ok_or_else(|| at("an object"))?;
            let read = |f: &clockwork_core::recipe::Field| f.read(record);
            let optional = |f: &Option<clockwork_core::recipe::Field>| {
                f.as_ref().map(|f| f.read(record)).unwrap_or(Some(0))
            };
            let (Some(x), Some(y), Some(shape), Some(size), Some(tile)) = (
                read(&fields.x),
                read(&fields.y),
                read(&fields.shape),
                read(&fields.size),
                read(&fields.tile),
            ) else {
                return Err(at("an object"));
            };
            let (Some(hflip), Some(vflip), Some(palette)) = (
                optional(&fields.hflip),
                optional(&fields.vflip),
                optional(&fields.palette),
            ) else {
                return Err(at("an object"));
            };
            list_objects.push(Object {
                x: x as i32,
                y: y as i32,
                shape: shape as u8,
                size: size as u8,
                hflip: hflip != 0,
                vflip: vflip != 0,
                tile: tile as u32,
                palette: palette as u8,
            });
        }
        let tile_base =
            tile_bases.as_ref().map_or(0, |t| t[index].max(0) as u32) + frames.first_tile;
        let (image, origin) = compose(&list_objects, tiles, tile_base, palettes);
        sprite.frames.push(SpriteFrame { image, origin });
    }
    Ok(sprite)
}

/// Reads one animation's keyframes into a [`SpriteAnimation`] of `sprite`.
pub(crate) fn keyframe_animation(
    step: usize,
    data: &[u8],
    sprite: &Sprite,
    name: &str,
    at: &AnimationAt,
    layout: &KeyframeLayout,
    fps: f32,
) -> Result<SpriteAnimation, RecipeError> {
    let past_end = || decode(step, format!("animation \"{name}\" runs past the buffer"));
    let start = match at {
        AnimationAt::Offset(offset) => *offset as usize,
        AnimationAt::Pointer { table, index, base } => {
            let entry = *table as usize + *index as usize * 4;
            let bytes = data.get(entry..entry + 4).ok_or_else(past_end)?;
            let pointer = u32::from_le_bytes(bytes.try_into().unwrap());
            pointer.checked_sub(*base).ok_or_else(|| {
                decode(
                    step,
                    format!(
                        "animation \"{name}\": pointer {pointer:#x} is below the base {base:#x}"
                    ),
                )
            })? as usize
        }
    };
    let mut steps = Vec::new();
    loop {
        if steps.len() >= MAX_KEYFRAMES {
            return Err(decode(
                step,
                format!("animation \"{name}\" has no end after {MAX_KEYFRAMES} keyframes"),
            ));
        }
        let record = data
            .get(start + steps.len() * layout.stride as usize..)
            .ok_or_else(past_end)?;
        let (Some(frame), Some(ticks), Some(end)) = (
            layout.frame.read(record),
            layout.ticks.read(record),
            layout.end.read(record),
        ) else {
            return Err(past_end());
        };
        let flag = |f: &Option<clockwork_core::recipe::Field>| {
            f.as_ref().and_then(|f| f.read(record)).unwrap_or(0) != 0
        };
        let frame = usize::try_from(frame).unwrap_or(usize::MAX);
        if frame >= sprite.frames.len() {
            return Err(decode(
                step,
                format!(
                    "animation \"{name}\": keyframe {} uses frame {frame}, but the sprite has {}",
                    steps.len(),
                    sprite.frames.len()
                ),
            ));
        }
        steps.push(AnimationStep {
            frame,
            ticks: ticks.max(0) as u32,
            flip_x: flag(&layout.flip_x),
            flip_y: flag(&layout.flip_y),
        });
        if end != 0 {
            break;
        }
    }
    let loop_start = match layout.after_end {
        AfterEnd::Stop => None,
        AfterEnd::Restart => Some(0),
        AfterEnd::RewindByte => {
            let back = *data
                .get(start + steps.len() * layout.stride as usize)
                .ok_or_else(past_end)? as usize;
            (back > 0).then(|| steps.len().saturating_sub(back))
        }
    };
    Ok(SpriteAnimation {
        name: name.to_string(),
        steps,
        fps,
        loop_start,
    })
}

#[cfg(test)]
mod tests {
    use clockwork_core::recipe::{Field, FieldSize};

    use super::*;

    fn layout(after_end: AfterEnd) -> KeyframeLayout {
        KeyframeLayout {
            stride: 4,
            frame: Field::new(0, FieldSize::U8),
            ticks: Field::new(1, FieldSize::U8),
            flip_x: Some(Field::new(2, FieldSize::U8).bits(6, 1)),
            flip_y: None,
            end: Field::new(3, FieldSize::U8).bits(7, 1),
            after_end,
        }
    }

    fn sprite(frames: usize) -> Sprite {
        Sprite {
            frames: (0..frames)
                .map(|_| SpriteFrame {
                    image: clockwork_core::image::RgbaImage::new(1, 1),
                    origin: [0, 0],
                })
                .collect(),
            animations: vec![],
        }
    }

    #[test]
    fn reads_keyframes_and_the_rewind_byte() {
        // Three keyframes (the second mirrored, the third ends the list), then "go back 2".
        let data = [0, 5, 0, 0, 1, 7, 0x40, 0, 2, 9, 0, 0x80, 2];
        let a = keyframe_animation(
            0,
            &data,
            &sprite(3),
            "walk",
            &AnimationAt::Offset(0),
            &layout(AfterEnd::RewindByte),
            60.0,
        )
        .unwrap();
        assert_eq!(a.steps.len(), 3);
        assert_eq!(
            (a.steps[1].frame, a.steps[1].ticks, a.steps[1].flip_x),
            (1, 7, true)
        );
        assert!(!a.steps[0].flip_x);
        assert_eq!(a.loop_start, Some(1));
    }

    #[test]
    fn follows_pointer_tables() {
        // Memory based at 0x1000: a pointer table at 0, an animation at 8.
        let mut data = vec![];
        data.extend_from_slice(&0x1008u32.to_le_bytes());
        data.extend_from_slice(&0x1008u32.to_le_bytes());
        data.extend_from_slice(&[1, 3, 0, 0x80]);
        let a = keyframe_animation(
            0,
            &data,
            &sprite(2),
            "idle",
            &AnimationAt::Pointer {
                table: 0,
                index: 1,
                base: 0x1000,
            },
            &layout(AfterEnd::Restart),
            30.0,
        )
        .unwrap();
        assert_eq!(a.steps[0].frame, 1);
        assert_eq!(a.loop_start, Some(0));
    }

    #[test]
    fn rejects_frames_the_sprite_does_not_have() {
        let data = [5, 1, 0, 0x80];
        let err = keyframe_animation(
            0,
            &data,
            &sprite(2),
            "bad",
            &AnimationAt::Offset(0),
            &layout(AfterEnd::Stop),
            30.0,
        )
        .unwrap_err();
        assert!(err.to_string().contains("frame 5"));
    }

    #[test]
    fn composes_frames_from_tables() {
        use clockwork_core::recipe::{FrameTables, ObjectLayout};
        // One red tile; one frame whose list (count 1) holds a single 8×8 OAM object at (2, 3).
        let mut buffers: Buffers = HashMap::new();
        buffers.insert("tiles".into(), vec![0x11; 32]);
        // attr0: y = 3, shape 0. attr1: x = 2, size 0. attr2: tile 0, palette 0.
        buffers.insert("objects".into(), vec![1, 3, 0, 2, 0, 0, 0]);
        buffers.insert("lists".into(), vec![0, 0, 0, 0]);
        let mut banks = HashMap::new();
        let mut bank = PaletteBank::default();
        bank.load(&[0, 0, 0x1F, 0], 0);
        banks.insert("obj".to_string(), bank);
        let frames = FrameTables {
            lists: Table {
                buffer: "lists".into(),
                offset: 0,
                stride: 4,
                count: 1,
                field: Field::new(0, FieldSize::U32),
            },
            list_base: 0,
            count_size: FieldSize::U8,
            tile_base: None,
            first_tile: 0,
        };
        let sprite = object_frames(
            0,
            &buffers,
            &banks,
            "tiles",
            "obj",
            "objects",
            &ObjectLayout::Oam,
            &frames,
        )
        .unwrap();
        let frame = &sprite.frames[0];
        assert_eq!(
            (frame.image.width, frame.image.height, frame.origin),
            (8, 8, [-2, -3])
        );
        assert_eq!(frame.image.get(0, 0), [255, 0, 0, 255]);
    }
}
