# Linux local-player audio handoff

**State: retail player audio merged and audible offline on Linux; in-game capture still outstanding.** Skate3Recomp cannot reliably reach gameplay on this machine; do not make launching it a prerequisite. `offline_voice` renders one owned mono sample. `grind_instance` can drive eight traced footstep voices through its graph and a host-owned fixture sink with `PCM_OUT`; neither reproduces the retail bus/output driver or game playback.

**2026-09-27 merge.** The downstream retail implementation (SK8-ENGINE PR #4, `audio/retail-exact-player-sound`, head `1415b5d`) is merged in commit `9d82f4c` with the Linux input/crash/build changes kept. `crates/skate-audio-core` is now the fork's full retail core (776 tests pass on Linux); the game-side `player_audio` plugin, physics observation bridge, catalog, MixMap and DAC are in tree. Linux additions on top of the merge: `SKATE_ASSETS` honoured by the wheels/speed rigs, a new `headless_footstep_is_audible` rig, `render_player_audio` loads the MixMap and the three boot utilities, and the TU3 extractor publishes the catalog's `g_XXXX.bin` pages plus a receipt, wired into `asset_exports.core` so setup stages player audio from the owner's own base XEX + TU3 automatically. Measured: footstep peak −14.9 dBFS from both the live capture and the locally extracted static image; 6/10 pre-existing headless rigs pass, plus the two repaired component rigs. Open: in-game audible capture (clean install + gameplay + device capture), the surface sweep (`0x40040004`) and the upstream heavy-landing ladder item. See `docs/linux-player-audio-status.md`. The pinned TU3 static-image extractor is in `tools/asset_pipeline/player_image.py` and is not playback. `SKATE_AUDIO_PLAY` is only a manual decoder/device check. The game's existing synthesized vehicle sounds, Linux controller path, and setup remain in place. Do not claim graph parity from voice-open logs or test counts. `docs/linux-player-audio-status.md` has the measured hashes, the 14-word explanation and the extractor limits; this file is the work queue.

## Scope and source of truth

Play the **local player's** retail trick, landing, board contact, rolling, skid, grind, foot/body/cloth, and speed sounds through the fork's original evaluator/graph, not Bevy sample-selection approximations. Music, ambience, NPC/remote sounds, and Hall of Meat (until that mode exists here) are out of scope. Keep `SKATE_AUDIO_PLAY` and `skate3-audio-check` as diagnostics. Original plan and traces: `audiofork/skate-3-rust-engine/docs/audio-player-sounds-handoff.md` (local ignored fork); source repo `github.com/andrewnakas/skate3-audio`, `phase0-measurements` commit `d3d81ffd634a70299f504b39cccf0595fadd43af` (local `/tmp/skate3-audio`), `docs/PLAN.md` Phase 6, `docs/audio-banks.md`, `probe/traces/`. The imported core is pinned to fork commit `cc41fb980c4bc3505e17ae97f6385347006318e8` in `crates/skate-audio-core/`; do not merge that fork's Windows input/setup changes.

## What exists

- `crates/skate-audio-formats/`: archive, `.abk`/`.csi`, EAAC parsing; `crates/skate-data/src/audio/`: FFmpeg-backed XMA decode; `crates/skate-game/src/skate_audio.rs`: Bevy playback of decoded streams on request, **not** a retail graph player.
- `crates/skate-audio-core/`: fork's evaluator, voice graph builders, DSP, graph and `SndPlayer1` pieces, imported **for offline development only**. No game crate depends on it. A bounded command drain, short kind-0 play, and decoded-PCM `StreamFill` exist. Retail bus initialization/output driver, long/looping play, and gameplay posting are missing. Several ported pieces remain unverified glue; 688 unit tests do not prove retail playback.
- `crates/skate-audio-formats/examples/check_player_image.rs`: non-enabling structural probe of five image cells and 72 object-table entries against the owned archive's CSI. It checks **symbol membership, not expected identity/order per slot or provenance**.
- `docs/linux-player-audio-status.md`: TU3 input/runtime/image hashes, comparison bounds, limitations. `.gitignore` excludes common game audio archives and decrypted image names; still inspect any staged files before committing. Never commit XEX/XEXP, archives, decrypted image, WAV/PCM, or other retail bytes.

## Owned TU3 evidence (no title-update re-download needed)

The user authorized the Title Update 3 already installed in `/mnt/SSD/Games/Skate3/Skate3Recomp-Linux/game/`. The base `default.xex` matches `data/disc/default.xex` (SHA-256 `1db39496585c521d17a2137804f42cf73ebed2b32cac166ec42dbf772f4dcf7f`); the installed `default.xexp` is `eb9ef9109dfa6d940df2e156e7eaeda4603d2b2319ca6451f324b1c27f2b1f4c`. The recomp's own TU validation pins that payload hash, and its isolated run logged successful patch `3.0.0.0 → 3.0.3.0` **before** guest execution.

Private, disposable captures currently live at `/tmp/skate-audio-capture.kksQuK/` (`0700` parent): `first/g_*.bin` and `g_*.bin` are two byte-identical early live captures (ten 2-MiB chunks, guest `0x82000000..0x83400000`, concatenated SHA-256 `73ee34b22475eba6e0893faf1d6e13e3ba85263d746f886cce84814ed537941b`). `static/default_82000000_011B0000.bin` is the 18,546,688-byte TU3 image emitted by that installed runtime's `REXGLUE_DUMP_XEX_IMAGE_DIR` hook (SHA-256 `f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`). Exactly **55 bytes at 14 word addresses** differ between the static image and the same-sized prefix of the early live image; the live tail `0x831b0000..0x83400000` was not compared. Do not silently swap static for live memory: explanations for those differences remain hypotheses. Both image layouts pass `check_player_image` against owned `audiofiles.big`. This is a **development compatibility demonstration**, not an installer-ready extractor or proof of audio output.

If `/tmp` is gone, the installed runtime (not the published `rexglue-sdk v0.8.1.19` CLI) has the dump hook. This is the complete **local-only development** capture recipe; substitute the installed recomp root if different. GDB must be the parent (`ptrace_scope` blocks attach). This copies inputs into a private directory, uses a separate user-data directory, stops before gameplay, and kills **only its own inferior**. The upstream `probe/harness/dump_image.gdb` has an author's hard-coded output path; do not use it unchanged.

```sh
set -eu
SRC=/mnt/SSD/Games/Skate3/Skate3Recomp-Linux
CAP=$(mktemp -d /tmp/skate-audio-capture.XXXXXX)
chmod 700 "$CAP"
mkdir -p "$CAP/app/game/data/webkit" "$CAP/user" "$CAP/home" "$CAP/static"
cp "$SRC/skate3" "$SRC/librexruntime.so" "$CAP/app/"
cp "$SRC/game/default.xex" "$SRC/game/default.xexp" "$CAP/app/game/"
cp "$SRC/game/data/webkit/EAWebkit.xex" "$SRC/game/data/webkit/EAWebkit.xexp" "$CAP/app/game/data/webkit/"
ln -s "$SRC/game/data/audio" "$CAP/app/game/data/audio"
python3 - "$CAP" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
cmd = ['set pagination off', 'set confirm off', 'set debuginfod enabled off',
       'break sub_82EE7828', 'run', 'printf "GUEST BASE %p\\n", $rsi',
       'set $mb = (unsigned char *)$rsi']
for a in range(0x8200, 0x8321, 0x20):
    cmd.append(f'dump binary memory {p}/g_{a:04x}.bin $mb+0x{a:04x}0000 $mb+0x{a+0x20:04x}0000')
cmd += ['kill', 'quit']
(p/'capture.gdb').write_text('\n'.join(cmd) + '\n')
PY
(cd "$CAP/app" && HOME="$CAP/home" XDG_DATA_HOME="$CAP/user" \
  REXGLUE_DUMP_XEX_IMAGE_DIR="$CAP/static" timeout --signal=INT --kill-after=5s 100s \
  gdb -nx -q -batch -x "$CAP/capture.gdb" --args ./skate3 \
  --game_data_root="$CAP/app/game" --user_data_root="$CAP/user" \
  --log_file="$CAP/runtime.log" > "$CAP/capture.log" 2>&1)
rg 'XEX patch applied successfully' "$CAP/runtime.log"
python3 - "$CAP" <<'PY'
from hashlib import sha256
from pathlib import Path
import sys
p = Path(sys.argv[1]); chunks = []
for a in range(0x8200, 0x8321, 0x20):
    chunk = (p/f'g_{a:04x}.bin').read_bytes()
    assert len(chunk) == 0x200000, f'bad chunk {a:04x}'
    chunks.append(chunk)
assert sha256(b''.join(chunks)).hexdigest() == '73ee34b22475eba6e0893faf1d6e13e3ba85263d746f886cce84814ed537941b'
flat = (p/'static/default_82000000_011B0000.bin').read_bytes()
assert len(flat) == 0x11B0000
assert sha256(flat).hexdigest() == 'f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4'
print('TU3 development capture matches pinned image hashes')
PY
```

Require ten 2-MiB chunks, a flat 18,546,688-byte static dump, matching source/runtime hashes above, the logged `3.0.3.0` patch, and probe success. If any step fails, stop; **never** guess offsets or copy these bytes into git. This recipe depends on a separately installed recomp and GDB, so it cannot satisfy the clean-install release gate.

## Reproduce the narrow checks

From this repository (owned disc files are under ignored `data/disc/`; substitute another owned path if needed):

```sh
cargo test -p skate-audio-formats -p skate-data --lib
cargo test -p skate-audio-formats --example check_player_image
cargo test -p skate-audio-core --lib
cargo check -p skate-game
cargo run -q -p skate-audio-formats --example verify_banks -- data/disc/data/audio/audiofiles.big
cargo run -q -p skate-audio-formats --example check_player_image -- \
  /tmp/skate-audio-capture.kksQuK/first data/disc/data/audio/audiofiles.big
```

Observed: 61 formats tests, 27 data tests, 3 probe tests, 678 core tests; owned `audiofiles.big`: 428 members (376 ABK, 9 CSI), 1,509 archive checks agreed, all 72 image object pairs resolved. The fixed footstep trace can be replayed without emitting PCM (the trace file is in the sk8Audio source checkout, locally `/tmp/skate3-audio/probe/traces/fstep_updates_40C02FD0.txt`):

```sh
PAYLOAD='00000000 00000000 00001000 00000000 000061A8 00000000 00000000 00007FFF 00000000 00000000 00000000 00000000 00000001 00000001 00000000 00000000 00000001 00000001 00000000 00000000 00000000 00000000 00000000 00000000 0000000C' \
UPDATE_FILE=/tmp/skate3-audio/probe/traces/fstep_updates_40C02FD0.txt \
UPDATE_EVERY=6 FRAMES=70 GRAPH=1 \
cargo run -q -p skate-audio-core --example grind_instance -- \
  data/disc/data/audio/audiofiles.big /tmp/skate-audio-capture.kksQuK/first \
  fstep_skateshoe1_sm.abk playercharacter_footstep
```

Observed: seven voice opens at frame 59, one at frame 65; each built a nine-module `SndPlayer1 → Rechannel → Resample → HighPassIir2 → LowPassIir2 → Send → Gain → Pan2D1 → Send` graph. The example now drains 49+7 commands with host-owned bus objects, but `GRAPH=1` without `PCM_OUT` only logs commands. With `PCM_OUT=/tmp/footstep.wav FRAMES=220` added to the same environment, it decodes and renders all eight voices to a fixture sink (41,216 six-channel frames; eight stopped slots; 688 core tests pass). This does **not** prove retail bus/output routing, exact random variants or audible gameplay. See the status doc for PCM measurements and the Ghidra finding.

## Next implementation order and stop conditions

1. **Owned-data gate — extractor landed, not a release enablement.** `tools/asset_pipeline/player_image.py` extracts only the pinned base XEX plus TU3, records source hashes, image hash, read-only region offsets and a private receipt, and rejects unknown editions without failing setup. The static digest matched the development capture; the 14 live words are import patches plus two entry stores of `-1` (see the status doc). Ordered slots are `tools/asset_pipeline/player_objects.txt`, not CSI membership. Do not treat `check_player_image` or a receipt as audible audio. The installed recomp/GDB procedure is not a release dependency.
2. **Offline retail-output proof before gameplay.** The bounded variable-size drain, kind-0 play, decoded-PCM fill, 256-frame graph pass and six-channel fixture output now exist; `grind_instance` renders a traced eight-voice footstep using structurally valid **host-owned** buses, not retail bus instances. `PCM_MUTE=1` causes an expected silence error; the normal run ends every SndPlayer1 slot in state 4. This proves only a host-configured event→graph→PCM path. Investigate the retail bus table writer/initializer and output connection using Ghidra on the pinned TU3 static image; `sub_82491108` fetches the indexed bus and `sub_824916E8` only configures existing objects. Replace the fixture with verified bus/output routing and compare to a reference capture where available before claiming parity. Test levels (current raw peak 1.284 and one clipped sample), six-channel layout (currently only one slot nonzero), finite PCM, start/stop and termination. Maintain post/update → tick → drain → graph → output order and fail visibly on unknown handlers. Long/looping play is a separate gate.
3. **Runtime bridge only after an offline PCM proof.** Worker owns banks, voices, decoder and graph. The Bevy audio callback consumes bounded queued 48-kHz stereo PCM and returns silence on underrun—no FFmpeg, disk, graph or blocking lock on callback. Use an explicit tested 5.1→stereo matrix. **Proposed limits, not measured capacity:** 256 queued commands, 32 active voices and two seconds of PCM per voice. Count and drop *new* excess cosmetic starts, preserving existing voices; reserve delivery for stop/release/update controls (or retire the generation), since dropping lifecycle controls can strand voices. Tag commands/blocks with map generation; retire/invalidate on map replacement or shutdown. Device/decoder failures remain nonfatal.
4. **First gameplay trigger is landing, not a speed-based fake footstep.** `crates/skate-game/src/physics/frame.rs` publishes `PhysicalOutputSnapshot.landed` from `PhysicsEvent::Landing` in fixed-tick simulation. That boolean snapshot is **not** a lossless event channel: forward distinct fixed-tick events or sequence IDs so multiple fixed ticks before one audio update cannot lose or duplicate a landing. Obtain the real held `Class_Treatment` constructor payload and per-frame updates from the handoff traces, post once per physical event, and test zero/multiple fixed ticks, pause/replay/map replacement. Footsteps require a real step/animation event; do not invent one from movement speed. Playback stays off the physics thread.
5. **Then loop and broaden.** Implement long play, looping seek, held-message updates, gain/pitch/filter/pan, release/restart; validate an entry→updates→exit grind. Add traced trick/cloth and board/body/speed groups, checking actual archive posts and opens. Probe `wheels.big` and `grains.big` loader paths before claiming full rolling coverage; no Hall of Meat without that game mode. Keep unsupported groups explicit, not replaced with plausible sounds.
6. **Release gate.** Optional versioned audio output from owned disc through Linux setup, receipt revalidation on refresh, no retail assets in build/repo. A clean compatible-disc install must play supported player sounds **without environment variables**; unsupported audio must leave game playable and report why. Capture the Linux output device for audible end-to-end confirmation. Cross-session bit-for-bit identity is not expected because retail uses a global random counter; deterministic offline inputs can be compared bytewise where controlled.

The plan is large; keep each missing stage labelled unverified until it has real-data evidence. Do not modify the already-committed Linux input/crash/build changes as a side effect of audio work.
