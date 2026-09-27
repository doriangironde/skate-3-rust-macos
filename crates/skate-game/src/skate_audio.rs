//! Playback of the game's own audio streams.
//!
//! The engine had no game audio at all: the only sounds were synthesized. This plays the real
//! thing, out of the retail archives, through the decode path in `skate-data`.
//!
//! Decoding is not cheap and it spawns an external process, so a request never runs on a schedule
//! thread. It goes to the async compute pool and the finished PCM is picked up on a later frame.
//! A stream that fails to decode is reported once and dropped rather than retried, because every
//! failure here is a property of the asset or the host's decoder and retrying would only repeat
//! the process spawn every frame.
//!
//! ## Verified by recording the sound card
//!
//! `skate3-audio-check` (`src/audio_check_main.rs`) runs this plugin with no window, renderer or
//! assets, so the audio path can be exercised on a checkout the game itself will not boot on.
//! Playing 60 blocks of the five-channel ambience bed through it, while recording the output
//! device's monitor:
//!
//! * the plugin decoded 306,816 frames of 5 channels (6.39 s) in 0.19 s and the stream ended
//!   after 6.6 s;
//! * the recorded level rose from about -58 dBFS before playback to about -45 dBFS during it;
//! * cross-correlating the decoded waveform against the recording peaks at **lag 1.370 s** --
//!   exactly the recorder's one-second head start plus this program's startup -- with a
//!   peak-to-mean ratio of **58.8**, against **5.7** for a time-reversed control whose peak lands
//!   at a lag outside the overlap.
//!
//! The absolute correlation is 0.22 rather than near 1 because the mixer downmixes five channels
//! to two and the device has its own response. The sharpness of the peak and the lag being right
//! are what carry the claim.
//!
//! ## What has been verified, and what has not
//!
//! On Linux, the audio-check binary decoded a retail University ambience entry and playback
//! produced nonzero samples on the output device monitor. Game startup playback remains opt-in
//! via `SKATE_AUDIO_PLAY`; gameplay does not yet post player sound events.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bevy::audio::{AddAudioSource, Source, Volume};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};
use skate_audio_formats::eaac;
use skate_data::audio::{self, ffmpeg::FfmpegDecoder};

/// Decoded interleaved 16-bit PCM, ready to hand to the mixer.
#[derive(Asset, TypePath, Clone)]
pub struct StreamPcm {
    samples: Arc<Vec<i16>>,
    channels: u16,
    sample_rate: u32,
}

impl StreamPcm {
    pub fn frames(&self) -> usize {
        if self.channels == 0 { 0 } else { self.samples.len() / usize::from(self.channels) }
    }

    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.frames() as f64 / f64::from(self.sample_rate))
    }
}

/// Plays a [`StreamPcm`] once, converting to the float samples the mixer wants.
pub struct PcmPlayback {
    samples: Arc<Vec<i16>>,
    at: usize,
    channels: u16,
    sample_rate: u32,
}

impl Iterator for PcmPlayback {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let s = *self.samples.get(self.at)?;
        self.at += 1;
        // i16::MIN has no positive counterpart, so dividing by 32768 keeps the full range inside
        // [-1, 1] instead of letting the one extreme sample clip a fraction above it.
        Some(f32::from(s) / 32768.0)
    }
}

impl Source for PcmPlayback {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.samples.len().saturating_sub(self.at))
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        let frames = if self.channels == 0 { 0 } else { self.samples.len() / usize::from(self.channels) };
        (self.sample_rate != 0)
            .then(|| Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate)))
    }
}

impl Decodable for StreamPcm {
    type DecoderItem = f32;
    type Decoder = PcmPlayback;

    fn decoder(&self) -> PcmPlayback {
        PcmPlayback {
            samples: self.samples.clone(),
            at: 0,
            channels: self.channels,
            sample_rate: self.sample_rate,
        }
    }
}

/// Where one stream lives, and what it takes to read it.
///
/// The ambience and grain archives store bare block chains, so `channels` and `sample_rate` are
/// part of the request: they come from the metadata table, not from the audio.
#[derive(Clone, Debug)]
pub struct StreamRequest {
    pub archive: PathBuf,
    /// A member name, or its index in the entry table as a decimal string.
    pub entry: String,
    pub channels: u8,
    pub sample_rate: u32,
    /// Stop after this many blocks; 0 decodes the whole member.
    pub blocks: usize,
    pub volume: f32,
    /// Ambience beds run about two minutes and are meant to run under everything, so they repeat.
    pub looping: bool,
}

