# Linux player audio: TU3 development evidence (not yet enabled)

`SKATE_AUDIO_PLAY` is a decoder/device diagnostic. The player still has no audible gameplay sound through the retail graph. Vehicle synthesis and Linux setup are unchanged.

## Owned-data compatibility measured on this machine

The user's installed skate3recomp has Title Update 3. Its `IsTitleUpdateInstalled` checks pinned payload sizes and SHA-256 (`skate3recomp/src/skate3_title_update_installer.cpp`). The local installed files and extracted owned disc matched:

| input | SHA-256 |
|---|---|
| `default.xex` (base, 6,615,040 bytes) | `1db39496585c521d17a2137804f42cf73ebed2b32cac166ec42dbf772f4dcf7f` |
| `default.xexp` (TU3, 1,701,888 bytes) | `eb9ef9109dfa6d940df2e156e7eaeda4603d2b2319ca6451f324b1c27f2b1f4c` |
| `EAWebkit.xexp` (TU3, 4,096 bytes) | `5d4a308d2a6c768fc27c8b62ccb6661171dc504f8c0011a3e116cf0074e09438` |
| installed `skate3` executable | `8a5107890fa4d34ade92bfa539dfa4ccf06be045a07024051934707e8f8bab2f` |
| installed `librexruntime.so` (has XEX dump hook) | `768d014a1df425b5357767a54abc2c07b6b36a05fd6e8d44800c52b391559823` |

**Reproducible development capture, not an installer recipe.** In a private temporary copy of the installed recomp, with separate game and user-data directories, GDB stopped at the entry of `sub_82EE7828`. Its disassembly confirms guest base is the second SysV argument (`rsi`); the run logged `XEX patch applied successfully: base version: 3.0.0.0, new version: 3.0.3.0` before hitting the breakpoint. `$rsi` was `0x100000000`. Dumping the guest range `0x82000000..0x83400000` in ten 2-MiB `g_*.bin` pieces twice gave byte-identical complete images (SHA-256 of chunks concatenated in ascending address order: `73ee34b22475eba6e0893faf1d6e13e3ba85263d746f886cce84814ed537941b`). These dumps stay outside git. This shows repeatability **for that runtime**, not an independent implementation.

Setting `REXGLUE_DUMP_XEX_IMAGE_DIR` for the same run also wrote a 18,546,688-byte **static** TU3 image (`default_82000000_011B0000.bin`, base `0x82000000`, SHA-256 `f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`). Its entire range `0x82000000..0x831b0000` was compared with exactly the same-sized prefix of the early live capture, after asserting both lengths: 55 differing bytes at 14 word addresses (12 in `0x820005e4..0x820007ec`, two at `0x830ef9ec..0x830ef9f0`). The live capture's remaining `0x831b0000..0x83400000` (padded/other mapped pages) has **no** static counterpart in this comparison. Import fixups/initialization are **hypotheses**, not established explanations for those bytes. The published SDK release `v0.8.1.19` does *not* expose `dump-xex`; this particular installed runtime supplies the dump hook.

`cargo run -p skate-audio-formats --example check_player_image -- IMAGE AUDIOFILES.BIG` passes for both the static image and the live chunks: five measured guest cells agree; all 72 object names and project/id pairs resolve through the archive's nine CSI projects. Directory mode expects ten padded 2-MiB chunks `g_8200.bin`…`g_8320.bin`; flat mode expects exactly 18,546,688 bytes. This check proves **membership**, not the independently expected identity at each slot, provenance, graph output or audible sound.

The owned `audiofiles.big` passes `verify_banks` (428 members, 376 ABK, 9 CSI, 1,509 checks agreed). `cargo test -p skate-audio-formats -p skate-data --lib` passed 61 + 27 tests. The fork's core has been imported at `crates/skate-audio-core` from commit `cc41fb980c4bc3505e17ae97f6385347006318e8` **for offline development only**; its 678 unit tests pass in this workspace and `cargo check -p skate-game` still succeeds. With the owned TU3 live image, its `grind_instance` example and the handoff's 25-word footstep post + 409 traced updates opened seven voices at frame 59 and one at 65, building the expected nine-module graph for each. That example **does not emit PCM**; RNG, command handling, source frames and mix remain unverified.

## Still gated

An installer-usable, pinned extraction from the user's owned executable and TU, independent of an installed recomp, is still missing. Never bundle XEX/XEXP, decrypted image, audio archives or PCM. Do not enable player audio or infer unknown offsets for other editions. Next development boundary is **one bounded offline footstep WAV** from the original evaluator, command drain, stream fill and graph, checked for finite signal, channel layout, voice lifetime and trace ordering. Then compare it with an actual reference capture. Release remains gated on a compatible-disc installation playing local-player sounds without environment variables; unsupported audio must not stop gameplay.
