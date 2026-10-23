//! PlayStation and PlayStation 2 identification from executable and disc contents.
//!
//! Supports PS-X EXE files and ISO9660 data tracks in 2048-byte ISO or 2352-byte
//! Mode 1 / Mode 2 Form 1 BIN sectors. BOOT / BOOT2 in SYSTEM.CNF selects the
//! console; the referenced executable must also be present and valid. Disc titles
//! come from the ISO volume label, and serials from the boot executable name.
//!
//! Raw images are deliberately preserved: converting every sector to 2048 bytes
//! would discard XA audio/video, CD audio, and sector metadata. Compressed images,
//! 2448-byte subchannel dumps, and standalone PS2 ELF files are not identified.
//!
//! Format references: [ECMA-119](https://ecma-international.org/publications-and-standards/standards/ecma-119/),
//! [PS-X EXE](https://psx-spx.consoledev.net/dev/ps1/cdr/cdromfileformats/executables/),
//! [PCSX2 boot detection](https://github.com/PCSX2/pcsx2/blob/master/pcsx2/CDVD/CDVD.cpp).

use clockwork_core::platform::{Platform, RomInfo, header_text};

const BLOCK: usize = 2048;
const RAW_BLOCK: usize = 2352;
const SYNC: [u8; 12] = [0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0];
const MAX_DIRECTORY: usize = 4 * 1024 * 1024;
const MAX_CONFIG: usize = 64 * 1024;

pub struct Ps1;
pub struct Ps2;

impl Platform for Ps1 {
    fn id(&self) -> &'static str {
        "ps1"
    }

    fn identify(&self, rom: &[u8]) -> Option<RomInfo> {
        if valid_psx_exe(rom, rom.len()) {
            return Some(RomInfo {
                platform: self.id(),
                title: String::new(),
                game_code: String::new(),
            });
        }
        identify_disc(rom, self.id())
    }
}

impl Platform for Ps2 {
    fn id(&self) -> &'static str {
        "ps2"
    }

    fn identify(&self, rom: &[u8]) -> Option<RomInfo> {
        identify_disc(rom, self.id())
    }
}

fn identify_disc(bytes: &[u8], platform: &'static str) -> Option<RomInfo> {
    let disc = Disc::open(bytes)?;
    let (detected, path) = if let Some(config) = disc.find("SYSTEM.CNF") {
        let config = disc.read(config, MAX_CONFIG)?;
        boot_path(&config)?
    } else {
        // The PS1 BIOS also supports a root PSX.EXE with no SYSTEM.CNF.
        ("ps1", "PSX.EXE".to_owned())
    };
    if detected != platform {
        return None;
    }
    let boot = disc.find(&path)?;
    let valid = match platform {
        "ps1" => valid_psx_exe(&disc.read_range(boot, 0, BLOCK)?, boot.size),
        "ps2" => valid_ps2_elf(&disc, boot),
        _ => false,
    };
    valid.then(|| RomInfo {
        platform,
        title: disc.title.clone(),
        game_code: serial(&path),
    })
}

fn valid_psx_exe(header: &[u8], file_size: usize) -> bool {
    if header.len() < BLOCK || &header[..8] != b"PS-X EXE" {
        return false;
    }
    let payload = le32(header, 0x1c).unwrap_or(0) as usize;
    payload != 0
        && payload
            .checked_add(BLOCK)
            .is_some_and(|size| size <= file_size)
}

fn valid_ps2_elf(disc: &Disc<'_>, file: Entry) -> bool {
    let Some(header) = disc.read_range(file, 0, 52) else {
        return false;
    };
    if &header[..7] != b"\x7fELF\x01\x01\x01"
        || le16(&header, 16) != Some(2)
        || le16(&header, 18) != Some(8)
        || le32(&header, 20) != Some(1)
        || le16(&header, 40) != Some(52)
        || le16(&header, 42) != Some(32)
    {
        return false;
    }
    let offset = le32(&header, 28).unwrap_or(0) as usize;
    let count = le16(&header, 44).unwrap_or(0) as usize;
    if offset < 52 || count == 0 || count > 1024 {
        return false;
    }
    let Some(table) = disc.read_range(file, offset, count * 32) else {
        return false;
    };
    let mut loadable = false;
    for program in table.as_chunks::<32>().0 {
        if le32(program, 0) != Some(1) {
            continue;
        }
        let offset = le32(program, 4).unwrap_or(0) as usize;
        let size = le32(program, 16).unwrap_or(0) as usize;
        let memory_size = le32(program, 20).unwrap_or(0) as usize;
        if size > memory_size || offset.checked_add(size).is_none_or(|end| end > file.size) {
            return false;
        }
        loadable |= size > 0;
    }
    loadable
}