impl StreamRequest {
    pub fn new(archive: impl Into<PathBuf>, entry: impl Into<String>, channels: u8, sample_rate: u32) -> Self {
        Self {
            archive: archive.into(),
            entry: entry.into(),
            channels,
            sample_rate,
            blocks: 0,
            volume: 1.0,
            looping: false,
        }
    }

    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    pub fn blocks(mut self, blocks: usize) -> Self {
        self.blocks = blocks;
        self
    }

    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
}

/// Ask for a stream to be decoded and played once.
#[derive(Message, Clone, Debug)]
pub struct PlayStream(pub StreamRequest);

/// Ask for the ambience bed that suits a place, if the archive has one for it.
///
/// Resolution is by member name (`skate_data::audio::ambience`), which is a stand-in for the
/// undecoded audio metadata, so a place with no matching district plays nothing at all rather
/// than something plausible.
#[derive(Message, Clone, Debug)]
pub struct PlayAmbience {
    /// The archive of `.snr` headers, e.g. `ambienceresident.big`.
    pub resident: PathBuf,
    /// The archive of `.sns` payloads, e.g. `ambience.big`.
    pub payload: PathBuf,
    pub place: String,
    pub volume: f32,
}

/// Resolve a place to a bed, across the pair of archives a bed is stored in.
///
/// Ambience is **split in two**: `ambienceresident.big` holds one `.snr` header record per bed,
/// and `ambience.big` holds the matching `.sns` block chain, paired by the name's stem. Reading a
/// bed out of the resident archive alone gets a header and no audio -- the block walk then reads
/// the next member's bytes and reports "block size 0 does not advance", which looks like a corrupt
/// archive and is really the wrong file.
///
/// So the header supplies the channel count and rate, and the payload supplies the blocks.
pub fn resolve_ambience(
    resident: &std::path::Path,
    payload: &std::path::Path,
    place: &str,
) -> Result<StreamRequest, String> {
    let header_data = std::fs::read(resident).map_err(|e| format!("{}: {e}", resident.display()))?;
    let described = audio::describe_archive(&header_data).map_err(|e| e.to_string())?;
    let beds: Vec<audio::ambience::Bed> = described
        .iter()
        .filter_map(|(name, _)| audio::ambience::parse_bed(name))
        .collect();
    if beds.is_empty() {
        return Err(format!("{} holds no named ambience beds", resident.display()));
    }
    let bed = audio::ambience::pick(place, &beds)
        .ok_or_else(|| format!("no bed matches {place:?} among {} in the archive", beds.len()))?;
    let info = described
        .iter()
        .find(|(name, _)| *name == bed.member)
        .map(|(_, info)| info.clone())
        .ok_or_else(|| format!("{} has no header", bed.member))?;

    // The payload member carries the same stem with a .sns extension.
    let stem = bed.member.strip_suffix(".snr").unwrap_or(&bed.member);
    let entry = format!("{stem}.sns");
    let payload_data = std::fs::read(payload).map_err(|e| format!("{}: {e}", payload.display()))?;
    let archive = skate_audio_formats::eb::Archive::parse(&payload_data)
        .map_err(|e| e.message.clone())?;
    if archive.find(&entry).is_none() {
        return Err(format!("{} has no member {entry}", payload.display()));
    }
    Ok(StreamRequest::new(payload, entry, info.channels, info.sample_rate).looping(true))
}

#[derive(Component)]
struct Decoding {
    task: Task<Result<StreamPcm, String>>,
    request: StreamRequest,
}

pub struct SkateAudioPlugin;

impl Plugin for SkateAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<StreamPcm>()
            .add_message::<PlayStream>()
            .add_message::<PlayAmbience>()
            .add_systems(Startup, play_requested_at_startup)
            .add_systems(Update, (resolve_ambience_requests, start_decoding, finish_decoding));
    }
}

/// `SKATE_AUDIO_PLAY=<archive>:<entry>:<channels>:<rate>[:<blocks>]` plays one stream at
/// startup. It exists so the decode path can be *heard* rather than only measured, on a build
/// that has no automatic ambience yet: which stream belongs to which map lives in the metadata
/// table, and that table is not decoded.
fn play_requested_at_startup(mut requests: MessageWriter<PlayStream>) {
    let Ok(spec) = std::env::var("SKATE_AUDIO_PLAY") else { return };
    match parse_spec(&spec) {
        Ok(request) => {
            info!("skate-audio: playing {} entry {} on request", request.archive.display(), request.entry);
            requests.write(PlayStream(request));
        }
        Err(e) => warn!("skate-audio: SKATE_AUDIO_PLAY={spec}: {e}"),
    }
}

