//! The voice device's open, `sub_824A3140`: turn a bank sample and its descriptor into a voice
//! object over a built module graph, configure the sound player, route the sends and post the
//! panner's angles.
//!
//! **Unverified new work.** The open and its helpers run on the game thread and have no C++ bodies;
//! each is transcribed from its lifted body and unit-tested. `examples/open_voice.rs` runs the open
//! over the dumped image's real classes.
//!
//! | function | here |
//! |---|---|
//! | `sub_824A3140`, the device's `vtable+0` | [`open_voice_graph`] |
//! | `sub_824A2908`, a voice object's initial fields | [`init_voice_object`] |
//! | `sub_82491108`, a bus by index | [`bus_for`] |
//! | `sub_824916E8`'s entry, before it builds the bus | [`DeviceHost::create_bus`] takes over |
//! | the module `vtable+4` entries it calls | [`configure`] |
//! | `sub_82B31370`, `Send`'s | [`send_configure`] |
//! | `sub_82B23798`, `GainFader`'s | [`fader_configure`] |
//! | `sub_82B328E8`, `SndPlayer1`'s, ids 0 and 5 only | [`sndplayer_configure`] |
//!
//! **The voice graph it builds** (the `msgs1` trace's 119-build shape):
//! `SndPlayer1 → Rechannel → Resample → HighPassIir2 → LowPassIir2 → [Send] → Gain → [Send] →
//! Pan2D1 (6 channels) → Send (6 channels)`. The first `Send` is there when the bus manager's
//! `[[+52]]` is non-zero; the second when a routing record of 4096, 8192 or 16384 names an effect
//! bus.
//!
//! The voice object (104 bytes): `+0` vtable `0x822FBCA8`, `+4` player, `+8` `SndPlayer1`, `+12`
//! `Resample`, `+16` high-pass, `+20` low-pass, `+24` first send, `+28` gain, `+32` panner, `+52`
//! the play counter, `+56` the sample's duration (a double), `+68` channels, `+72`/`+76`/`+80`/`+88`
//! the effect routing, `+84` second send, `+92` the output bus, `+96` a mode word.
//!
//! **The frame.** The open's 624-byte frame is guest memory, because the builder and the configure
//! entries read the descriptors and parameter blocks it holds. Its register saves and the `fcfid`
//! scratch spills at `+80` are not written.

use crate::bitstream::unpack_stream_header;
use crate::classes::{self, BUS_ROOT, DEFAULT_BUS, REGISTERED, SEND_SLOT, TAG_INTEGER, TAG_POINTER, TAG_SINGLE, TAG_STRING};
use crate::fp::{add_single, fcfid, fctiwz_low_word, frsp, load_single, mul_single, store_single, sub_single};
use crate::mathlib::Trig;
use crate::modules::{self, HALF, ONE, PAN_FRONT, PAN_SIDE, SYSTEM, ZERO};
use crate::patch::Heap;
use crate::{Error, Guest, Result};

/// `stwu r1,-624(r1)`.
pub const FRAME_BYTES: u32 = 624;
/// `li r4,104`: the voice object's size.
pub const VOICE_BYTES: u32 = 104;
/// `lis -32208 ; addi -17240`.
pub const VOICE_VTABLE: u32 = 0x822F_BCA8;
/// The voice vtable's `+0`, which the open calls when the graph cannot be built. Not ported.
pub const VOICE_RELEASE: u32 = 0x82B1_E458;
/// `lis -31987 ; lwz -532`: the bus manager.
pub const BUS_MANAGER: u32 = 0x830C_FDEC;
/// `lis -31987 ; lwz -572`: the object whose `[[+688]+48]` and `[[+688]+40]` are the 512 and 2048
/// routing buses.
pub const ROUTING_ROOT: u32 = 0x830C_FDC4;
/// `lis -32208 ; addi -31232`: the rodata block the open keeps at `frame+88`.
pub const RODATA: u32 = 0x822F_8600;
/// `+256`: a zero double.
pub const ZERO_DOUBLE: u32 = RODATA + 256;
/// `+664`: 1/32767.
pub const LEVEL_SCALE: u32 = RODATA + 664;
/// `+744`: 360/65536, a descriptor angle to degrees.
pub const ANGLE_SCALE: u32 = RODATA + 744;
/// `+2736`: 4194304.0, where `SndPlayer1`'s play counter wraps.
pub const PLAY_LIMIT: u32 = RODATA + 2736;
/// `lis -32243 ; lfs 29160`: 0.01.
pub const PERCENT: u32 = 0x820D_71E8;
/// `lis -32219 ; addi -5868`: "Aems Player", the player's `+20`.
pub const PLAYER_NAME: u32 = 0x8224_E914;
/// `lis 16709 ; ori 19795`: "AEMS".
pub const AEMS: u32 = 0x4145_4D53;

/// Command handlers the open and the configure entries enqueue.
pub const COMMAND_PLAYER_FLOAT: u32 = 0x82B4_9268; // leaves::publish_float, verified
pub const COMMAND_STAMP: u32 = 0x82B4_63A8; // leaves::stamp_slot, verified
pub const COMMAND_SEND_BUS: u32 = 0x82B3_1680; // voices::repoint_link, verified
pub const COMMAND_SEND_NAME: u32 = 0x82B3_1720;
pub const COMMAND_SEND_2: u32 = 0x82B3_15E0;
pub const COMMAND_PLAY: u32 = 0x82B3_2DC8; // gate 1
pub const COMMAND_FADER: u32 = 0x82B2_3828;

