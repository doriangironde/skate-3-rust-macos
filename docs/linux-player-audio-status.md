# Linux player audio: retail graph merged, offline-verified

The downstream retail player-audio work (SK8-ENGINE PR #4, `audio/retail-exact-player-sound`,
head `1415b5d`) is merged into this Linux tree as commit `9d82f4c`, with the Linux input, crash and
build changes kept. The retail evaluator, voice graphs, MixMap, DAC and the game-side
`player_audio` plugin now build and run on Linux (776 `skate-audio-core` unit tests pass). Player
audio is **audible offline**; a real in-game clean-install capture is still outstanding.

## What was merged

* `crates/skate-audio-core`: the fork's full retail implementation (command ring, kind-0/long play,
  XMA pump, DAC, MixMap, authored runtime, grains, DSP, buses).
* `crates/skate-audio-formats`, `crates/skate-data/src/audio`: updated formats, catalog, splice
  banks, grains.
* `crates/skate-game/src/player_audio*`: the game-side plugin — physics observation bridge, the
  board/rolling/trick/contact/footstep/treatment families, `LivePcm` and the Bevy stereo output.
* `crates/skate-game/src/skate_audio.rs`, `audio_check_main.rs`, physics observation and collision
  surface tagging from the same branch.
* Kept from this tree: Linux controller input (`gilrs`), crash reporting, CI, `tools/setup.py`
  headless flow and the TU3 player-image extractor.

## Evidence measured on this machine (Linux, release)

* `cargo test --release -p skate-audio-core --lib`: **776 passed**.
* Headless retail rigs against staged owned assets
  (`SKATE_ASSETS`, no gameplay needed), 6 of 10 pre-existing rigs pass: rolling through the retail
  path, landing impact audible, revert skid level, skid replay, collision controller outputs and
  the voice-arena plateau. Two further component rigs (`wheels`, `rocket`/speed) pass after
  honouring `SKATE_ASSETS` instead of a hard-coded Windows path.
* **Footstep**: a new `headless_footstep_is_audible` rig drives the game's own
  `update_footsteps` path (authored `playercharacter_footstep` pair + per-frame updates) for three
  seconds of walking/pushing. Peak **−14.9 dBFS**, stereo WAV written with `SKATE_HEADLESS_WAV`.
* Known failures: `headless_landing_ladder_matches_retails_curve` fails by design (documented
  open item in `docs/engine-defects.md`); `headless_rolling_speed_and_surface_sweep` stops at
  `0x40040004: no segment covers this address` and is an open investigation.

## Runtime bridge and device output

* `LivePcm` is a bounded interleaved queue (4096 frames): `push` rejects non-finite or mid-frame
  input, discards the oldest complete frames on overflow and returns the discarded sample count.
* The audio callback (`LivePlayback::next`) takes `try_lock`, copies at most 256 whole frames and
  inserts silence on underflow; it performs no disk, FFmpeg, graph or blocking work.
* Underruns and delivery are observable: `LivePcmSnapshot { pushed, consumed, consumed_nonzero,
  underflow }` plus the 2-second `SKATE_PLAYER_AUDIO stream …` log; stale physics observations
  overflow-report and are counted.
* Device errors are non-fatal: with no ALSA device, Bevy logs `No audio device found`, the binary
  still exits 0, and worker failures only log `skate-audio player graph stopped`.
* `cargo test --release -p skate-game --bin skate3rust -- skate_audio::` passes 133 tests
  (29 asset-dependent tests ignored), including the LivePcm continuity/underflow and downmix
  cases.
* Captured from the Linux output monitor (`parec` on the HDMI sink monitor) on this machine:
  the 440 Hz test tone at **−14.0 dBFS**, and an owned retail bank sample
  (`fstep_skateshoe1_sm.abk` slot 168) through the Bevy source at **−4.8 dBFS** — samples reach the
  sound device, not just the sink's bookkeeping.

## The runtime image no longer needs a recomp capture

The catalog reads `<assets>/private/stock/audio-runtime-image` as `g_XXXX.bin` guest pages.
`tools/asset_pipeline/player_image.py` now decrypts the pinned base `default.xex` + TU3
`default.xexp` (owned files only, hashes pinned) and publishes those pages plus a receipt.
`asset_exports.core` runs it automatically during setup when no external image is supplied, so a
normal supported-disc install stages player audio **without any debug environment variable**.

Verified: the static image extracted from the owned XEX + TU3 hashes matches the earlier live
capture for the audio path — the footstep rig renders the same −14.9 dBFS peak with either image
(the live/static difference remains the 14 non-audio words documented below). The extractor tests
(`python3 -m unittest tools.asset_pipeline.test_player_image`) pass, including the owned-data case.

## Owned-data facts (unchanged)

| input | SHA-256 |
|---|---|
| `default.xex` (base, 6,615,040 bytes) | `1db39496585c521d17a2137804f42cf73ebed2b32cac166ec42dbf772f4dcf7f` |
| `default.xexp` (TU3, 1,701,888 bytes) | `eb9ef9109dfa6d940df2e156e7eaeda4603d2b2319ca6451f324b1c27f2b1f4c` |
| static TU3 image (18,546,688 bytes) | `f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4` |
| early live capture (ten 2-MiB chunks) | `73ee34b22475eba6e0893faf1d6e13e3ba85263d746f886cce84814ed537941b` |

The static dump is taken in `LoadContinue` before variable-import patching; exactly 14 words differ
from the live capture (12 `xboxkrnl.exe` variable imports plus two entry stores of `-1` at
`0x830EF9EC`/`0x830EF9F0`). None are audio tables, and the audio runtime produces identical output
from either image.

## Still gated

* **In-game audible capture.** The clean-install path, a gameplay run and a Linux output-device
  capture have not been done yet; only headless PCM and rigs are verified.
* **Rolling surface sweep** (`0x40040004`) and the heavy-landing ladder step (documented upstream
  open item).
* Long/looping play is implemented from the merged branch but not re-verified against a Linux
  playtest.
* Unsupported audio must never stop gameplay; the merged player-audio worker reports failures and
  the game continues (unchanged behaviour to re-verify in game).

Never bundle XEX/XEXP, the decrypted image, audio archives or PCM.
