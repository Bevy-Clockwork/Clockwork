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
    let counts = frames
        .counts
        .as_ref()
        .map(|t| table_values(step, buffers, t))
        .transpose()?;
    let palette_firsts = frames
        .palettes
        .as_ref()
        .map(|p| {
            Ok::<_, RecipeError>((
                buffer(step, buffers, &p.buffer)?,
                table_values(step, buffers, &p.first)?,
            ))
        })
        .transpose()?;
    for (what, len) in [
        ("tile", tile_bases.as_ref().map(Vec::len)),
        ("count", counts.as_ref().map(Vec::len)),
        ("palette", palette_firsts.as_ref().map(|(_, t)| t.len())),
    ] {
        if len.is_some_and(|len| len != lists.len()) {
            return Err(decode(
                step,
                format!("the {what} table and the list table need the same count"),
            ));
        }
    }

    let mut sprite = Sprite::default();
    for (index, list) in lists.iter().enumerate() {
        let at = |what: &str| {
            decode(
                step,
                format!("frame {index}: {what} runs past the end of the objects buffer"),
            )
        };
        let start = usize::try_from(*list * frames.list_unit as i64 + frames.list_base as i64)
            .map_err(|_| decode(step, format!("frame {index}: negative list offset")))?;
        // A list starts with its count, unless a table gives the counts.
        let (count, first_object) = match &counts {
            Some(counts) => (counts[index].max(0), start),
            None => (
                clockwork_core::recipe::Field::new(0, frames.count_size)
                    .read(objects.get(start..).ok_or_else(|| at("the list"))?)
                    .ok_or_else(|| at("the object count"))?,
                start + frames.count_size.bytes(),
            ),
        };
        let mut list_objects = Vec::new();
        for j in 0..count as usize {
            let record = first_object + j * fields.stride as usize;
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
        // The frame's own palettes go into the bank's slots from 0, as the game loads them.
        let own_bank = match &palette_firsts {
            Some((data, firsts)) => {
                let from = usize::try_from(firsts[index])
                    .ok()
                    .map(|first| first * 32)
                    .filter(|&from| from < data.len())
                    .ok_or_else(|| {
                        decode(
                            step,
                            format!(
                                "frame {index}: palette {} is past the end of the palettes buffer",
                                firsts[index]
                            ),
                        )
                    })?;
                let mut bank = palettes.clone();
                bank.load(&data[from..data.len().min(from + 16 * 32)], 0);
                Some(bank)
            }
            None => None,
        };
        let (image, origin) = compose(
            &list_objects,
            tiles,
            tile_base,
            own_bank.as_ref().unwrap_or(palettes),
        );
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
    // A directory gives the keyframe count; the other positions end at the end flag.
    let mut count = None;
    let start = match at {
        AnimationAt::Offset(offset) => *offset as usize,
        AnimationAt::Directory {
            table,
            index,
            stride,
            count: count_field,
            first,
            keys,
        } => {
            let entry = *table as usize + *index as usize * *stride as usize;
            let record = data.get(entry..).ok_or_else(past_end)?;
            let (Some(n), Some(first)) = (count_field.read(record), first.read(record)) else {
                return Err(past_end());
            };
            count = Some(n.max(0) as usize);
            *keys as usize + first.max(0) as usize * layout.stride as usize
        }
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
    match count {
        Some(0) => {
            return Err(decode(
                step,
                format!("animation \"{name}\" has no keyframes"),
            ));
        }
        Some(n) if n > MAX_KEYFRAMES => {
            return Err(decode(
                step,
                format!("animation \"{name}\" has {n} keyframes, more than {MAX_KEYFRAMES}"),
            ));
        }
        _ => {}
    }
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
        let end = match count {
            Some(n) => Some((steps.len() + 1 == n) as i64),
            None => layout.end.read(record),
        };
        let (Some(frame), Some(ticks), Some(end)) =
            (layout.frame.read(record), layout.ticks.read(record), end)
        else {
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
            list_unit: 1,
            counts: None,
            palettes: None,
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

    #[test]
    fn older_frame_tables_keep_their_meaning() {
        let frames: FrameTables = ron::from_str(
            r#"(lists: (buffer: "t", offset: 0, stride: 4, count: 1, field: (at: 0)))"#,
        )
        .unwrap();
        assert_eq!(frames.list_unit, 1);
        assert!(frames.counts.is_none() && frames.palettes.is_none());
    }

    #[test]
    fn frames_can_name_their_count_first_object_and_palettes() {
        use clockwork_core::recipe::{FramePalettes, ObjectLayout};
        // Frame records of three u16s: object count, first object (by number), first palette.
        // The one frame draws object 1 (8-byte objects, no counts in between) with the frame's
        // palette 1 + the object's palette 0.
        let mut buffers: Buffers = HashMap::new();
        buffers.insert("tiles".into(), vec![0x11; 32]);
        buffers.insert(
            "objects".into(),
            vec![
                9, 9, 9, 9, 0, 0, 0, 0, /* object 1: y 3, x 2 */ 3, 0, 2, 0, 0, 0, 0, 0,
            ],
        );
        buffers.insert("frames".into(), vec![1, 0, 1, 0, 1, 0]);
        let mut palettes = vec![0u8; 64];
        palettes[32 + 2..32 + 4].copy_from_slice(&0x03E0u16.to_le_bytes()); // palette 1: green
        buffers.insert("palettes".into(), palettes);
        let mut bank = PaletteBank::default();
        bank.load(&[0, 0, 0x1F, 0], 0); // the bank's red, which the frame's palettes replace
        let banks = HashMap::from([("obj".to_string(), bank)]);
        let field = |at| Table {
            buffer: "frames".into(),
            offset: 0,
            stride: 6,
            count: 1,
            field: Field::new(at, FieldSize::U16),
        };
        let frames = FrameTables {
            lists: field(2),
            list_base: 0,
            count_size: FieldSize::U8,
            list_unit: 8,
            counts: Some(field(0)),
            palettes: Some(FramePalettes {
                buffer: "palettes".into(),
                first: field(4),
            }),
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
        assert_eq!(frame.origin, [-2, -3]);
        assert_eq!(frame.image.get(0, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn directories_give_the_keyframe_count() {
        // A directory record (count 2, first keyframe 1) at 0, keyframes from 4. No keyframe
        // has its end flag set, and the first has it set: the count decides.
        let data = [
            2, 1, 0, 0, /* keyframes */ 7, 7, 0, 0x80, 0, 5, 0, 0x80, 1, 6, 0, 0, 9, 9, 0, 0,
        ];
        let a = keyframe_animation(
            0,
            &data,
            &sprite(2),
            "walk",
            &AnimationAt::Directory {
                table: 0,
                index: 0,
                stride: 4,
                count: Field::new(0, FieldSize::U8),
                first: Field::new(1, FieldSize::U8),
                keys: 4,
            },
            &layout(AfterEnd::Restart),
            60.0,
        )
        .unwrap();
        assert_eq!(
            a.steps
                .iter()
                .map(|s| (s.frame, s.ticks))
                .collect::<Vec<_>>(),
            [(0, 5), (1, 6)]
        );
        assert_eq!(a.loop_start, Some(0));
    }
}