/// Module `vtable+4` entries.
pub const CONFIGURE_NONE: u32 = 0x82B6_1BB8;
pub const CONFIGURE_SEND: u32 = 0x82B3_1370;
pub const CONFIGURE_FADER: u32 = 0x82B2_3798;
pub const CONFIGURE_SNDPLAYER: u32 = 0x82B3_28E8;

/// The class slots, relative to `r23 = 0x83082904`.
const GAIN_SLOT: u32 = REGISTERED + 17;
const HIGHPASS_SLOT: u32 = REGISTERED + 21;
const LOWPASS_SLOT: u32 = REGISTERED + 25;
const PAN_SLOT: u32 = REGISTERED + 29;
const SNDPLAYER_SLOT: u32 = REGISTERED + 37;
const RECHANNEL_SLOT: u32 = REGISTERED + 41;
const RESAMPLE_SLOT: u32 = REGISTERED + 45;

const _: () = {
    const fn lis(hi: i32, lo: i32) -> u32 {
        (((hi & 0xFFFF) << 16) as u32).wrapping_add(lo as u32)
    }
    assert!(VOICE_VTABLE == lis(-32208, -17240) && BUS_MANAGER == lis(-31987, -532) && ROUTING_ROOT == lis(-31987, -572));
    assert!(RODATA == lis(-32208, -31232) && PERCENT == lis(-32243, 29160) && PLAYER_NAME == lis(-32219, -5868));
    assert!(AEMS == (16709 << 16) | 19795);
    assert!(COMMAND_PLAYER_FLOAT == lis(-32075, -28056) && COMMAND_STAMP == lis(-32076, 25512));
    assert!(COMMAND_SEND_BUS == lis(-32077, 5760) && COMMAND_SEND_NAME == lis(-32077, 5920) && COMMAND_SEND_2 == lis(-32077, 5600));
    assert!(COMMAND_PLAY == lis(-32077, 11720) && COMMAND_FADER == lis(-32078, 14376));
    assert!(PAN_SLOT == 0x8308_2904 && GAIN_SLOT == 0x8308_28F8 && RESAMPLE_SLOT == 0x8308_2914);
};

/// What the open cannot do itself.
pub trait DeviceHost {
    /// `sub_824916E8` from `0x82491748` on: build bus `index` of `manager`, whose created flag
    /// ([`bus_for`] sets it) is already 1. It reads tuning records by hashed id and makes indirect
    /// calls, so it is the host's.
    fn create_bus(&mut self, g: &mut Guest, manager: u32, index: u32) -> Result<()>;
}

/// A host with no buses to build.
pub struct NoBuses;

impl DeviceHost for NoBuses {
    fn create_bus(&mut self, _g: &mut Guest, _manager: u32, _index: u32) -> Result<()> {
        Err(Error::new(0x8249_16E8, "bus creation is not ported"))
    }
}

/// `sub_824A2908`.
pub fn init_voice_object(g: &mut Guest, voice: u32) -> Result<()> {
    let one = load_single(g, ONE)?;
    let zero_double = g.u64(ZERO_DOUBLE)?; // lfd f11,256(r7)
    let zero = load_single(g, ZERO)?;
    let minus_one = load_single(g, modules::MINUS_ONE)?;
    g.set_u32(voice + 4, 0)?;
    store_single(g, voice + 36, one)?;
    g.set_u32(voice, VOICE_VTABLE)?;
    store_single(g, voice + 40, one)?;
    g.set_u32(voice + 8, 0)?;
    store_single(g, voice + 44, one)?;
    g.set_u32(voice + 12, 0)?;
    store_single(g, voice + 48, zero)?;
    g.set_u32(voice + 16, 0)?;
    store_single(g, voice + 52, minus_one)?;
    g.set_u32(voice + 20, 0)?;
    g.set_u64(voice + 56, zero_double)?;
    g.set_u32(voice + 24, 0)?;
    store_single(g, voice + 80, zero)?;
    for offset in [28, 32, 64] {
        g.set_u32(voice + offset, 0)?;
    }
    g.set_u8(voice + 68, 0)?;
    g.set_u8(voice + 69, 1)?;
    for offset in [72, 76, 84, 88, 92] {
        g.set_u32(voice + offset, 0)?;
    }
    g.set_u32(voice + 96, 1)
}

/// `sub_82491108`: bus `index` of `manager` (8 is the default bus behind [`BUS_ROOT`]), creating
/// it first when `create` is set and it has not been.
pub fn bus_for<D: DeviceHost + ?Sized>(g: &mut Guest, host: &mut D, manager: u32, index: u32, create: u8) -> Result<u32> {
    if index as i32 == 8 {
        let root = g.u32(BUS_ROOT)?;
        let table = g.u32(root.wrapping_add(44))?;
        return g.u32(table);
    }
    if create != 0 {
        // sub_824916E8's entry: nothing for bus 8 or a bus already made; otherwise mark it made.
        let flag = index.wrapping_add(manager).wrapping_add(1148);
        if g.u8(flag)? == 0 {
            g.set_u8(flag, 1)?;
            host.create_bus(g, manager, index)?;
        }
    }
    let cell = g.u32(manager.wrapping_add(index.wrapping_add(271).wrapping_mul(4)))?;
    g.u32(cell)
}

