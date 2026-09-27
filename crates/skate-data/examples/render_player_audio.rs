//! Replay exact posted/update payloads through the authored evaluator, voice graphs and DAC.
//! PCM stays in the owner's private output path; this is not an asset export for redistribution.
use skate_audio_core::authored::AuthoredRuntime;
use skate_data::audio::catalog::PlayerAudioCatalog;
use std::{io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assets = PathBuf::from(std::env::args().nth(1).ok_or("need assets directory")?);
    let cache = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("Skate3RustEngine/audio-pcm-cache"));
    let selected = std::env::var("BANK").ok();
    let archive = assets.join("private/stock/data/audio/audiofiles.big");
    let mut catalog = if let Some(name) = &selected {
        PlayerAudioCatalog::load_banks(
            &archive,
            &assets.join("private/stock/audio-runtime-image"),
            cache.as_deref(),
            &[name],
        )?
    } else {
        PlayerAudioCatalog::from_assets(&assets, cache.as_deref())?
    };
    // The same preparation the game's worker runs: splice collision banks, the three persistent
    // boot utilities, then the authored MixMap -- without these the graph opens but stays silent.
    catalog.load_splice_banks(&archive, cache.as_deref())?;
    eprintln!(
        "catalog: {} samples ({} cached)",
        catalog.samples.len(),
        catalog.cache_hits
    );
    // `load_banks` (the BANK selector) does not read the MixMap; `from_assets` does. Fall back to
    // the staged file so both paths replay with the authored mixer.
    let mixmap = catalog
        .mixmap
        .clone()
        .or_else(|| std::fs::read(assets.join("private/stock/data/audio/MixMapSK8.mxb")).ok());
    let mut runtime = AuthoredRuntime::new(catalog.guest, catalog.projects, catalog.banks)?;
    for sample in catalog.samples {
        let base = runtime.bank_base(&sample.bank).ok_or("missing bank")?;
        runtime.insert_pcm(base + sample.header_offset, sample.pcm)?;
    }
    let utility_relocations = [
        (3, 0x1c),
        (7, 0x2c),
        (11, 0x3c),
        (15, 0x4c),
        (19, 0x5c),
        (23, 0x6c),
        (27, 0x7c),
    ];
    let utility_packet = [0u32; 28];
    for object in ["c_emitter_utility", "Start_up_Play_ctl", "c_foley_utility"] {
        if runtime.has_object(object) {
            runtime.post_relocated(object, &utility_packet, &utility_relocations)?;
        }
    }
    match &mixmap {
        Some(mxb) => runtime.load_mixmap(mxb)?,
        None => return Err("MixMapSK8.mxb is not staged in the assets".into()),
    }
    let object = std::env::var("OBJECT").unwrap_or_else(|_| "Class_grind".into());
    let payload = std::env::var("PAYLOAD")
        .unwrap_or_else(|_| "0 7FFF 0 0 0 61A8 0 1388 400 3 0 4E20 0 0 0 0 0".into());
    let parse = |s: &str| -> Result<Vec<u32>, std::num::ParseIntError> {
        s.split_whitespace()
            .map(|w| u32::from_str_radix(w, 16))
            .collect()
    };
    let handle = runtime.post(&object, &parse(&payload)?)?;
    let updates: Vec<Vec<u32>> = match std::env::var("UPDATE_FILE") {
        Ok(path) => std::fs::read_to_string(path)?
            .lines()
            .map(parse)
            .collect::<Result<_, _>>()?,
        Err(_) => Vec::new(),
    };
    let frames = std::env::var("FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600usize);
    let every = std::env::var("UPDATE_EVERY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6usize)
        .max(1);
    let mut file = std::env::var_os("PCM_OUT")
        .map(std::fs::File::create)
        .transpose()?
        .map(std::io::BufWriter::new);
    let mut nonzero = 0;
    for frame in 0..frames {
        if frame > 0 && (frame - 1) % every == 0 {
            if let Some(payload) = updates.get((frame - 1) / every) {
                runtime.redeliver(handle, payload)?;
            }
        }
        let pcm = runtime
            .pump_once()
            .map_err(|e| format!("block {frame}: {e}"))?;
        assert_eq!(pcm.len(), 256 * 6);
        nonzero += usize::from(pcm.iter().any(|s| s.abs() > 1e-8));
        if let Some(file) = &mut file {
            for sample in pcm {
                file.write_all(&sample.to_le_bytes())?;
            }
        }
        if frame % 100 == 0 {
            eprintln!("block {frame}: {:?}", runtime.stats());
        }
    }
    runtime.release(handle)?;
    if let Some(file) = &mut file {
        file.flush()?;
    }
    println!("{:?}; {nonzero} nonzero blocks", runtime.stats());
    if nonzero == 0 {
        return Err("authored graph produced no audible PCM".into());
    }
    Ok(())
}
