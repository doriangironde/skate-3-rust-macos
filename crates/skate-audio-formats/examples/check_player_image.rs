//! Offline compatibility probe for a *user-supplied* decrypted guest image and audiofiles.big.
//!
//! This does not extract a XEX or certify its provenance. Do not enable gameplay audio on the
//! strength of this probe alone: a supported XEX+title-update extraction profile is still needed.
//! IMAGE_DIR holds ten 2-MiB g_8200.bin..g_8320.bin chunks (including unmapped padding),
//! or IMAGE is a contiguous 0x11B0000-byte static `rexglue dump-xex` output.
//! cargo run -p skate-audio-formats --example check_player_image -- IMAGE AUDIOFILES.BIG

use skate_audio_formats::{banks, eb};
use std::{collections::HashMap, fs, path::Path};

const BASE: u32 = 0x8200_0000;
const CHUNK: usize = 0x20_0000;
const TABLE: u32 = 0x8302_D4A4;

fn image(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if path.is_file() {
        let bytes = fs::read(path)?;
        if bytes.len() != 0x11B_0000 {
            return Err(format!("image length {}, expected 18546688", bytes.len()).into());
        }
        return Ok(bytes);
    }
    let mut bytes = Vec::with_capacity(10 * CHUNK);
    for base in (0x8200..=0x8320).step_by(0x20) {
        let name = path.join(format!("g_{base:04x}.bin"));
        let chunk = fs::read(&name).map_err(|e| format!("{}: {e}", name.display()))?;
        if chunk.len() != CHUNK {
            return Err(format!(
                "{}: expected {CHUNK} bytes, got {}",
                name.display(),
                chunk.len()
            )
            .into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn bytes_at(image: &[u8], address: u32, length: usize) -> Result<&[u8], String> {
    let offset = address
        .checked_sub(BASE)
        .ok_or(format!("address {address:#x} below image"))? as usize;
    image
        .get(offset..offset.saturating_add(length))
        .ok_or(format!("address {address:#x} outside image"))
}

fn word(image: &[u8], address: u32) -> Result<u32, String> {
    Ok(u32::from_be_bytes(
        bytes_at(image, address, 4)?.try_into().unwrap(),
    ))
}

fn check(image: &[u8], projects: &HashMap<u16, banks::Csi>) -> Result<(), String> {
    // Independent fixed addresses from the fork's measured opcode table and constant pool.
    for (address, expected) in [
        (0x82FD_3600, 0x82B1_BF68), // first evaluator opcode
        (0x82FD_366C, 0x82B1_D240), // evaluator slot 27
        (0x8216_5A10, 0),
        (0x8231_A844, 0x3F80_0000),
        (0x822F_8EA4, 0x4480_0000), // 1024.0f
    ] {
        let actual = word(image, address)?;
        if actual != expected {
            return Err(format!(
                "guest {address:#x}: expected {expected:#x}, got {actual:#x}"
            ));
        }
    }
    // Membership check, not an authenticated slot mapping: duplicates can pass here.
    // Every name pointer and project/id pair must resolve independently through the owned CSI.
    for i in 0..72 {
        let at = TABLE + i * 8;
        let ptr = word(image, at)?;
        let id = bytes_at(image, at + 4, 4)?;
        let project_id = u16::from_be_bytes(id[..2].try_into().unwrap());
        let name_id = u16::from_be_bytes(id[2..].try_into().unwrap());
        let raw = bytes_at(image, ptr, 128)?;
        let end = raw
            .iter()
            .position(|&byte| byte == 0)
            .ok_or(format!("object {i} has no terminated name"))?;
        let name = std::str::from_utf8(&raw[..end]).map_err(|e| e.to_string())?;
        let symbol = projects
            .get(&project_id)
            .and_then(|p| p.symbols.iter().find(|s| s.id == name_id && s.name == name));
        if symbol.is_none() {
            return Err(format!(
                "object {i} {name:?} {project_id:#x}:{name_id:#x} absent from CSI"
            ));
        }
    }
    Ok(())
}

fn projects(
    archive: &[u8],
    entries: &[eb::Entry],
) -> Result<HashMap<u16, banks::Csi>, Box<dyn std::error::Error>> {
    let mut projects = HashMap::new();
    for entry in entries {
        if entry
            .name
            .as_deref()
            .is_some_and(|name| name.ends_with(".csi"))
        {
            if entry.is_compressed() {
                return Err("compressed CSI not supported".into());
            }
            let start = usize::try_from(entry.offset)?;
            let end = start
                .checked_add(entry.uncompressed_size as usize)
                .ok_or("CSI member length overflow")?;
            let member = archive
                .get(start..end)
                .ok_or("CSI member outside archive")?;
            let project = banks::Csi::parse(member)?;
            if projects.insert(project.project_id, project).is_some() {
                return Err("duplicate CSI project".into());
            }
        }
    }
    Ok(projects)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: check_player_image IMAGE_DIR_OR_DUMP AUDIOFILES.BIG".into());
    }
    let image = image(Path::new(&args[0]))?;
    let archive = fs::read(&args[1])?;
    let entries = eb::Archive::parse(&archive)?;
    let projects = projects(&archive, &entries.entries)?;
    check(&image, &projects)?;
    println!(
        "candidate image agrees with {} CSI projects and all 72 object entries; XEX/TU provenance and runtime audio NOT verified",
        projects.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_member_bounds_without_panicking() {
        let entry = eb::Entry {
            offset: u64::MAX,
            compressed_size: 0,
            uncompressed_size: 20,
            name_hash: 0,
            name: Some("bad.csi".into()),
        };
        assert!(projects(&[], &[entry]).is_err());
        let entry = eb::Entry {
            offset: 10,
            ..eb::Entry {
                offset: 0,
                compressed_size: 0,
                uncompressed_size: 20,
                name_hash: 0,
                name: Some("bad.csi".into()),
            }
        };
        assert!(
            projects(&[0; 12], &[entry])
                .unwrap_err()
                .to_string()
                .contains("outside archive")
        );
    }

    #[test]
    fn accepts_only_the_documented_image_layouts() {
        let dir = std::env::temp_dir().join(format!("skate-image-check-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let flat = dir.join("image.bin");
        fs::write(&flat, [0u8; 1]).unwrap();
        assert!(
            image(&flat)
                .unwrap_err()
                .to_string()
                .contains("image length")
        );
        let chunks = dir.join("chunks");
        fs::create_dir(&chunks).unwrap();
        fs::write(chunks.join("g_8200.bin"), [0u8; 1]).unwrap();
        assert!(
            image(&chunks)
                .unwrap_err()
                .to_string()
                .contains("expected 2097152")
        );
        fs::write(&flat, vec![0; 0x11B_0000]).unwrap();
        assert_eq!(image(&flat).unwrap().len(), 0x11B_0000);
        let chunk = vec![0; CHUNK];
        for base in (0x8200..=0x8320).step_by(0x20) {
            fs::write(chunks.join(format!("g_{base:04x}.bin")), &chunk).unwrap();
        }
        assert_eq!(image(&chunks).unwrap().len(), 10 * CHUNK);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_changed_constants_and_wrong_object_identity() {
        let mut image = vec![0; 0x114_0000];
        fn put(image: &mut [u8], address: u32, value: u32) {
            let at = (address - BASE) as usize;
            image[at..at + 4].copy_from_slice(&value.to_be_bytes());
        }
        let projects = HashMap::new();
        assert!(check(&[], &projects).unwrap_err().contains("outside image"));
        put(&mut image, 0x82FD_3600, 0x82B1_BF68);
        assert!(check(&image, &projects).unwrap_err().contains("0x82fd366c"));
        put(&mut image, 0x82FD_366C, 0x82B1_D240);
        put(&mut image, 0x8231_A844, 0x3F80_0000);
        put(&mut image, 0x822F_8EA4, 0x4480_0000);
        for i in 0..72 {
            put(&mut image, TABLE + i * 8, 0x8224_D700);
            let at = (TABLE + i * 8 + 4 - BASE) as usize;
            image[at..at + 4].copy_from_slice(&[0x5c, 0x48, 0x4d, 0x2e]);
        }
        image[(0x8224_D700 - BASE) as usize..(0x8224_D700 - BASE) as usize + 3]
            .copy_from_slice(b"foo");
        let projects = HashMap::from([(
            0x5c48,
            banks::Csi {
                project_id: 0x5c48,
                group_counts: [0, 1, 0],
                pool_offset: 0,
                symbols: vec![banks::Symbol {
                    group: 1,
                    id: 0x4d2e,
                    name: "foo".into(),
                    unknown_a: 0,
                    unknown_b: 0,
                }],
            },
        )]);
        check(&image, &projects).unwrap();
        image[(TABLE + 71 * 8 + 7 - BASE) as usize] ^= 1;
        assert!(check(&image, &projects).unwrap_err().contains("object 71"));
    }
}