/// Reserve `bytes` on the command ring of `system`: `{+48 ring, +204 write offset}`.
fn enqueue(g: &mut Guest, system: u32, bytes: u32) -> Result<u32> {
    let offset = g.u32(system.wrapping_add(204))?;
    let ring = g.u32(system.wrapping_add(48))?;
    g.set_u32(system.wrapping_add(204), offset.wrapping_add(bytes))?;
    Ok(ring.wrapping_add(offset))
}

/// A module `vtable+4` call: `(module, id, block)`.
pub fn configure(g: &mut Guest, module: u32, id: u32, block: u32) -> Result<()> {
    let vtable = g.u32(module)?;
    let function = g.u32(vtable.wrapping_add(4))?;
    match function {
        CONFIGURE_NONE => Ok(()),
        CONFIGURE_SEND => send_configure(g, module, id, block),
        CONFIGURE_FADER => fader_configure(g, module, block),
        CONFIGURE_SNDPLAYER => sndplayer_configure(g, module, id, block),
        _ => Err(Error::new(function, "module configure entry not ported")),
    }
}

/// The open's property post: a 16-byte [`COMMAND_STAMP`] record `{module, id, value}`, then the
/// module's `+4` callback, which the builder leaves 0 and this does not port.
fn post_property(g: &mut Guest, module: u32, id: u32, value: f64) -> Result<()> {
    let system = g.u32(module.wrapping_add(8))?;
    let at = enqueue(g, system, 16)?;
    store_single(g, at.wrapping_add(12), value)?;
    g.set_u32(at.wrapping_add(4), module)?;
    g.set_u32(at.wrapping_add(8), id)?;
    g.set_u32(at, COMMAND_STAMP)?;
    let callback = g.u32(module.wrapping_add(4))?;
    if callback != 0 {
        return Err(Error::new(callback, "a module property callback is not ported"));
    }
    Ok(())
}

/// Bytes of a NUL-terminated guest string, not counting the NUL.
fn strlen(g: &Guest, at: u32) -> Result<u32> {
    let mut end = at;
    while g.u8(end)? != 0 {
        end = end.wrapping_add(1);
    }
    Ok(end.wrapping_sub(at))
}

/// `sub_82B31370`: `Send`. Ids 0 and 2 enqueue the block's first 8 bytes behind
/// [`COMMAND_SEND_BUS`] and [`COMMAND_SEND_2`]; id 1 enqueues the string at `[block+4]` behind
/// [`COMMAND_SEND_NAME`]; any other id answers `{TAG_POINTER, [module+72]}` into the block.
pub fn send_configure(g: &mut Guest, module: u32, id: u32, block: u32) -> Result<()> {
    let system = g.u32(module.wrapping_add(8))?;
    match id {
        2 | 0 => {
            let at = enqueue(g, system, 16)?;
            if id == 2 {
                g.set_u32(at, COMMAND_SEND_2)?;
                g.set_u32(at.wrapping_add(4), module)?;
            } else {
                g.set_u32(at.wrapping_add(4), module)?;
                g.set_u32(at, COMMAND_SEND_BUS)?;
            }
            let value = g.u64(block)?;
            g.set_u64(at.wrapping_add(8), value)
        }
        1 => {
            let name = g.u32(block.wrapping_add(4))?;
            let length = strlen(g, name)?;
            let size = length.wrapping_add(16) & 0xFFFF_FFFC; // addi r6,r10,16 ; rlwinm r7,r6,0,0,29
            let at = enqueue(g, system, size)?;
            g.set_u32(at, COMMAND_SEND_NAME)?;
            g.set_u32(at.wrapping_add(4), module)?;
            g.set_u32(at.wrapping_add(8), size)?;
            for k in 0..=length {
                let byte = g.u8(name.wrapping_add(k))?;
                g.set_u8(at.wrapping_add(12).wrapping_add(k), byte)?;
            }
            Ok(())
        }
        _ => {
            let bus = g.u32(module.wrapping_add(72))?;
            g.set_u32(block, TAG_POINTER)?;
            g.set_u32(block.wrapping_add(4), bus)
        }
    }
}

/// `sub_82B23798`: `GainFader`, which ignores the id: a 32-byte [`COMMAND_FADER`] record of the
/// block's double at `+0`, singles at `+12` and `+20`, and `+28` rounded half away from zero.
pub fn fader_configure(g: &mut Guest, module: u32, block: u32) -> Result<()> {
    let system = g.u32(module.wrapping_add(8))?;
    let zero = load_single(g, ZERO)?;
    let at = enqueue(g, system, 32)?;
    g.set_u32(at, COMMAND_FADER)?;
    g.set_u32(at.wrapping_add(4), module)?;
    let target = g.u64(block)?;
    g.set_u64(at.wrapping_add(8), target)?;
    let a = load_single(g, block.wrapping_add(12))?;
    store_single(g, at.wrapping_add(16), a)?;
    let b = load_single(g, block.wrapping_add(20))?;
    store_single(g, at.wrapping_add(20), b)?;
    let value = load_single(g, block.wrapping_add(28))?;
    let half = load_single(g, HALF)?;
    let rounded = if value < zero { sub_single(value, half) } else { add_single(value, half) };
    g.set_u32(at.wrapping_add(24), fctiwz_low_word(rounded))
}

