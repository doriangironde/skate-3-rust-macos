# macOS port

This fork starts at `b2fce60f22b7c7eeaac8a4a07f3d6e60fce4edfd` from
<https://git.ragusa-it.de/pinj/skate-3-rust-linux>.

## Platform changes

- Select Metal on macOS. Windows and Linux retain Vulkan.
- Share the background `gilrs` controller adapter between Linux and macOS,
  preserving Xbox button layout, trigger ranges, packet publication and hotplug.
- Enable the recovered audio modules on ARM64. The `simd` module uses the
  pinned MIT-licensed `sse2neon` crate and corrects min/max operand selection,
  invalid float-to-integer conversion and dot-product reduction order. Intel
  builds retain Rust's native SSE intrinsics and CPU feature checks.
- Map audio flush/rounding control to ARM FPCR, mask traps during audio work,
  and restore the complete incoming control register after each scope.
- Prefer the checkout's Python virtual environment during first-run setup.
- Find Homebrew's FFmpeg on desktop launches whose PATH contains only system
  tools. Explicit `SKATE_FFMPEG` and available PATH decoders keep precedence.
- Build the setup-time native helper as a Mac `.dylib`, avoiding the Linux
  `.so` cached in the upstream repository.
- Add a native build script, Metal rendering probes, and macOS/Intel/Linux CI.

## Verification

The unmodified game failed to compile for `aarch64-apple-darwin` because
required audio modules were gated to `x86_64`. The port passes native ARM64
and cross-target Intel macOS compile checks, plus all 780 audio unit tests on
Apple Silicon, using Rust 1.99.0.
The setup/player-image suites run 27 tests: 25 pass and two platform or
owned-game-dependent tests are skipped.
All audio diagnostic examples compile on ARM64. The native setup-time C helper
also builds on ARM64 and passes an AES reference-vector check.
With the owned game and matching TU3 payloads, the full player-audio rolling
check produces continuous non-silent stereo at 2, 5 and 9 m/s. It also passes
with a system-only PATH after invalidating a decoded bank, exercising the
Homebrew decoder fallback. Live playback consumes non-zero audio samples
without queue underflows. The FPS overlay regression reports 30, 60 and 120
render updates correctly with 30 Hz physics and paused game time.
The game tests pass 33 controller/input checks, three Rust setup checks and
the shader composition check (two asset-dependent input tests are ignored).
Both headless Metal probes pass on an Apple M5 running macOS 27.2: the retail
material renders the expected `[64, 64, 64, 255]` pixel through MSAA resolve
and readback, and the retail shadow pipeline compiles successfully.

The Metal probes exercise the non-bindless retail material path: Metal does
not provide the storage buffer binding arrays required by Bevy's bindless
materials. Bevy selects its existing fallback automatically. Run the probes
on a Mac with a GPU; hosted CI does not substitute for this check:

```sh
cargo test --locked -p skate-game --bin skate3rust --no-default-features metal_ -- --ignored --nocapture
```

## Requirements and limits

Use Xcode or its Command Line Tools, Rust stable, and Python 3.11 or newer.
Python 3.13 is used by the documented setup and CI. `ffmpeg` is needed for
retail audio decoding. A compatible controller must be connected in macOS.
Game assets must come from your own extracted Xbox 360 game folder; choose
`default.xex` and keep its matching `data` directory. Built-in ISO extraction,
the updater, character importer and Steam relay remain Windows-only.

The ARM64 audio tests validate the port against the existing suite, including
rounding, denormal flushing, kernels, graph evaluation and voice lifecycle.
They do not establish bit-identical output against the upstream x86 C++
recordings: those vectors are absent from this repository. NaN payloads and
platform math-library results may differ. Live gameplay, audible retail
output and physical controller behavior require game assets and a controller.
The Intel target has been cross-checked on this Apple Silicon machine; running
it on Intel hardware is covered by the added CI and has not been verified locally.
