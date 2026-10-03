<p align="center">
  <img src="docs/images/skating-crab.png" alt="Rust crab riding a skateboard" width="480">
</p>

# Skate 3 Rust Engine

A Rust and Bevy skating project built from Skate 3 reverse-engineering research.
Includes skating, tricks, grinds, offboard movement, difficulty settings and
`.skate` map support. Gameplay parity is still a work in progress.

## Mods and agent authoring

Drop mod ZIPs into top-level `mods/` and enable them in the Mods menu. **Point your coding agent at [sdk/AGENTS.md](sdk/AGENTS.md)**
to make a mod. See [package structure](docs/mod-packages.md), [Lua API](docs/lua-modding.md),
[vehicle API](docs/vehicle-sdk.md), [multiplayer mod SDK](docs/multiplayer-mods.md) and [Mixamo workflow](docs/mixamo-vehicle-workflow.md).
Editable examples live in `sdk/examples/`; `tools/package_mod.py` validates and packages them.

## Play

This fork adds native macOS support to
[pinj/skate-3-rust-linux](https://git.ragusa-it.de/pinj/skate-3-rust-linux),
based on [SK8-ENGINE/skate-3-rust-engine](https://github.com/SK8-ENGINE/skate-3-rust-engine).
Build locally using the instructions below. Game assets are not included.

If you have an existing Windows release ZIP, extract it and run
`skate3rust.exe`. Select your Skate 3 Xbox 360 ISO, or select `default.xex` in an
extracted game folder. Keep its `data` folder alongside it. Setup prepares the
skater, animations and all disc maps, then
starts University. The original scoring and session-marker HUD assets are also
exported automatically during setup. No Blender, Python or Rust installation is needed.
ISO extraction needs internet access. The first conversion can take a while.

Use an XInput controller on Windows, or a macOS/Linux-compatible gamepad on
those platforms. Escape opens graphics, difficulty and map
settings. Maps can be switched without restarting the game.

**Skate 3 assets are not included.** Your converted files stay in
the `data` folder beside your executable. Each freshly unpacked copy runs its
own setup; it does not adopt another installation. In-place updates refresh
only changed asset groups.

### Linux and macOS first run

Linux and macOS builds ship no `skate3setup.exe`. On first launch the game finds
`tools/setup.py` above the executable and runs it with the checkout's
`.venv/bin/python` when present, otherwise system `python3`; without
Tk it asks in the terminal for your extracted Skate 3 `default.xex` (keep its
`data` folder beside it). If you installed Python dependencies in the virtual
environment below, run setup explicitly before the first launch:

    .venv/bin/python tools/setup.py --base target/debug/data --game-exe target/debug/skate3rust --source /path/to/default.xex
    ./target/debug/skate3rust

`--source` selects the game and skips the prompt; a folder or `default.xex` is
equivalent. Tk is optional. ISO extraction is Windows-only, so extract the ISO
with a trusted tool and select `default.xex`; setup never downloads or runs an
extractor on Linux or macOS. `--assets DIRECTORY` still selects a prepared asset set
for development. Updating, the character importer and the Steam relay remain
Windows-only.

## Build

### macOS

Supports native Apple Silicon (`aarch64-apple-darwin`) and Intel
(`x86_64-apple-darwin`) builds. macOS uses Metal graphics and the `gilrs`
controller adapter. Apple Silicon audio uses NEON; the original Intel audio
path is retained. See [macOS validation and limitations](docs/macos.md).

Install the Xcode Command Line Tools, Rust stable, and Python **3.11 or newer**.
Homebrew users can install Python and the optional retail-audio decoder with:

    xcode-select --install
    brew install python@3.13 ffmpeg

If Xcode or its Command Line Tools are already installed, skip the first command.
Install Rust using [rustup](https://rustup.rs/), then:

    python3.13 -m venv .venv
    .venv/bin/python -m pip install -r tools/requirements-setup.txt
    sh scripts/build-macos.sh

For a release build use `sh scripts/build-macos.sh --release`.
The default binary is `target/debug/skate3rust`; release builds use
`target/release/skate3rust`. Run the first-run setup command above with your
own extracted `default.xex` and matching `data` folder. Use `target/release`
in both paths if you built with `--release`.

The Metal GPU tests run without game assets:

    cargo test --locked -p skate-game --bin skate3rust --no-default-features metal_ -- --ignored --nocapture

### Linux

Install Rust stable (edition 2024), Python 3, and the Linux build libraries.
On Debian/Ubuntu:

    sudo apt-get update
    sudo apt-get install -y build-essential pkg-config libasound2-dev libudev-dev libwayland-dev python3-venv
    python3 -m venv .venv
    .venv/bin/pip install -r tools/requirements-setup.txt
    cargo build --locked -p skate-game --bin skate3rust --no-default-features

The binary is `target/debug/skate3rust`. Forgejo runs these Linux build and
first-run tests on pushes and pull requests using a Docker runner labeled
`docker`; it does not publish downloads or run game assets in CI.

Retail audio playback (Linux, experimental) needs `ffmpeg` and your own extracted
`data/audio` archives. To test an actual University ambience stream:

    sudo apt-get install -y ffmpeg
    SKATE_AMBIENCE="$PWD/data/disc/data/audio/ambienceresident.big:$PWD/data/disc/data/audio/ambience.big:University" cargo run -p skate-game --bin skate3-audio-check -- 5

In the game, `SKATE_AUDIO_PLAY="<absolute-path>/ambience.big:07_univ_mt_high.sns:5:48000:40"`
plays that entry at startup. This is a manual sound check, not yet gameplay-triggered
skating audio; the imported fork does not implement player-event playback either.
Playback currently decodes at most 128 archive blocks (roughly 14 seconds) into memory;
this is not suitable for uninterrupted full-length ambience or music.

### Windows

Requires Rust with the MSVC toolchain and LLVM installed in its default location.
Run `BUILD.bat` to build, then `PLAY.bat` to launch the test world.
`PLAY.bat` opens your saved map (University by default); use the in-game menu to switch maps, or drag a `.skate` file onto `PLAY.bat`. An XInput controller is required for gameplay;
Escape opens difficulty and graphics settings.

Development builds use a prepared asset set in `assets/private/` or the
installed asset directory. `scripts/Build-Release.ps1` still builds a portable
Windows package with Python 3.13, but Forgejo CI no longer publishes it. The
Windows updater still points at GitHub and is not configured for private Forgejo
releases.

Custom animations and climbing support remain available, but no custom clips
are shipped. The included format-demo map is original procedural content.

Implementation notes are in [`docs/`](docs/). Patched Bevy dependencies and
their licenses are in [`vendor/`](vendor/). This is an unofficial project,
not affiliated with EA.

## Advanced diagnostics

Windows builds support opt-in [performance timeline capture](docs/performance-tracing.md)
through the `--trace` CLI option, including optional GPU pass diagnostics.
