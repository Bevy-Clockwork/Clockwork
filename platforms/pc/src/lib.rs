//! Identifies Windows PE and Linux x86/x64 ELF executables for library import.
//!
//! Executable headers do not establish whether a program is a game, nor expose a reliable
//! game title. The library supplies the filename as a fallback. This module does not run
//! executables or interpret their assets. Static PIE without an interpreter is not recognized.

use clockwork_core::platform::{Platform, RomInfo};

pub struct Pc;

impl Platform for Pc {
    fn id(&self) -> &'static str {
        "pc"
    }

    fn identify(&self, data: &[u8]) -> Option<RomInfo> {
        if pe_executable(data).is_none() && elf_executable(data).is_none() {
            return None;
        }
        Some(RomInfo {
            platform: self.id(),
            title: String::new(),
            game_code: String::new(),
        })
    }
}

// Layout and flag definitions: https://learn.microsoft.com/en-us/windows/win32/debug/pe-format
fn pe_executable(data: &[u8]) -> Option<()> {
    if data.get(..2)? != b"MZ" {
        return None;
    }
    let offset = usize::try_from(u32_at(data, 0x3c)?).ok()?;
    if offset < 64 || data.get(offset..offset.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let coff = data.get(offset.checked_add(4)?..offset.checked_add(24)?)?;
    let machine = u16_at(coff, 0)?;
    let sections = usize::from(u16_at(coff, 2)?);
    let optional_len = usize::from(u16_at(coff, 16)?);
    let flags = u16_at(coff, 18)?;
    if !(1..=96).contains(&sections) || flags & 2 == 0 || flags & 0x3000 != 0 {
        return None;
    }
    let optional_offset = offset.checked_add(24)?;
    let section_offset = optional_offset.checked_add(optional_len)?;
    let optional = data.get(optional_offset..section_offset)?;
    let minimum = match (machine, u16_at(optional, 0)?) {
        (0x014c, 0x010b) => 96,
        (0x8664 | 0xaa64, 0x020b) => 112,
        _ => return None,
    };
    if optional.len() < minimum || !matches!(u16_at(optional, 68)?, 2 | 3) {
        return None;
    }
    let directories = usize::try_from(u32_at(optional, minimum - 4)?).ok()?;
    if directories > (optional_len - minimum) / 8 {
        return None;
    }
    let entry = u64::from(u32_at(optional, 16)?);
    if entry == 0 {
        return None;
    }
    let table_end = section_offset.checked_add(sections.checked_mul(40)?)?;
    let table = data.get(section_offset..table_end)?;
    let headers_len = usize::try_from(u32_at(optional, 60)?).ok()?;
    if headers_len < table_end || headers_len > data.len() {
        return None;
    }
    let mut executable_entry = false;
    for section in table.as_chunks::<40>().0 {
        let virtual_size = u64::from(u32_at(section, 8)?);
        let address = u64::from(u32_at(section, 12)?);
        let raw_size = u64::from(u32_at(section, 16)?);
        let raw_offset = u64::from(u32_at(section, 20)?);
        if raw_size != 0 {
            file_range(data, raw_offset, raw_size)?;
        }
        if u32_at(section, 36)? & 0x2000_0000 != 0
            && entry >= address
            && entry - address < virtual_size.max(raw_size)
        {
            executable_entry = true;
        }
    }
    executable_entry.then_some(())
}

// ELF ABI: https://refspecs.linuxfoundation.org/elf/elf.pdf
fn elf_executable(data: &[u8]) -> Option<()> {
    if data.get(..4)? != b"\x7fELF" || data.get(5..7)? != [1, 1] {
        return None;
    }
    // System V and Linux ABI, restricted to desktop x86/x64. In particular PS2 MIPS ELF
    // executables must remain available to the PlayStation 2 detector.
    if !matches!(*data.get(7)?, 0 | 3) || u32_at(data, 20)? != 1 {
        return None;
    }
    let kind = u16_at(data, 16)?;
    if !matches!(kind, 2 | 3) {
        return None;
    }
    let (entry, table_offset, header_len, entry_len, count, is_64) =
        match (*data.get(4)?, u16_at(data, 18)?) {
            (1, 3) => (
                u64::from(u32_at(data, 24)?),
                u64::from(u32_at(data, 28)?),
                u16_at(data, 40)?,
                u16_at(data, 42)?,
                u16_at(data, 44)?,
                false,
            ),
            (2, 62) => (
                u64_at(data, 24)?,
                u64_at(data, 32)?,
                u16_at(data, 52)?,
                u16_at(data, 54)?,
                u16_at(data, 56)?,
                true,
            ),
            _ => return None,
        };
    if entry == 0
        || header_len != if is_64 { 64 } else { 52 }
        || entry_len != if is_64 { 56 } else { 32 }
        || count == 0
        || count == u16::MAX
        || table_offset < u64::from(header_len)
    {
        return None;
    }
    let table = file_range(data, table_offset, u64::from(entry_len) * u64::from(count))?;
    let mut executable_entry = false;
    let mut interpreter = false;
    for program in table.chunks_exact(usize::from(entry_len)) {
        let segment_type = u32_at(program, 0)?;
        let (flags, offset, address, file_size, memory_size) = if is_64 {
            (
                u32_at(program, 4)?,
                u64_at(program, 8)?,
                u64_at(program, 16)?,
                u64_at(program, 32)?,
                u64_at(program, 40)?,
            )
        } else {
            (
                u32_at(program, 24)?,
                u64::from(u32_at(program, 4)?),
                u64::from(u32_at(program, 8)?),
                u64::from(u32_at(program, 16)?),
                u64::from(u32_at(program, 20)?),
            )
        };
        if segment_type == 1 {
            if file_size > memory_size {
                return None;
            }
            file_range(data, offset, file_size)?;
            if flags & 1 != 0 && entry >= address && entry - address < file_size {
                executable_entry = true;
            }
        } else if segment_type == 3 {
            let path = file_range(data, offset, file_size)?;
            if path.first() != Some(&b'/') || path.last() != Some(&0) || path.len() < 3 {
                return None;
            }
            interpreter = true;
        }
    }
    (executable_entry && (kind == 2 || interpreter)).then_some(())
}

fn file_range(data: &[u8], offset: u64, len: u64) -> Option<&[u8]> {
    let start = usize::try_from(offset).ok()?;
    let end = usize::try_from(offset.checked_add(len)?).ok()?;
    data.get(start..end)
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(data: &mut [u8], offset: usize, value: u16) {
        data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put32(data: &mut [u8], offset: usize, value: u32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(data: &mut [u8], offset: usize, value: u64) {
        data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn pe(is_64: bool) -> Vec<u8> {
        let mut data = vec![0; 1024];
        data[..2].copy_from_slice(b"MZ");
        put32(&mut data, 0x3c, 0x80);
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        put16(&mut data, 0x84, if is_64 { 0x8664 } else { 0x14c });
        put16(&mut data, 0x86, 1);
        let optional_len = if is_64 { 240 } else { 224 };
        put16(&mut data, 0x94, optional_len);
        put16(&mut data, 0x96, 2);
        put16(&mut data, 0x98, if is_64 { 0x20b } else { 0x10b });
        put32(&mut data, 0xa8, 0x1000);
        put32(&mut data, 0xd0, 0x2000);
        put32(&mut data, 0xd4, 512);
        put16(&mut data, 0xdc, 2);
        let section = 0x98 + usize::from(optional_len);
        data[section..section + 5].copy_from_slice(b".text");
        put32(&mut data, section + 8, 512);
        put32(&mut data, section + 12, 0x1000);
        put32(&mut data, section + 16, 512);
        put32(&mut data, section + 20, 512);
        put32(&mut data, section + 36, 0x6000_0020);
        data
    }

    fn elf(is_64: bool, pie: bool) -> Vec<u8> {
        let mut data = vec![0; 512];
        data[..4].copy_from_slice(b"\x7fELF");
        data[4] = if is_64 { 2 } else { 1 };
        data[5] = 1;
        data[6] = 1;
        put16(&mut data, 16, if pie { 3 } else { 2 });
        put16(&mut data, 18, if is_64 { 62 } else { 3 });
        put32(&mut data, 20, 1);
        let (header_len, program_len) = if is_64 {
            put64(&mut data, 24, 0x400000);
            put64(&mut data, 32, 64);
            put16(&mut data, 52, 64);
            put16(&mut data, 54, 56);
            put16(&mut data, 56, if pie { 2 } else { 1 });
            (64, 56)
        } else {
            put32(&mut data, 24, 0x400000);
            put32(&mut data, 28, 52);
            put16(&mut data, 40, 52);
            put16(&mut data, 42, 32);
            put16(&mut data, 44, if pie { 2 } else { 1 });
            (52, 32)
        };
        let program = header_len + if pie { program_len } else { 0 };
        put32(&mut data, program, 1);
        if is_64 {
            put32(&mut data, program + 4, 5);
            put64(&mut data, program + 8, 256);
            put64(&mut data, program + 16, 0x400000);
            put64(&mut data, program + 32, 256);
            put64(&mut data, program + 40, 256);
        } else {
            put32(&mut data, program + 4, 256);
            put32(&mut data, program + 8, 0x400000);
            put32(&mut data, program + 16, 256);
            put32(&mut data, program + 20, 256);
            put32(&mut data, program + 24, 5);
        }
        if pie {
            let path = b"/lib/ld-linux.so.2\0";
            data[192..192 + path.len()].copy_from_slice(path);
            put32(&mut data, header_len, 3);
            if is_64 {
                put64(&mut data, header_len + 8, 192);
                put64(&mut data, header_len + 32, path.len() as u64);
            } else {
                put32(&mut data, header_len + 4, 192);
                put32(&mut data, header_len + 16, path.len() as u32);
            }
        }
        data
    }

    #[test]
    fn identifies_pe32_and_pe32_plus_without_inventing_metadata() {
        for data in [pe(false), pe(true)] {
            assert_eq!(
                Pc.identify(&data),
                Some(RomInfo {
                    platform: "pc",
                    title: String::new(),
                    game_code: String::new(),
                })
            );
        }
        let mut arm64 = pe(true);
        put16(&mut arm64, 0x84, 0xaa64);
        assert!(Pc.identify(&arm64).is_some());
    }

    #[test]
    fn rejects_dll_system_and_non_windows_pe_images() {
        for flags in [0, 0x2002, 0x1002] {
            let mut data = pe(false);
            put16(&mut data, 0x96, flags);
            assert!(Pc.identify(&data).is_none());
        }
        for subsystem in [0, 1, 9, 10, 14, 16] {
            let mut data = pe(false);
            put16(&mut data, 0xdc, subsystem);
            assert!(Pc.identify(&data).is_none());
        }
    }

    #[test]
    fn rejects_invalid_pe_entry_and_truncated_sections() {
        let mut data = pe(false);
        put32(&mut data, 0xa8, 0x3000);
        assert!(Pc.identify(&data).is_none());
        let mut data = pe(false);
        put32(&mut data, 0x178 + 36, 0x4000_0040);
        assert!(Pc.identify(&data).is_none());
        for length in 0..1024 {
            assert!(Pc.identify(&pe(false)[..length]).is_none());
        }
    }

    #[test]
    fn identifies_x86_and_x64_elf_executables_and_dynamic_pie() {
        for is_64 in [false, true] {
            for pie in [false, true] {
                assert!(Pc.identify(&elf(is_64, pie)).is_some());
            }
        }
    }

    #[test]
    fn rejects_shared_objects_mips_elf_and_non_executable_segments() {
        let mut data = elf(true, false);
        put16(&mut data, 16, 3);
        assert!(Pc.identify(&data).is_none());
        let mut data = elf(false, false);
        put16(&mut data, 18, 8);
        assert!(Pc.identify(&data).is_none());
        let mut data = elf(true, false);
        put32(&mut data, 68, 4);
        assert!(Pc.identify(&data).is_none());
    }

    #[test]
    fn rejects_truncated_elf_and_unbounded_header_offsets() {
        for length in 0..512 {
            assert!(Pc.identify(&elf(true, true)[..length]).is_none());
        }
        let mut data = elf(true, true);
        put64(&mut data, 32, u64::MAX);
        assert!(Pc.identify(&data).is_none());
        let mut data = elf(true, true);
        put64(&mut data, 64 + 8, u64::MAX);
        assert!(Pc.identify(&data).is_none());
        let mut data = pe(false);
        put32(&mut data, 0x3c, u32::MAX);
        assert!(Pc.identify(&data).is_none());
    }

    #[test]
    fn rejects_magic_only_and_unrelated_files() {
        for data in [
            b"MZ".as_slice(),
            b"\x7fELF",
            b"not an executable",
            &[0; 4096],
        ] {
            assert!(Pc.identify(data).is_none());
        }
    }
}
