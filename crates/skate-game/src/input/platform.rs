//! Device transport. Raw signed axes and trigger bytes reach the TU3 converter
//! without Bevy/gilrs deadzones or normalized-axis reconstruction.
//! Windows reads XInput. Linux reads evdev gamepads and emits the same Xbox state.
use skate_core::input::xbox::XboxState;

pub(crate) struct DevicePacket {
    pub number: u32,
    pub state: XboxState,
    pub subtype: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceError {
    Disconnected,
    #[cfg(windows)]
    State(u32),
    Capabilities(u32),
    #[cfg(not(any(windows, target_os = "linux")))]
    UnsupportedPlatform,
}

/// Device identity is metadata; raw input is still sampled every host frame.
/// Refresh periodically as well as after errors, so hot swaps cannot leave a
/// subtype cached indefinitely even if Windows never exposes a disconnect.
#[derive(Default)]
pub(crate) struct CapabilityCache {
    value: Option<(u8, std::time::Instant)>,
}
impl CapabilityCache {
    pub(crate) fn invalidate(&mut self) {
        self.value = None;
    }
    fn get(
        &mut self,
        now: std::time::Instant,
        read: impl FnOnce() -> Result<u8, DeviceError>,
    ) -> Result<u8, DeviceError> {
        if let Some((subtype, expires)) = self.value {
            if now < expires {
                return Ok(subtype);
            }
        }
        self.value = None;
        let subtype = read()?;
        self.value = Some((subtype, now + std::time::Duration::from_secs(1)));
        Ok(subtype)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::mem::MaybeUninit;

    // ABI from the installed Windows SDK Xinput.h. No OS-owned pointers are
    // retained and only successful calls permit reading output storage.
    #[repr(C)]
    struct Gamepad {
        buttons: u16,
        left_trigger: u8,
        right_trigger: u8,
        left_x: i16,
        left_y: i16,
        right_x: i16,
        right_y: i16,
    }
    #[repr(C)]
    struct State {
        number: u32,
        gamepad: Gamepad,
    }
    #[repr(C)]
    struct Vibration {
        left: u16,
        right: u16,
    }
    #[repr(C)]
    struct Capabilities {
        device_type: u8,
        subtype: u8,
        flags: u16,
        gamepad: Gamepad,
        vibration: Vibration,
    }
    const _: () = assert!(size_of::<Gamepad>() == 12);
    const _: () = assert!(size_of::<State>() == 16);
    const _: () = assert!(size_of::<Capabilities>() == 20);

    #[link(name = "xinput")]
    unsafe extern "system" {
        fn XInputGetState(index: u32, state: *mut State) -> u32;
        fn XInputGetCapabilities(index: u32, flags: u32, capabilities: *mut Capabilities) -> u32;
    }

    pub(super) fn poll(
        index: u32,
        cache: &mut CapabilityCache,
    ) -> Result<DevicePacket, DeviceError> {
        let mut state = MaybeUninit::<State>::uninit();
        // SAFETY: properly aligned writable storage with the SDK's exact C ABI.
        let result = unsafe { XInputGetState(index, state.as_mut_ptr()) };
        if result != 0 {
            cache.invalidate();
        }
        if result == 1167 {
            return Err(DeviceError::Disconnected);
        }
        if result != 0 {
            return Err(DeviceError::State(result));
        }
        let subtype = cache.get(std::time::Instant::now(), || {
            let mut capabilities = MaybeUninit::<Capabilities>::uninit();
            // SAFETY: writable storage with the SDK ABI; read only on success.
            let result = unsafe { XInputGetCapabilities(index, 1, capabilities.as_mut_ptr()) };
            if result != 0 {
                return Err(DeviceError::Capabilities(result));
            }
            Ok(unsafe { capabilities.assume_init() }.subtype)
        })?;
        // SAFETY: successful XInputGetState initialized the complete structure.
        let state = unsafe { state.assume_init() };
        Ok(DevicePacket {
            number: state.number,
            state: XboxState {
                buttons: state.gamepad.buttons,
                triggers: [state.gamepad.left_trigger, state.gamepad.right_trigger],
                left: [state.gamepad.left_x, state.gamepad.left_y],
                right: [state.gamepad.right_x, state.gamepad.right_y],
            },
            subtype,
        })
    }
}

/// One kernel absolute axis, already carrying the device's reported range.
#[derive(Clone, Copy)]
pub(crate) struct AbsSample {
    pub code: u16,
    pub value: i32,
    pub min: i32,
    pub max: i32,
}

/// Linux gamepad specification plus the `xpad` trigger axes, as XInput state.
/// Stick Y is negated: the kernel reports negative as up, XInput reports positive as up.
pub(crate) fn xbox_from_linux(keys: &[u16], axes: &[AbsSample]) -> XboxState {
    const BTN_SOUTH: u16 = 0x130;
    const BTN_EAST: u16 = 0x131;
    const BTN_NORTH: u16 = 0x133;
    const BTN_WEST: u16 = 0x134;
    const BTN_TL: u16 = 0x136;
    const BTN_TR: u16 = 0x137;
    const BTN_TL2: u16 = 0x138;
    const BTN_TR2: u16 = 0x139;
    const BTN_SELECT: u16 = 0x13a;
    const BTN_START: u16 = 0x13b;
    const BTN_THUMBL: u16 = 0x13d;
    const BTN_THUMBR: u16 = 0x13e;
    const BTN_DPAD_UP: u16 = 0x220;
    const BTN_DPAD_DOWN: u16 = 0x221;
    const BTN_DPAD_LEFT: u16 = 0x222;
    const BTN_DPAD_RIGHT: u16 = 0x223;
    const ABS_X: u16 = 0x00;
    const ABS_Y: u16 = 0x01;
    const ABS_Z: u16 = 0x02;
    const ABS_RX: u16 = 0x03;
    const ABS_RY: u16 = 0x04;
    const ABS_RZ: u16 = 0x05;
    const ABS_HAT0X: u16 = 0x10;
    const ABS_HAT0Y: u16 = 0x11;
    const ABS_HAT2X: u16 = 0x14;
    const ABS_HAT2Y: u16 = 0x15;

    let mut buttons = 0u16;
    for code in keys {
        buttons |= match *code {
            BTN_SOUTH => 0x1000,
            BTN_EAST => 0x2000,
            BTN_WEST => 0x4000,
            BTN_NORTH => 0x8000,
            BTN_TL => 0x0100,
            BTN_TR => 0x0200,
            BTN_SELECT => 0x0020,
            BTN_START => 0x0010,
            BTN_THUMBL => 0x0040,
            BTN_THUMBR => 0x0080,
            BTN_DPAD_UP => 0x0001,
            BTN_DPAD_DOWN => 0x0002,
            BTN_DPAD_LEFT => 0x0004,
            BTN_DPAD_RIGHT => 0x0008,
            _ => 0,
        };
    }
    if let Some(hat) = axis(axes, ABS_HAT0X) {
        if hat.value < 0 {
            buttons |= 0x0004;
        } else if hat.value > 0 {
            buttons |= 0x0008;
        }
    }
    if let Some(hat) = axis(axes, ABS_HAT0Y) {
        if hat.value < 0 {
            buttons |= 0x0001;
        } else if hat.value > 0 {
            buttons |= 0x0002;
        }
    }
    XboxState {
        buttons,
        triggers: [
            trigger(keys, axes, ABS_HAT2Y, ABS_Z, BTN_TL2),
            trigger(keys, axes, ABS_HAT2X, ABS_RZ, BTN_TR2),
        ],
        left: [stick(axes, ABS_X, false), stick(axes, ABS_Y, true)],
        right: [stick(axes, ABS_RX, false), stick(axes, ABS_RY, true)],
    }
}

fn axis(axes: &[AbsSample], code: u16) -> Option<&AbsSample> {
    axes.iter().find(|sample| sample.code == code)
}

fn stick(axes: &[AbsSample], code: u16, negate_y: bool) -> i16 {
    let Some(sample) = axis(axes, code) else {
        return 0;
    };
    let scaled = scale_stick(sample.value, sample.min, sample.max);
    if negate_y {
        if scaled == i16::MIN { i16::MAX } else { -scaled }
    } else {
        scaled
    }
}

fn scale_stick(value: i32, min: i32, max: i32) -> i16 {
    if max <= min {
        return 0;
    }
    let span = i64::from(max) - i64::from(min);
    let shifted = (i64::from(value.clamp(min, max)) - i64::from(min)) * 65535 / span - 32768;
    shifted.clamp(-32768, 32767) as i16
}

fn trigger(keys: &[u16], axes: &[AbsSample], spec: u16, xpad: u16, digital: u16) -> u8 {
    if let Some(sample) = axis(axes, spec).or_else(|| axis(axes, xpad)) {
        return scale_trigger(sample.value, sample.min, sample.max);
    }
    if keys.contains(&digital) { 255 } else { 0 }
}

fn scale_trigger(value: i32, min: i32, max: i32) -> u8 {
    let min = min.max(0);
    if max <= min {
        return 0;
    }
    let value = value.clamp(min, max);
    ((i64::from(value - min) * 255) / i64::from(max - min)) as u8
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{AbsSample, CapabilityCache, DeviceError, DevicePacket, xbox_from_linux};
    use evdev::{Device, EventSummary, KeyCode};
    use skate_core::input::xbox::XboxState;
    use std::{
        io::ErrorKind,
        path::PathBuf,
        sync::Mutex,
        time::{Duration, Instant},
    };

    struct OpenPad {
        path: PathBuf,
        device: Device,
        keys: Vec<u16>,
        axes: Vec<AbsSample>,
        number: u32,
        state: XboxState,
    }

    impl OpenPad {
        fn open(path: PathBuf, device: Device) -> Result<Self, ()> {
            device.set_nonblocking(true).map_err(|_| ())?;
            let mut keys = Vec::new();
            if let Ok(state) = device.get_key_state() {
                for key in state.iter() {
                    keys.push(key.code());
                }
            }
            let mut axes = Vec::new();
            if let Ok(info) = device.get_absinfo() {
                for (code, abs) in info {
                    axes.push(AbsSample {
                        code: code.0,
                        value: abs.value(),
                        min: abs.minimum(),
                        max: abs.maximum(),
                    });
                }
            }
            let state = xbox_from_linux(&keys, &axes);
            Ok(Self { path, device, keys, axes, number: 0, state })
        }

        fn pump(&mut self) -> Result<(u32, XboxState), DeviceError> {
            match self.device.fetch_events() {
                Ok(events) => {
                    for event in events {
                        match event.destructure() {
                            EventSummary::Key(_, key, value) => set_key(&mut self.keys, key.code(), value),
                            EventSummary::AbsoluteAxis(_, axis, value) => set_axis(&mut self.axes, axis.0, value),
                            _ => {}
                        }
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => return Err(DeviceError::Disconnected),
            }
            let next = xbox_from_linux(&self.keys, &self.axes);
            if next.buttons != self.state.buttons
                || next.triggers != self.state.triggers
                || next.left != self.state.left
                || next.right != self.state.right
            {
                self.number = self.number.wrapping_add(1);
                self.state = next;
            }
            Ok((self.number, XboxState {
                buttons: self.state.buttons,
                triggers: self.state.triggers,
                left: self.state.left,
                right: self.state.right,
            }))
        }
    }

    fn set_key(keys: &mut Vec<u16>, code: u16, value: i32) {
        if value == 0 {
            keys.retain(|pressed| *pressed != code);
        } else if !keys.contains(&code) {
            keys.push(code);
        }
    }

    fn set_axis(axes: &mut [AbsSample], code: u16, value: i32) {
        if let Some(sample) = axes.iter_mut().find(|sample| sample.code == code) {
            sample.value = value;
        }
    }

    struct Hub {
        pads: Vec<OpenPad>,
        last_scan: Option<Instant>,
        dirty: bool,
        complained: bool,
    }

    impl Hub {
        const fn new() -> Self {
            Self { pads: Vec::new(), last_scan: None, dirty: true, complained: false }
        }

        fn sample(&mut self, index: usize) -> Result<(u32, XboxState), DeviceError> {
            let due = self.last_scan.is_none_or(|scan| scan.elapsed() >= Duration::from_secs(1));
            if self.dirty || (self.pads.len() < 4 && due) {
                self.scan();
            }
            let pumped = {
                let Some(pad) = self.pads.get_mut(index) else {
                    return Err(DeviceError::Disconnected);
                };
                pad.pump()
            };
            match pumped {
                Ok(packet) => Ok(packet),
                Err(error) => {
                    self.pads.remove(index);
                    self.dirty = true;
                    Err(error)
                }
            }
        }

        fn scan(&mut self) {
            self.last_scan = Some(Instant::now());
            self.dirty = false;
            let mut paths = match std::fs::read_dir("/dev/input") {
                Ok(entries) => entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                            let rest = name.strip_prefix("event");
                            rest.is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
                        })
                    })
                    .collect::<Vec<_>>(),
                Err(error) => {
                    self.pads.clear();
                    self.complain(error.kind() == ErrorKind::PermissionDenied);
                    return;
                }
            };
            paths.sort();
            let mut kept = Vec::new();
            let mut denied = 0u32;
            let mut opened = 0u32;
            for path in paths {
                if let Some(index) = self.pads.iter().position(|pad| pad.path == path) {
                    kept.push(self.pads.swap_remove(index));
                    continue;
                }
                if kept.len() >= 4 {
                    continue;
                }
                match Device::open(&path) {
                    Ok(device) => {
                        opened += 1;
                        if !is_gamepad(&device) {
                            continue;
                        }
                        if let Ok(pad) = OpenPad::open(path, device) {
                            kept.push(pad);
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::PermissionDenied => denied += 1,
                    Err(_) => {}
                }
            }
            kept.sort_by(|left, right| left.path.cmp(&right.path));
            kept.truncate(4);
            self.pads = kept;
            if self.pads.is_empty() && denied > 0 && opened == 0 {
                self.complain(true);
            }
        }

        fn complain(&mut self, denied: bool) {
            if !denied || self.complained {
                return;
            }
            self.complained = true;
            eprintln!(
                "Gamepad input is not readable (/dev/input/event* denied). Add this user to the input group, or grant the local session access, then start the game again."
            );
        }
    }

    fn is_gamepad(device: &Device) -> bool {
        device.supported_keys().is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH))
    }