/// `sub_82B328E8`: `SndPlayer1`. Id 0 does nothing. Id 5 plays: it advances the play counter at
/// `[module+440]` (wrapping above [`PLAY_LIMIT`] to 1) and the count at `[module+444]`, and
/// enqueues a [`COMMAND_PLAY`] record of the parameter 5 block — three 8-byte slots, the words at
/// `+36`, `+44` and `+52`, the name string at `[block+28]` from `+56`, and `+60` rounded to a byte —
/// then writes the counter back into the block at `+64`. Ids 1 to 4 are not ported.
pub fn sndplayer_configure(g: &mut Guest, module: u32, id: u32, block: u32) -> Result<()> {
    match id {
        0 => return Ok(()),
        5 => {}
        _ => return Err(Error::new(CONFIGURE_SNDPLAYER, "SndPlayer1 configure: only ids 0 and 5 are ported")),
    }
    let system = g.u32(module.wrapping_add(8))?;
    let counter = g.u32(module.wrapping_add(440))?;
    let one = load_single(g, ONE)?;
    let value = load_single(g, counter)?;
    store_single(g, counter, add_single(value, one))?;
    let plays = g.u32(module.wrapping_add(444))?;
    let limit = load_single(g, PLAY_LIMIT)?;
    let count = g.u32(plays)?;
    g.set_u32(plays, count.wrapping_add(1))?;
    let counter = g.u32(module.wrapping_add(440))?;
    if load_single(g, counter)? > limit {
        store_single(g, counter, one)?;
    }
    let name = g.u32(block.wrapping_add(28))?;
    let length = if name == 0 { 1 } else { strlen(g, name)?.wrapping_add(1) }; // r11
    let size = length.wrapping_add(59) & 0xFFFF_FFFC;
    let at = enqueue(g, system, size)?;
    g.set_u32(at, COMMAND_PLAY)?;
    g.set_u32(at.wrapping_add(4), module)?;
    let counter = g.u32(module.wrapping_add(440))?;
    let value = load_single(g, counter)?;
    g.set_u16(at.wrapping_add(44), size as u16)?;
    store_single(g, at.wrapping_add(48), value)?;
    if length == 1 {
        g.set_u8(at.wrapping_add(56), 0)?;
    } else {
        for k in 0..length {
            let byte = g.u8(name.wrapping_add(k))?;
            g.set_u8(at.wrapping_add(56).wrapping_add(k), byte)?;
        }
    }
    for k in 0..3 {
        let slot = g.u64(block.wrapping_add(8 * k))?;
        g.set_u64(at.wrapping_add(8 + 8 * k), slot)?;
    }
    let zero = load_single(g, ZERO)?;
    for (from, to) in [(36, 32), (44, 36), (52, 40)] {
        let word = g.u32(block.wrapping_add(from))?;
        g.set_u32(at.wrapping_add(to), word)?;
    }
    let counter = g.u32(module.wrapping_add(440))?;
    let gain = load_single(g, block.wrapping_add(60))?;
    let half = load_single(g, HALF)?;
    let value = load_single(g, counter)?;
    g.set_u32(block.wrapping_add(64), TAG_SINGLE)?;
    store_single(g, block.wrapping_add(68), value)?;
    let rounded = if gain < zero { sub_single(gain, half) } else { add_single(gain, half) };
    g.set_u8(at.wrapping_add(46), fctiwz_low_word(rounded) as u8)
}

/// Write module descriptor `i` of the frame's list at `+208`: class, argument, channel byte.
fn descriptor(g: &mut Guest, frame: u32, i: u32, arg: u32, class: u32, channels: u8) -> Result<()> {
    let at = frame.wrapping_add(208).wrapping_add(12 * i);
    g.set_u32(at.wrapping_add(4), class)?;
    g.set_u32(at, arg)?;
    g.set_u8(at.wrapping_add(8), channels)
}