fn boot_path(config: &[u8]) -> Option<(&'static str, String)> {
    let text = std::str::from_utf8(config).ok()?.trim_end_matches('\0');
    let mut boot = None;
    let mut boot2 = None;
    for line in text.split(['\r', '\n']) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let slot = if key.trim().eq_ignore_ascii_case("BOOT2") {
            &mut boot2
        } else if key.trim().eq_ignore_ascii_case("BOOT") {
            &mut boot
        } else {
            continue;
        };
        if slot.is_some() {
            continue;
        }
        let token = value.split_ascii_whitespace().next()?;
        let (device, path) = token.split_once(':')?;
        if !device.eq_ignore_ascii_case("cdrom") && !device.eq_ignore_ascii_case("cdrom0") {
            return None;
        }
        let path = path.trim_start_matches(['/', '\\']);
        if path.is_empty() || path.as_bytes().contains(&0) {
            return None;
        }
        *slot = Some(path.to_owned());
    }
    // A PS2 boot entry takes precedence if a hybrid disc also supplies BOOT.
    boot2
        .map(|path| ("ps2", path))
        .or_else(|| boot.map(|path| ("ps1", path)))
}

fn serial(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let name = name.split(';').next().unwrap_or(name).as_bytes();
    if name.len() == 11
        && name[..4].iter().all(u8::is_ascii_alphabetic)
        && matches!(name[4], b'_' | b'-')
        && name[5..8].iter().all(u8::is_ascii_digit)
        && name[8] == b'.'
        && name[9..].iter().all(u8::is_ascii_digit)
    {
        let mut code = String::from_utf8_lossy(&name[..4]).to_ascii_uppercase();
        code.push('-');
        code.push_str(std::str::from_utf8(&name[5..8]).unwrap_or_default());
        code.push_str(std::str::from_utf8(&name[9..]).unwrap_or_default());
        code
    } else {
        String::new()
    }
}

#[derive(Clone, Copy)]
struct Entry {
    block: usize,
    size: usize,
    directory: bool,
}

struct Disc<'a> {
    bytes: &'a [u8],
    stride: usize,
    start: usize,
    blocks: usize,
    root: Entry,
    title: String,
}

impl<'a> Disc<'a> {
    fn open(bytes: &'a [u8]) -> Option<Self> {
        // A raw data track can include its 150-sector pregap.
        for (stride, start) in [(BLOCK, 0), (RAW_BLOCK, 0), (RAW_BLOCK, 150)] {
            if !bytes.len().is_multiple_of(stride) {
                continue;
            }
            let mut disc = Self {
                bytes,
                stride,
                start,
                blocks: (bytes.len() / stride).saturating_sub(start),
                root: Entry {
                    block: 0,
                    size: 0,
                    directory: true,
                },
                title: String::new(),
            };
            // Bound descriptor scans; no search through arbitrary game data.
            for block in 16..48 {
                let Some(sector) = disc.sector(block) else {
                    break;
                };
                if &sector[1..6] != b"CD001" || sector[6] != 1 || sector[0] == 255 {
                    break;
                }
                if sector[0] != 1 {
                    continue;
                }
                let Some(blocks) = both32(sector, 80).map(|n| n as usize) else {
                    break;
                };
                if blocks <= block
                    || blocks > disc.blocks
                    || both16(sector, 128) != Some(BLOCK as u16)
                {
                    break;
                }
                let Some(root) = entry(&sector[156..190], blocks) else {
                    break;
                };
                if !root.directory || root.size == 0 {
                    break;
                }
                disc.title = header_text(&sector[40..72]);
                disc.root = root;
                disc.blocks = blocks;
                return Some(disc);
            }
        }
        None
    }