    fn hub() -> std::sync::MutexGuard<'static, Hub> {
        static HUB: Mutex<Hub> = Mutex::new(Hub::new());
        HUB.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    pub(super) fn poll(index: usize, cache: &mut CapabilityCache) -> Result<DevicePacket, DeviceError> {
        let (number, state) = match hub().sample(index) {
            Ok(packet) => packet,
            Err(error) => {
                cache.invalidate();
                return Err(error);
            }
        };
        // Linux gamepads are ordinary pads. Subtype 7 is the guitar layout and would
        // zero the triggers inside the native converter.
        let subtype = cache.get(std::time::Instant::now(), || Ok(1))?;
        Ok(DevicePacket { number, state, subtype })
    }

}

pub(crate) fn poll_cached(
    index: usize,
    cache: &mut CapabilityCache,
) -> Result<DevicePacket, DeviceError> {
    assert!(index < 4);
    #[cfg(windows)]
    return windows::poll(index as u32, cache);
    #[cfg(target_os = "linux")]
    return linux::poll(index, cache);
    #[cfg(not(any(windows, target_os = "linux")))]
    Err(DeviceError::UnsupportedPlatform)
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn capability_cache_refreshes_and_never_caches_errors() {
        let start = std::time::Instant::now();
        let mut cache = CapabilityCache::default();
        assert_eq!(cache.get(start, || Ok(1)), Ok(1));
        assert_eq!(
            cache.get(start + std::time::Duration::from_millis(999), || panic!(
                "redundant capability query"
            )),
            Ok(1)
        );
        assert_eq!(
            cache.get(start + std::time::Duration::from_secs(1), || Ok(2)),
            Ok(2)
        );
        cache.invalidate();
        assert_eq!(
            cache.get(start, || Err(DeviceError::Capabilities(5))),
            Err(DeviceError::Capabilities(5))
        );
        assert_eq!(cache.get(start, || Ok(3)), Ok(3));
        cache.invalidate();
        assert_eq!(cache.get(start, || Ok(4)), Ok(4));
    }
}

