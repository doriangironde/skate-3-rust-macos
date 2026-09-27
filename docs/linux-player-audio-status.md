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

## Event-to-output coverage matrix (headless, owned assets)

`headless_group_coverage_is_audible` drives one stage per named group through the game's own
`sound.frame` path and asserts every stage is audible. Run it with `SKATE_AUDIO_TRACE=<file>` for
the per-stage attribution below (posts/updates read from that trace, voices from
`RuntimeStats`):

| stage | frames | posted/held object | voices | peak |
|---|---|---|---|---|
| rolling | 0–90 | `Class_rolling` + `Class_Seams` | 41 | −10.4 dBFS |
| speed (wind/rattle/jet) | 90–210 | `Rolling_Rattle_Class`, `SenseOfSpeed_wind` | 18 | −6.5 dBFS |
| skid (powerslide/revert) | 210–270 | `Class_wheels_skid` (held, driven here) | 6 | −8.8 dBFS |
| board/foot contact | 270–330 | `Class_Seams` + `Class_foot_drag` | 53 | −3.1 dBFS |
| grind | 330–420 | `Class_grind` | 17 | −12.6 dBFS |
| trick (flip + cloth) | 420–480 | `Class_Flips` + `cloth_trick` | 10 | −17.3 dBFS |
| landing | 480–550 | `Class_Treatment` + contact `Class_wheels_skid` | 22 | −3.5 dBFS |
| footstep | 550–670 | `playercharacter_footstep` (held; step updates) | 60 | −11.0 dBFS |
| body/cloth (wipeout) | 670–750 | `c_cloth_falls` + `Class_foot_drag` family | 21 | −16.7 dBFS |

Continuous families (rolling bed, treatment, footsteps, seams) are held objects whose per-frame
updates carry the controls; one-shots (rattle, foot drag, flips, cloth, grind layer) post when
their event fires. Start/stop and voice lifetime are covered by `headless_rolling_through_the_retail_path`,
`headless_revert_skid_level`, `wheels_play_headless`, `rocket_plays_headless` and the voice-arena
plateau test. Short (one-shot) and looping (grind/rolling layers) play are both exercised. No
independent reference capture was available, so levels are measured, not claimed as parity.

## Event delivery and lifecycle

* A completed fixed tick is published exactly once even when presentation runs more than once
  between ticks (`AudioObservationCursor::begin_tick`), and several ticks between two audio
  updates are forwarded in order by the host frame path; a full observation queue is counted and
  logged (`SKATE_PLAYER_AUDIO observation_overflow`) rather than silently dropping.
* Pause repeats the last observation frozen (`retail.paused`) at the sound frame rate, which is
  retail's frozen state and the MixMap's pause input; it never invents a new tick.
* A committed map replacement now also writes `PlayerAudioWorldChanged`; the host clears queued
  PCM, the carried observation, buffered front-end ids and the overflow count so nothing from the
  old world rides into the new one.
* Shutdown: the worker stops when the observation sender is dropped (`next_observation` returns a
  disconnect error); device and decoder failures remain non-fatal and logged.
* Regression tests: `forward_delivers_ticks_pauses_overflows_and_resets`,
  `a_disconnected_observation_source_stops_the_worker`,
  `a_completed_fixed_tick_is_published_exactly_once` — part of 427 passing tests in the game
  binary (125 asset-dependent tests ignored).

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
