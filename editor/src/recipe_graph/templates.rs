//! The steps and outputs that can be added, as RON text to start from.

use std::sync::LazyLock;

use clockwork_core::{
    recipe::{Access, NameKind, Output, Step},
    recipe_schema::{self, FieldType},
};

/// New steps and outputs, as written in RON. Names starting with `new_` get a number that makes
/// them unique.
pub const STEP_TEMPLATES: &[(&str, &str)] = &[
    (
        "Read",
        r#"Read(name: "new_buffer", offset: 0x0, size: Some(16))"#,
    ),
    ("Place", r#"Place(from: "", into: "new_buffer", at: 0x0)"#),
    ("Fill", r#"Fill(name: "new_buffer", size: 16)"#),
    ("Literal", r#"Literal(name: "new_buffer", hex: "00")"#),
    (
        "GbaPalettes",
        r#"GbaPalettes(from: "", bank: "bg", slot: 0)"#,
    ),
    (
        "GbaMetatiles",
        r#"GbaMetatiles(tiles: "", char_base: 0x0, metatiles: "", bank: "bg", columns: 16, into: "new_image")"#,
    ),
    (
        "GbaMetatileMap",
        r#"GbaMetatileMap(tiles: "", char_base: 0x0, metatiles: "", map: "", width: 16, bank: "bg", into: "new_image")"#,
    ),
    ("Overlay", r#"Overlay(from: "", into: "")"#),
    (
        "Crop",
        r#"Crop(from: "", region: (x: 0, y: 0, w: 16, h: 16), into: "new_image")"#,
    ),
    (
        "Text",
        r#"Text(from: "", at: 0x0, runs: [(0x00, "0123456789")], codes: {}, end: 0xFF, into: "text")"#,
    ),
    (
        "Canvas",
        r#"Canvas(name: "new_image", width: 64, height: 64)"#,
    ),
    ("Paste", r#"Paste(from: "", into: "", at: (0, 0))"#),
    (
        "N64Texture",
        r#"N64Texture(from: "", at: 0x0, format: Rgba16, width: 16, height: 16, into: "new_image")"#,
    ),
    (
        "Lookup",
        r#"Lookup(indices: "", table: "", into: "new_buffer")"#,
    ),
    (
        "N64Model",
        r#"N64Model(segments: {}, bones: [], into: "new_model")"#,
    ),
    (
        "CollisionTriangles",
        r#"CollisionTriangles(from: "", at: 0x0, layout: (word: U16, end: 0x0), into: "new_collision")"#,
    ),
    (
        "ChannelAnimation",
        r#"ChannelAnimation(from: "", name: "new_animation", index: 0x0, values: 0x0, frames: 1, into: "")"#,
    ),
    (
        "GbaObjectFrames",
        r#"GbaObjectFrames(tiles: "", bank: "", objects: "", frames: (lists: (buffer: "", offset: 0x0, stride: 4, count: 1, field: (at: 0))), into: "new_sprite")"#,
    ),
    ("MergeSprite", r#"MergeSprite(from: "", into: "")"#),
    (
        "KeyframeAnimation",
        r#"KeyframeAnimation(from: "", name: "new_animation", at: Offset(0x0), layout: (stride: 4, frame: (at: 0), ticks: (at: 1), end: (at: 2)), fps: 60.0, into: "")"#,
    ),
];

pub const OUTPUT_TEMPLATES: &[(&str, &str)] = &[
    ("Image", r#"Image(id: "new_image", from: "")"#),
    (
        "Surfaces",
        r#"Surfaces(id: "new_surfaces", width: 1, cell_size: 16, grids: [])"#,
    ),
    (
        "Model",
        r#"Model(id: "new_model", from: "", meters_per_unit: 0.01)"#,
    ),
    (
        "Sprite",
        r#"Sprite(id: "new_sprite", from: "", pixels_per_meter: 16.0)"#,
    ),
    ("Items", r#"Items(id: "new_items", items: [])"#),
    (
        "Ability",
        r#"Ability(id: "new_ability", name: "New ability", from: "", actions: [])"#,
    ),
];

/// Every step or output that can be added, by variant name: the written templates, then one
/// for each other kind of output the schema describes, made from its starter
/// ([`clockwork_core::asset::starter`]: a built-in asset or default values) or from its
/// fields' defaults.
pub(super) fn templates(step: bool) -> &'static [(&'static str, String)] {
    static STEPS: LazyLock<Vec<(&'static str, String)>> = LazyLock::new(|| {
        STEP_TEMPLATES
            .iter()
            .map(|(n, t)| (*n, t.to_string()))
            .collect()
    });
    static OUTPUTS: LazyLock<Vec<(&'static str, String)>> = LazyLock::new(|| {
        let mut all: Vec<(&'static str, String)> = OUTPUT_TEMPLATES
            .iter()
            .map(|(n, t)| (*n, t.to_string()))
            .collect();
        for info in recipe_schema::OUTPUTS {
            if !all.iter().any(|(n, _)| *n == info.name)
                && let Some(template) = schema_template(info)
            {
                all.push((info.name, template));
            }
        }
        all
    });
    if step { &STEPS } else { &OUTPUTS }
}

/// A new output of a kind with no written template, if one can be made that parses.
pub(super) fn schema_template(info: &recipe_schema::VariantInfo) -> Option<String> {
    let id = format!("new_{}", snake(info.name));
    let pretty = ron::ser::PrettyConfig::new()
        .compact_structs(true)
        .compact_arrays(true)
        .compact_maps(true);
    if let Some(output) = clockwork_core::asset::starter(info.name, &id) {
        return ron::ser::to_string_pretty(&output, pretty).ok();
    }
    // Its required fields at their defaults, each structure tried as a list, a map, a record
    // and a pair until the whole parses.
    const CANDIDATES: [&str; 4] = ["[]", "{}", "()", "(8, 8)"];
    let fields: Vec<&recipe_schema::FieldInfo> =
        info.fields.iter().filter(|f| !f.optional).collect();
    let structures = fields
        .iter()
        .filter(|f| f.key != "id" && f.ty == FieldType::Structure)
        .count();
    (0..CANDIDATES.len().pow(structures as u32)).find_map(|mut pick| {
        let values: Vec<String> = fields
            .iter()
            .map(|f| {
                let value = if f.key == "id" {
                    format!("\"{id}\"")
                } else if f.ty == FieldType::Structure {
                    let candidate = CANDIDATES[pick % CANDIDATES.len()];
                    pick /= CANDIDATES.len();
                    candidate.to_string()
                } else {
                    f.ty.default_text().to_string()
                };
                format!("{}: {value}", f.key)
            })
            .collect();
        let text = format!("{}({})", info.name, values.join(", "));
        ron::from_str::<Output>(&text).is_ok().then_some(text)
    })
}

/// "input_map" from "InputMap".
pub(super) fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// The kinds a template's step or output reads, and the kinds it makes.
pub(super) fn template_kinds(template: &str, step: bool) -> (Vec<NameKind>, Vec<NameKind>) {
    let ports: Vec<(NameKind, Access)> = if step {
        ron::from_str::<Step>(template)
            .map(|s| s.ports().iter().map(|p| (p.kind, p.access)).collect())
            .unwrap_or_default()
    } else {
        ron::from_str::<Output>(template)
            .map(|o| o.ports().iter().map(|p| (p.kind, p.access)).collect())
            .unwrap_or_default()
    };
    (
        ports
            .iter()
            .filter(|(_, a)| a.reads())
            .map(|(k, _)| *k)
            .collect(),
        ports
            .iter()
            .filter(|(_, a)| a.writes())
            .map(|(k, _)| *k)
            .collect(),
    )
}