#[cfg(test)]
mod linux_map_tests {
    use super::{AbsSample, xbox_from_linux};

    fn axis(code: u16, value: i32, min: i32, max: i32) -> AbsSample {
        AbsSample { code, value, min, max }
    }

    #[test]
    fn face_buttons_and_resting_sticks() {
        let state = xbox_from_linux(&[0x130, 0x134], &[]);
        assert_eq!(state.buttons, 0x1000 | 0x4000);
        assert_eq!(state.triggers, [0, 0]);
        assert_eq!(state.left, [0, 0]);
        assert_eq!(state.right, [0, 0]);
    }

    #[test]
    fn stick_y_is_negated_into_xinput_up() {
        let full = [axis(0x00, 32767, -32768, 32767), axis(0x01, -32767, -32768, 32767)];
        let state = xbox_from_linux(&[], &full);
        assert_eq!(state.left, [32767, 32767]);
        let down = [axis(0x01, 32767, -32768, 32767), axis(0x04, -32768, -32768, 32767)];
        let state = xbox_from_linux(&[], &down);
        assert_eq!(state.left[1], -32767);
        assert_eq!(state.right[1], 32767);
    }

    #[test]
    fn hat_dpad_and_xpad_triggers() {
        let axes = [
            axis(0x10, -1, -1, 1),
            axis(0x11, 1, -1, 1),
            axis(0x02, 255, 0, 255),
            axis(0x05, 0, 0, 255),
        ];
        let state = xbox_from_linux(&[], &axes);
        assert_eq!(state.buttons & 0x000f, 0x0004 | 0x0002);
        assert_eq!(state.triggers, [255, 0]);
    }

    #[test]
    fn spec_triggers_win_over_xpad_axes_and_digital_fallback() {
        let axes = [axis(0x15, 0, 0, 1023), axis(0x02, 255, 0, 255), axis(0x14, 512, 0, 1023)];
        let state = xbox_from_linux(&[0x138, 0x139], &axes);
        assert_eq!(state.triggers[0], 0);
        assert_eq!(state.triggers[1], ((512i64 * 255) / 1023) as u8);
        let digital = xbox_from_linux(&[0x138], &[]);
        assert_eq!(digital.triggers, [255, 0]);
    }
}

// Preserve the uncached API for menu-only polling.
pub(crate) fn poll(index: usize) -> Result<DevicePacket, DeviceError> {
    poll_cached(index, &mut CapabilityCache::default())
}