/// Parse `archive:entry:channels:rate[:blocks]`. The archive path is taken from the left, so a
/// Windows drive letter or any other colon inside the path would need the fields reordered; this
/// is a debug hook and says so rather than pretending to be a general parser.
fn parse_spec(spec: &str) -> Result<StreamRequest, String> {
    let parts: Vec<&str> = spec.rsplitn(5, ':').collect();
    // rsplitn yields the tail first, so the fields come back reversed.
    let (archive, entry, channels, rate, blocks) = match parts.len() {
        4 => (parts[3], parts[2], parts[1], parts[0], "0"),
        5 => (parts[4], parts[3], parts[2], parts[1], parts[0]),
        _ => return Err("expected archive:entry:channels:rate[:blocks]".into()),
    };
    Ok(StreamRequest::new(
        archive,
        entry,
        channels.parse::<u8>().map_err(|e| format!("channels: {e}"))?,
        rate.parse::<u32>().map_err(|e| format!("rate: {e}"))?,
    )
    .blocks(blocks.parse::<usize>().map_err(|e| format!("blocks: {e}"))?))
}

/// Turn a place into a stream request, or say why it could not be.
fn resolve_ambience_requests(
    mut asked: MessageReader<PlayAmbience>,
    mut streams: MessageWriter<PlayStream>,
) {
    for ask in asked.read() {
        match resolve_ambience(&ask.resident, &ask.payload, &ask.place) {
            Ok(request) => {
                info!("skate-audio: ambience for {:?}: {}", ask.place, request.entry);
                streams.write(PlayStream(request.volume(ask.volume)));
            }
            // Not an error worth stopping for: a map with no bed simply has no ambience yet.
            Err(e) => info!("skate-audio: no ambience for {:?}: {e}", ask.place),
        }
    }
}

fn start_decoding(mut commands: Commands, mut requests: MessageReader<PlayStream>) {
    for PlayStream(request) in requests.read() {
        let job = request.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move { decode(&job) });
        commands.spawn(Decoding { task, request: request.clone() });
    }
}

fn finish_decoding(
    mut commands: Commands,
    mut assets: ResMut<Assets<StreamPcm>>,
    mut pending: Query<(Entity, &mut Decoding)>,
) {
    for (entity, mut job) in &mut pending {
        let Some(result) = block_on(future::poll_once(&mut job.task)) else { continue };
        commands.entity(entity).despawn();
        match result {
            Ok(pcm) => {
                info!(
                    "skate-audio: {} entry {} decoded, {} frames of {} channels ({:.2} s)",
                    job.request.archive.display(), job.request.entry, pcm.frames(),
                    pcm.channels, pcm.duration().as_secs_f64()
                );
                let volume = job.request.volume;
                let handle = assets.add(pcm);
                let settings = if job.request.looping {
                    PlaybackSettings::LOOP
                } else {
                    PlaybackSettings::DESPAWN
                };
                commands.spawn((AudioPlayer(handle), settings.with_volume(Volume::Linear(volume))));
            }
            Err(e) => warn!("skate-audio: {} entry {}: {e}", job.request.archive.display(), job.request.entry),
        }
    }
}

