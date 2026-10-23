//! Runs a plugin's recipe against a ROM and writes its images as PNGs.
//!
//! cargo run -p clockwork_recipe --example extract -- <plugin folder> <rom or zip> <out folder>

use clockwork_library::Library;
use clockwork_recipe::Plugin;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [plugin, rom, out] = args.as_slice() else {
        panic!("usage: extract <plugin folder> <rom or zip> <out folder>");
    };

    let plugin = Plugin::load(plugin).expect("could not load plugin");
    let mut library = Library::new().with_platforms(clockwork_platforms::all());
    let entry = library.import(rom).expect("could not import ROM").clone();
    if !plugin.manifest.supports_rom(&entry.sha1) {
        eprintln!("warning: plugin does not list this ROM ({})", entry.sha1);
    }
    let bytes = library.rom_bytes(&entry).expect("could not read ROM");
    let extracted = clockwork_recipe::run(&plugin.recipe, &bytes).expect("recipe failed");

    std::fs::create_dir_all(out).unwrap();
    for (id, image) in extracted.images {
        let path = std::path::Path::new(out).join(format!("{id}.png"));
        image::save_buffer(
            &path,
            &image.pixels,
            image.width,
            image.height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        println!("{} ({}×{})", path.display(), image.width, image.height);
    }
    for (id, map) in extracted.surfaces {
        println!("surfaces \"{id}\": {}×{} cells", map.width, map.height);
        let mut kinds: Vec<_> = map.kinds.iter().enumerate().collect();
        let count = |k: usize| map.cells.iter().filter(|&&c| c as usize == k).count();
        kinds.sort_by_key(|(k, _)| std::cmp::Reverse(count(*k)));
        for (k, kind) in kinds {
            println!("  {:5}  {:?}  {:?}", count(k), kind.properties, kind.codes);
        }
    }
}