/// `sub_824A3140`: open a voice on `sample` (an EA Audio Core stream) with the six descriptor words
/// at `descriptor`, `byte` (the descriptor's byte 2), `bank_72`, `arg8` and the `{count, records*}`
/// block at `records` of 12-byte `{u8 id, s32, s32 value}` records. `device` is the device object,
/// `sp` the caller's stack pointer. Returns the voice, or 0 when its allocation fails.
#[allow(clippy::too_many_arguments)]
pub fn open_voice_graph<H: Heap + ?Sized, T: Trig + ?Sized, D: DeviceHost + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    trig: &mut T,
    host: &mut D,
    device: u32,
    sample: u32,
    byte: u32,
    descriptor_words: u32,
    bank_72: u32,
    arg8: u32,
    records: u32,
    sp: u32,
) -> Result<u32> {
    let frame = sp.wrapping_sub(FRAME_BYTES);
    g.set_u32(frame.wrapping_add(644), device)?; // stw r3,644(r1)
    g.set_u32(frame.wrapping_add(668), descriptor_words)?; // stw r6,668(r1)
    let system = g.u32(SYSTEM)?; // r18
    if g.u8(REGISTERED)? == 0 {
        classes::register_voice_classes(g, heap)?; // bl sub_824A2FD8
    }
    let header = frame.wrapping_add(192);
    unpack_stream_header(g, sample, header, frame)?; // bl sub_82B31D90
    let channels = g.u8(header)?; // r20
    let (i100, i104, i112) = match channels {
        1 => (0, 0, 0),
        2 => (0, 1, 0),
        4 => (0, 1, 3),
        _ => (1, 2, 4),
    };
    g.set_u32(frame.wrapping_add(100), i100)?;
    g.set_u32(frame.wrapping_add(104), i104)?;
    g.set_u32(frame.wrapping_add(112), i112)?;

    let voice = heap.alloc(g, VOICE_BYTES, 16)?;
    if voice == 0 {
        return Ok(0);
    }
    init_voice_object(g, voice)?;
    init_voice_object(g, voice)?; // called twice in the original
    g.set_u32(voice + 76, 0)?;
    g.set_u32(voice + 72, 0)?;
    g.set_u32(voice + 84, 0)?;
    g.set_u32(voice + 88, 0)?;
    let zero = load_single(g, ZERO)?; // f31
    g.set_u32(voice + 92, 0)?;
    store_single(g, voice + 80, zero)?;
    g.set_u32(voice + 96, 1)?;
    g.set_u32(frame.wrapping_add(88), RODATA)?;

    // The routing records: only ids 9 and above are read here.
    let mut effect_send = false; // r26's low byte
    let mut index = 0u32; // r30
    let mut offset = 0u32; // r29
    if g.u32(records)? != 0 {
        let level_scale = load_single(g, LEVEL_SCALE)?; // f30
        loop {
            let count = g.u32(records)?; // r9
            let list = g.u32(records.wrapping_add(4))?; // r8
            if index >= count || list == 0 || (g.u8(list.wrapping_add(offset))? as i32) < 9 {
                index += 1;
                offset += 12;
            } else {
                let value = g.u32(list.wrapping_add(offset).wrapping_add(8))?; // r4
                index += 1;
                offset += 12;
                let v = value as i32;
                let mut buses = false;
                if v > 2048 {
                    if v == 4096 || v == 8192 || v == 16384 {
                        let first = if index < count && list != 0 { g.u32(list.wrapping_add(offset).wrapping_add(8))? } else { 0 };
                        let (next_index, next_offset) = (index + 1, offset + 12);
                        let second = if next_index < count && list != 0 { g.u32(list.wrapping_add(next_offset).wrapping_add(8))? } else { 0 };
                        index = next_index + 1;
                        offset = next_offset + 12;
                        if first == 1 {
                            g.set_u32(voice + 72, value)?;
                            g.set_u32(voice + 76, 1)?;
                            let level = mul_single(frsp(fcfid(second as i32 as i64)), level_scale);
                            store_single(g, voice + 80, level)?;
                            let which = u32::from(value == 16384);
                            let manager = g.u32(BUS_MANAGER)?;
                            let cell = g.u32(manager.wrapping_add((which + 29) * 4))?;
                            effect_send = true;
                            let bus = g.u32(cell)?;
                            g.set_u32(voice + 88, bus)?;
                        }
                    } else {
                        buses = true;
                    }
                } else if v == 2048 || v == 512 {
                    let root = g.u32(ROUTING_ROOT)?;
                    let routing = g.u32(root.wrapping_add(688))?;
                    let cell = g.u32(routing.wrapping_add(if v == 2048 { 40 } else { 48 }))?;
                    let bus = g.u32(cell)?;
                    g.set_u32(voice + 96, 0)?;
                    g.set_u32(voice + 72, value)?;
                    g.set_u32(voice + 92, bus)?;
                } else if v != 256 && v != 1024 {
                    buses = true;
                }
                if buses {
                    if (0..8).contains(&v) {
                        let manager = g.u32(BUS_MANAGER)?;
                        let bus = bus_for(g, host, manager, value, 0)?;
                        g.set_u32(voice + 92, bus)?;
                    } else if (10..18).contains(&v) {
                        let manager = g.u32(BUS_MANAGER)?;
                        let bus = bus_for(g, host, manager, value - 10, 1)?;
                        g.set_u32(voice + 92, bus)?;
                    }
                }
            }
            if index >= g.u32(records)? {
                break;
            }
        }
    }

    // The module list.
    let rate = g.u32(header.wrapping_add(4))?;
    let frames = g.u32(header.wrapping_add(8))?;
    g.set_u8(voice + 68, channels)?;
    let duration = fcfid(frames as i32 as i64) / fcfid(rate as i32 as i64); // fdiv: double precision
    g.set_u64(voice + 56, duration.to_bits())?;
    let ch = g.u8(voice + 68)?;
    let send_class = g.u32(SEND_SLOT)?; // r5
    descriptor(g, frame, 0, 0, g.u32(SNDPLAYER_SLOT)?, ch)?;
    descriptor(g, frame, 1, 0, g.u32(RECHANNEL_SLOT)?, ch)?;
    descriptor(g, frame, 2, 0, g.u32(RESAMPLE_SLOT)?, ch)?;
    descriptor(g, frame, 3, 0, g.u32(HIGHPASS_SLOT)?, ch)?;
    descriptor(g, frame, 4, 0, g.u32(LOWPASS_SLOT)?, ch)?;
    let manager = g.u32(BUS_MANAGER)?;
    let first_send = g.u32(g.u32(manager.wrapping_add(52))?)? as i32 != 0;
    let (first_send_index, gain_index) = if first_send { (5u32, 6u32) } else { (0, 5) }; // r24, r28
    if first_send {
        descriptor(g, frame, 5, 0, send_class, ch)?;
    }
    descriptor(g, frame, gain_index, 0, g.u32(GAIN_SLOT)?, ch)?;
    let mut pan_index = gain_index + 1; // r29
    let mut effect_index = u32::MAX; // r26 = -1
    if effect_send {
        descriptor(g, frame, pan_index, 0, send_class, ch)?;
        effect_index = pan_index;
        pan_index += 1;
    }
    // Pan2D1's argument: its class's leading rows, then front, side and law overridden.
    let pan_class = g.u32(PAN_SLOT)?;
    let block = frame.wrapping_add(160);
    let words = g.u8(pan_class.wrapping_add(41))? as u32;
    let table = g.u32(pan_class.wrapping_add(20))?;
    for k in 0..words {
        let slot = g.u64(table.wrapping_add(8).wrapping_add(40 * k))?;
        g.set_u64(block.wrapping_add(8 * k), slot)?;
    }
    let output_index = pan_index + 1; // r25
    store_single(g, frame.wrapping_add(180), zero)?;
    let front = load_single(g, PAN_FRONT)?;
    store_single(g, frame.wrapping_add(164), front)?;
    descriptor(g, frame, pan_index, block, pan_class, 6)?;
    let side = load_single(g, PAN_SIDE)?;
    store_single(g, frame.wrapping_add(172), side)?;
    descriptor(g, frame, output_index, 0, send_class, 6)?;
    for k in [0, 8, 16] {
        g.set_u32(block.wrapping_add(k), TAG_SINGLE)?;
    }
    let player = modules::build_graph(g, heap, trig, system, 0, output_index + 1, frame.wrapping_add(208))?;
    g.set_u32(voice + 4, player)?;
    if player == 0 {
        return Err(Error::new(VOICE_RELEASE, "the graph was not built, and the voice's release is not ported"));
    }

    g.set_u32(player + 20, PLAYER_NAME)?;
    let percent = load_single(g, PERCENT)?;
    let level = if (arg8 as i32) < 0 { 0 } else { arg8 }; // rlwinm r11,r19,1,31,31 ; addi -1 ; and
    let modules_at = player + 80; // r29
    for (to, from) in [(8, 80), (12, 88), (16, 92), (20, 96)] {
        let module = g.u32(player + from)?;
        g.set_u32(voice + to, module)?;
    }
    let scaled = mul_single(frsp(fcfid(byte as i32 as i64)), percent);
    let gain = g.u32(modules_at + 4 * gain_index)?;
    g.set_u32(voice + 28, gain)?;
    let pan = g.u32(modules_at + 4 * pan_index)?;
    g.set_u32(voice + 32, pan)?;
    let player_system = g.u32(player + 16)?;
    let at = enqueue(g, player_system, 12)?;
    store_single(g, at + 8, scaled)?;
    g.set_u32(at, COMMAND_PLAYER_FLOAT)?;
    g.set_u32(at + 4, player)?;

    // Play: SndPlayer1's parameter 5.
    let play = frame.wrapping_add(384);
    classes::class_defaults(g, g.u32(SNDPLAYER_SLOT)?, 5, play)?;
    g.set_u32(play + 28, bank_72)?;
    g.set_u32(play + 36, sample)?;
    let one = load_single(g, ONE)?;
    store_single(g, play + 60, one)?;
    g.set_u64(play + 8, fcfid(level as i32 as i64).to_bits())?;
    g.set_u32(play + 56, TAG_SINGLE)?;
    g.set_u32(play + 24, TAG_STRING)?;
    g.set_u32(play + 32, TAG_POINTER)?;
    g.set_u32(play + 52, AEMS)?;
    g.set_u32(play + 48, TAG_INTEGER)?;
    configure(g, g.u32(voice + 8)?, 5, play)?;
    let effect_bus = g.u32(voice + 88)?;
    let counter = load_single(g, play + 68)?;
    store_single(g, voice + 52, counter)?;

    if effect_bus != 0 {
        let module = g.u32(modules_at.wrapping_add(effect_index.wrapping_mul(4)))?;
        g.set_u32(voice + 84, module)?;
        let route = frame.wrapping_add(128);
        classes::class_defaults(g, g.u32(SEND_SLOT)?, 0, route)?;
        let bus = g.u32(voice + 88)?;
        g.set_u32(route, TAG_POINTER)?;
        g.set_u32(route + 4, bus)?;
        configure(g, g.u32(voice + 84)?, 0, route)?;
        post_property(g, g.u32(voice + 84)?, 0, zero)?;
    } else {
        g.set_u32(voice + 84, 0)?;
    }

    let manager = g.u32(BUS_MANAGER)?;
    if g.u32(g.u32(manager.wrapping_add(52))?)? as i32 != 0 {
        let module = g.u32(modules_at + 4 * first_send_index)?;
        g.set_u32(voice + 24, module)?;
        let route = frame.wrapping_add(136);
        classes::class_defaults(g, g.u32(SEND_SLOT)?, 0, route)?;
        let manager = g.u32(BUS_MANAGER)?;
        let bus = g.u32(g.u32(manager.wrapping_add(52))?)?;
        g.set_u32(route, TAG_POINTER)?;
        g.set_u32(route + 4, bus)?;
        configure(g, g.u32(voice + 24)?, 0, route)?;
        post_property(g, g.u32(voice + 24)?, 0, zero)?;
    } else {
        g.set_u32(voice + 24, 0)?;
    }

    // The panner's angles, from the descriptor words.
    let angle = |g: &Guest, word_index: u32| -> Result<f64> {
        let words = g.u32(frame.wrapping_add(668))?;
        let scale = load_single(g, g.u32(frame.wrapping_add(88))?.wrapping_add(744))?;
        let word = g.u32(words.wrapping_add(4 * word_index))?;
        Ok(mul_single(frsp(fcfid(word as i32 as i64)), scale))
    };
    if g.u8(voice + 68)? > 1 {
        post_property(g, g.u32(voice + 32)?, 1, zero)?;
        let value = angle(g, g.u32(frame.wrapping_add(104))?)?;
        post_property(g, g.u32(voice + 32)?, 7, value)?;
        if g.u8(voice + 68)? > 2 {
            let value = angle(g, g.u32(frame.wrapping_add(112))?)?;
            post_property(g, g.u32(voice + 32)?, 8, value)?;
        }
    } else {
        let mode = g.u32(voice + 96)?;
        if mode as i32 == 1 {
            let value = angle(g, g.u32(frame.wrapping_add(100))?)?;
            post_property(g, g.u32(voice + 32)?, 0, value)?;
        } else {
            let value = angle(g, g.u32(frame.wrapping_add(100))?)?;
            let device = g.u32(frame.wrapping_add(644))?;
            store_single(g, device.wrapping_add(mode.wrapping_add(1).wrapping_mul(4)), value)?;
        }
        g.set_u8(voice + 69, 0)?;
    }

    // The output send: the voice's bus, or the default.
    let output_bus = g.u32(voice + 92)?;
    let send_class = g.u32(SEND_SLOT)?;
    let route = if output_bus == 0 { frame.wrapping_add(144) } else { frame.wrapping_add(120) };
    classes::class_defaults(g, send_class, 0, route)?;
    let bus = if output_bus == 0 { g.u32(DEFAULT_BUS)? } else { g.u32(voice + 92)? };
    g.set_u32(route, TAG_POINTER)?;
    g.set_u32(route + 4, bus)?;
    let output = g.u32(modules_at + 4 * output_index)?;
    configure(g, output, 0, route)?;
    Ok(voice)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEM: u32 = 0x5000_0000;
    const SYS: u32 = MEM + 0x100;
    const RING: u32 = MEM + 0x400;
    const MODULE: u32 = MEM + 0x800;
    const VTABLE: u32 = MEM + 0x900;
    const BLOCK: u32 = MEM + 0xA00;
    const MANAGER: u32 = MEM + 0x1000;

    fn guest() -> Guest {
        let mut g = Guest::single(MEM, 0x2000);
        for (at, v) in [(ONE, 1.0f32), (ZERO, 0.0), (HALF, 0.5), (modules::MINUS_ONE, -1.0), (PLAY_LIMIT, 4194304.0)] {
            g.put(at, v.to_bits().to_be_bytes().to_vec());
        }
        g.put(ZERO_DOUBLE, 0u64.to_be_bytes().to_vec());
        g.set_u32(SYS + 48, RING).unwrap();
        g.set_u32(MODULE, VTABLE).unwrap();
        g.set_u32(MODULE + 8, SYS).unwrap();
        g
    }

    struct Recorder(Vec<u32>);
    impl DeviceHost for Recorder {
        fn create_bus(&mut self, g: &mut Guest, manager: u32, index: u32) -> Result<()> {
            self.0.push(index);
            g.set_u32(manager + (index + 271) * 4, MEM + 0x1F00)
        }
    }

    #[test]
    fn a_voice_object_starts_with_unit_gains_and_mode_one() {
        let mut g = guest();
        g.fill(MEM + 0x200, 0xEE, 104).unwrap();
        init_voice_object(&mut g, MEM + 0x200).unwrap();
        let v = MEM + 0x200;
        assert_eq!(g.u32(v).unwrap(), VOICE_VTABLE);
        assert_eq!([g.f32(v + 36).unwrap(), g.f32(v + 44).unwrap(), g.f32(v + 52).unwrap()], [1.0, 1.0, -1.0]);
        assert_eq!((g.u8(v + 68).unwrap(), g.u8(v + 69).unwrap(), g.u32(v + 96).unwrap()), (0, 1, 1));
        assert_eq!(g.u32(v + 100).unwrap(), 0xEEEE_EEEE, "104 bytes and no more");
    }

    #[test]
    fn a_bus_is_created_once_and_bus_8_is_the_default() {
        let mut g = guest();
        g.put(BUS_ROOT, (MEM + 0x1E00).to_be_bytes().to_vec());
        g.set_u32(MEM + 0x1E00 + 44, MEM + 0x1E40).unwrap();
        g.set_u32(MEM + 0x1E40, 0xDEF0_0008).unwrap();
        g.set_u32(MEM + 0x1F00, 0xB0B0_0002).unwrap();
        let mut host = Recorder(Vec::new());
        assert_eq!(bus_for(&mut g, &mut host, MANAGER, 8, 1).unwrap(), 0xDEF0_0008);
        assert_eq!(bus_for(&mut g, &mut host, MANAGER, 2, 1).unwrap(), 0xB0B0_0002);
        assert_eq!(bus_for(&mut g, &mut host, MANAGER, 2, 1).unwrap(), 0xB0B0_0002);
        assert_eq!(host.0, [2], "created on first use only");
        assert_eq!(g.u8(MANAGER + 1148 + 2).unwrap(), 1);
    }

    #[test]
    fn send_routes_by_id() {
        let mut g = guest();
        g.set_u32(VTABLE + 4, CONFIGURE_SEND).unwrap();
        g.set_u64(BLOCK, 0x7FF7_FFF4_1234_5678).unwrap();
        configure(&mut g, MODULE, 0, BLOCK).unwrap();
        assert_eq!([g.u32(RING).unwrap(), g.u32(RING + 4).unwrap()], [COMMAND_SEND_BUS, MODULE]);
        assert_eq!(g.u64(RING + 8).unwrap(), 0x7FF7_FFF4_1234_5678);
        // A name: 5 bytes + 16, rounded down to 4.
        g.set_span(MEM + 0xB00, b"verb\0").unwrap();
        g.set_u32(BLOCK + 4, MEM + 0xB00).unwrap();
        send_configure(&mut g, MODULE, 1, BLOCK).unwrap();
        assert_eq!([g.u32(RING + 16).unwrap(), g.u32(RING + 24).unwrap()], [COMMAND_SEND_NAME, 20]);
        assert_eq!(g.span(RING + 28, 5).unwrap(), b"verb\0");
        assert_eq!(g.u32(SYS + 204).unwrap(), 36);
        g.set_u32(MODULE + 72, 0xAB).unwrap();
        send_configure(&mut g, MODULE, 3, BLOCK).unwrap();
        assert_eq!([g.u32(BLOCK).unwrap(), g.u32(BLOCK + 4).unwrap()], [TAG_POINTER, 0xAB]);
    }

    #[test]
    fn a_play_advances_the_counter_and_enqueues_the_block() {
        let mut g = guest();
        g.set_u32(VTABLE + 4, CONFIGURE_SNDPLAYER).unwrap();
        let (counter, plays) = (MEM + 0xC00, MEM + 0xC08);
        g.set_u32(MODULE + 440, counter).unwrap();
        g.set_u32(MODULE + 444, plays).unwrap();
        g.set_u32(counter, 4194304.0f32.to_bits()).unwrap();
        for k in 0..3u32 {
            g.set_u64(BLOCK + 8 * k, 0x1111_0000 + k as u64).unwrap();
        }
        g.set_span(MEM + 0xB00, b"step\0").unwrap();
        g.set_u32(BLOCK + 28, MEM + 0xB00).unwrap();
        g.set_u32(BLOCK + 36, 0x5000_1234).unwrap();
        g.set_u32(BLOCK + 52, AEMS).unwrap();
        g.set_u32(BLOCK + 60, 1.0f32.to_bits()).unwrap();
        configure(&mut g, MODULE, 5, BLOCK).unwrap();
        assert_eq!(g.f32(counter).unwrap(), 1.0, "wrapped above the limit");
        assert_eq!(g.u32(plays).unwrap(), 1);
        let size = (5 + 59) & !3;
        assert_eq!(g.u32(SYS + 204).unwrap(), size);
        assert_eq!([g.u32(RING).unwrap(), g.u32(RING + 4).unwrap(), g.u16(RING + 44).unwrap() as u32], [COMMAND_PLAY, MODULE, size]);
        assert_eq!([g.u64(RING + 8).unwrap(), g.u64(RING + 24).unwrap()], [0x1111_0000, 0x1111_0002]);
        assert_eq!([g.u32(RING + 32).unwrap(), g.u32(RING + 40).unwrap(), g.u8(RING + 46).unwrap() as u32], [0x5000_1234, AEMS, 1]);
        assert_eq!(g.span(RING + 56, 5).unwrap(), b"step\0");
        assert_eq!((g.u32(BLOCK + 64).unwrap(), g.f32(BLOCK + 68).unwrap()), (TAG_SINGLE, 1.0));
        assert!(sndplayer_configure(&mut g, MODULE, 2, BLOCK).is_err());
        sndplayer_configure(&mut g, MODULE, 0, BLOCK).unwrap();
    }

    #[test]
    fn a_fader_rounds_its_last_value() {
        let mut g = guest();
        g.set_u32(VTABLE + 4, CONFIGURE_FADER).unwrap();
        g.set_u64(BLOCK, 2.5f64.to_bits()).unwrap();
        g.set_u32(BLOCK + 12, 0.25f32.to_bits()).unwrap();
        g.set_u32(BLOCK + 20, 0.75f32.to_bits()).unwrap();
        g.set_u32(BLOCK + 28, (-2.5f32).to_bits()).unwrap();
        configure(&mut g, MODULE, 9, BLOCK).unwrap();
        assert_eq!([g.u32(RING).unwrap(), g.u32(SYS + 204).unwrap()], [COMMAND_FADER, 32]);
        assert_eq!(g.u64(RING + 8).unwrap(), 2.5f64.to_bits());
        assert_eq!([g.f32(RING + 16).unwrap(), g.f32(RING + 20).unwrap()], [0.25, 0.75]);
        assert_eq!(g.u32(RING + 24).unwrap() as i32, -3, "half away from zero");
    }

    #[test]
    fn an_unknown_configure_entry_is_an_error() {
        let mut g = guest();
        g.set_u32(VTABLE + 4, 0x8200_0000).unwrap();
        assert!(configure(&mut g, MODULE, 0, BLOCK).is_err());
        g.set_u32(VTABLE + 4, CONFIGURE_NONE).unwrap();
        configure(&mut g, MODULE, 0, BLOCK).unwrap();
    }
}