    fn sector(&self, block: usize) -> Option<&'a [u8]> {
        if block >= self.blocks {
            return None;
        }
        let offset = block.checked_add(self.start)?.checked_mul(self.stride)?;
        let sector = self.bytes.get(offset..offset.checked_add(self.stride)?)?;
        if self.stride == BLOCK {
            return Some(sector);
        }
        if sector[..12] != SYNC {
            return None;
        }
        let data_offset = match sector[15] {
            1 => 16,
            // XA subheaders occur twice; Form 2 is audio/video, not ISO file data.
            2 if sector[16..20] == sector[20..24] && sector[18] & 0x20 == 0 => 24,
            _ => return None,
        };
        sector.get(data_offset..data_offset + BLOCK)
    }

    fn read(&self, file: Entry, max: usize) -> Option<Vec<u8>> {
        if file.size > max {
            return None;
        }
        self.read_range(file, 0, file.size)
    }

    fn read_range(&self, file: Entry, offset: usize, length: usize) -> Option<Vec<u8>> {
        if offset.checked_add(length)? > file.size {
            return None;
        }
        let mut out = Vec::with_capacity(length);
        let mut position = offset;
        while out.len() < length {
            let block = file.block.checked_add(position / BLOCK)?;
            let sector = self.sector(block)?;
            let within = position % BLOCK;
            let take = (length - out.len()).min(BLOCK - within);
            out.extend_from_slice(&sector[within..within + take]);
            position = position.checked_add(take)?;
        }
        Some(out)
    }

    fn find(&self, path: &str) -> Option<Entry> {
        let mut current = self.root;
        for (depth, component) in path.split(['/', '\\']).enumerate() {
            if depth >= 16
                || component.is_empty()
                || matches!(component, "." | "..")
                || !current.directory
            {
                return None;
            }
            current = self.find_in_directory(current, component)?;
        }
        (!current.directory).then_some(current)
    }

    fn find_in_directory(&self, directory: Entry, component: &str) -> Option<Entry> {
        let data = self.read(directory, MAX_DIRECTORY)?;
        let mut offset = 0usize;
        while offset < data.len() {
            let size = data[offset] as usize;
            if size == 0 {
                offset = (offset / BLOCK + 1) * BLOCK;
                continue;
            }
            // ISO records may not straddle a logical sector.
            if size < 34 || offset % BLOCK + size > BLOCK {
                return None;
            }
            let record = data.get(offset..offset.checked_add(size)?)?;
            let name = record.get(33..33 + record[32] as usize)?;
            if name != [0] && name != [1] {
                let name = std::str::from_utf8(name).ok()?;
                let name = name.split(';').next()?;
                let component = component.split(';').next()?;
                if name.eq_ignore_ascii_case(component) {
                    return entry(record, self.blocks);
                }
            }
            offset += size;
        }
        None
    }
}

fn entry(record: &[u8], blocks: usize) -> Option<Entry> {
    if record.len() < 34
        || record[0] as usize != record.len()
        || record[26] != 0
        || record[27] != 0
        || record[25] & 0x80 != 0
    {
        return None;
    }
    // Skip any extended-attribute blocks before the actual file data.
    let block = (both32(record, 2)? as usize).checked_add(record[1] as usize)?;
    let size = both32(record, 10)? as usize;
    if block.checked_add(size.div_ceil(BLOCK))? > blocks {
        return None;
    }
    Some(Entry {
        block,
        size,
        directory: record[25] & 2 != 0,
    })
}

fn le16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn le32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn both16(bytes: &[u8], offset: usize) -> Option<u16> {
    let little = le16(bytes, offset)?;
    let big = u16::from_be_bytes(bytes.get(offset + 2..offset + 4)?.try_into().ok()?);
    (little == big).then_some(little)
}

fn both32(bytes: &[u8], offset: usize) -> Option<u32> {
    let little = le32(bytes, offset)?;
    let big = u32::from_be_bytes(bytes.get(offset + 4..offset + 8)?.try_into().ok()?);
    (little == big).then_some(little)
}

#[cfg(test)]
mod tests;