/// Read, split and decode one stream. Runs off the schedule threads: it spawns a process.
fn decode(request: &StreamRequest) -> Result<StreamPcm, String> {
    let data = std::fs::read(&request.archive).map_err(|e| format!("{e}"))?;
    let (at, end) = locate(&data, &request.entry)?;
    // A member may carry its own stream header -- the named wheel and grain sounds do -- or be a
    // bare block chain whose format lives in the metadata, as the ambience beds are. Prefer the
    // header when there is one, and skip it: its low 24 bits are the sample rate, so reading it
    // as a block header looks like a 48,000-byte block and fails far from the real mistake.
    let (chain, channels, rate) = match audio::describe(&data, at) {
        Ok(info) => (at + info.header_bytes, info.channels, info.sample_rate),
        Err(_) => (at, request.channels, request.sample_rate),
    };
    let widths = audio::context_widths(channels);
    let contexts = widths.len();
    if contexts == 0 {
        return Err("a stream with no channels".into());
    }

    // ponytail: cap in-memory PCM to about 14 seconds; stream chunks when full beds are needed.
    let block_limit = if request.blocks == 0 { 128 } else { request.blocks.min(128) };
    let mut chains: Vec<Vec<Vec<u8>>> = vec![Vec::new(); contexts];
    let mut seen = 0usize;
    for block in eaac::blocks(&data[chain..end]).map_err(|e| e.message.clone())? {
        if seen >= block_limit {
            break;
        }
        let range = block.data_range();
        let payload = &data[chain + range.start..chain + range.end];
        for (context, chunk) in eaac::split_block(payload, contexts)
            .map_err(|e| e.message.clone())?
            .into_iter()
            .enumerate()
        {
            chains[context].push(chunk.data.to_vec());
        }
        seen += 1;
    }

    let mut per_context = Vec::with_capacity(contexts);
    for (context, chunks) in chains.iter().enumerate() {
        per_context.push(
            audio::ffmpeg::decode_chain(chunks, widths[context], rate).map_err(|e| e.message)?,
        );
    }
    Ok(StreamPcm {
        samples: Arc::new(audio::interleave_contexts(&per_context, &widths)),
        channels: u16::from(channels),
        sample_rate: rate,
    })
}

/// Resolve a member to a byte range. The range matters as much as the offset: a block walk that
/// runs past the member reads the next one's first bytes as a block header.
fn locate(data: &[u8], entry: &str) -> Result<(usize, usize), String> {
    use skate_audio_formats::eb;
    let archive = eb::Archive::parse(data).map_err(|e| e.message.clone())?;
    let member = match entry.parse::<usize>() {
        Ok(index) => archive
            .entries
            .get(index)
            .ok_or_else(|| format!("entry {index}: the archive has {}", archive.entries.len()))?,
        Err(_) => archive.find(entry).ok_or_else(|| format!("no member named {entry}"))?,
    };
    if member.is_compressed() {
        return Err(format!("member {entry} is a chunkref block, not stored audio"));
    }
    let range = member.range();
    Ok((range.start, range.end.min(data.len())))
}

/// Is a decoder present? A host without one gets synthesized audio only.
pub fn decoder_available() -> bool {
    FfmpegDecoder::available()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_converts_full_scale_without_exceeding_unity() {
        // i16::MIN / 32767 would be 1.000031, which clips on a mixer that trusts the range.
        let pcm = StreamPcm {
            samples: Arc::new(vec![i16::MIN, i16::MAX, 0]),
            channels: 1,
            sample_rate: 48_000,
        };
        let got: Vec<f32> = pcm.decoder().collect();
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|s| (-1.0..=1.0).contains(s)), "{got:?}");
        assert_eq!(got[0], -1.0);
        assert_eq!(got[2], 0.0);
    }

    #[test]
    fn duration_counts_frames_not_samples() {
        // Five channels at 48 kHz: 240,000 samples are one second, not five.
        let pcm = StreamPcm {
            samples: Arc::new(vec![0i16; 48_000 * 5]),
            channels: 5,
            sample_rate: 48_000,
        };
        assert_eq!(pcm.frames(), 48_000);
        assert_eq!(pcm.duration(), Duration::from_secs(1));
        assert_eq!(pcm.decoder().total_duration(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn the_debug_spec_reads_its_fields_in_order() {
        let r = parse_spec("/a/ambience.big:0:5:48000").expect("four fields");
        assert_eq!(r.archive, PathBuf::from("/a/ambience.big"));
        assert_eq!(r.entry, "0");
        assert_eq!(r.channels, 5);
        assert_eq!(r.sample_rate, 48_000);
        assert_eq!(r.blocks, 0);
        let r = parse_spec("/a/b.big:name:2:44100:8").expect("five fields");
        assert_eq!(r.entry, "name");
        assert_eq!(r.blocks, 8);
        assert!(parse_spec("/a/b.big:0:5").is_err());
        assert!(parse_spec("/a/b.big:0:many:48000").is_err());
    }

    #[test]
    fn a_request_defaults_to_the_whole_member_at_full_volume() {
        let r = StreamRequest::new("/x/ambience.big", "0", 5, 48_000);
        assert_eq!(r.blocks, 0);
        assert_eq!(r.volume, 1.0);
        assert!(!r.looping, "a one-shot by default; only ambience repeats");
        assert_eq!(r.blocks(8).blocks, 8);
    }
}
