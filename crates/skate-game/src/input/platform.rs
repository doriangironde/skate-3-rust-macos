//! Device transport. Windows reads raw XInput; Linux and macOS map gilrs controller
//! states to Xbox values on a background input worker.
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
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod gilrs_backend {
    use super::{CapabilityCache, DeviceError, DevicePacket};
    use gilrs::{Axis, Button, EventType, Gamepad, GilrsBuilder};
    use skate_core::input::xbox::XboxState;
    use std::{sync::{Arc, Mutex, OnceLock}, time::Duration};

    #[derive(Clone, Copy, Default, PartialEq, Eq)]
    struct PadState {
        buttons: u16,
        triggers: [u8; 2],
        left: [i16; 2],
        right: [i16; 2],
    }

    #[derive(Default)]
    struct Snapshot {
        slots: [Option<PadState>; 4],
        numbers: [u32; 4],
    }

    fn stick(value: f32) -> i16 {
        (value.clamp(-1., 1.) * 32767.).round() as i16
    }
    fn trigger(value: f32) -> u8 {
        (value.clamp(0., 1.) * 255.).round() as u8
    }

    fn buttons(pressed: impl Fn(Button) -> bool) -> u16 {
        let mut bits = 0;
        for (button, bit) in [
            (Button::South, 0x1000), (Button::East, 0x2000),
            (Button::West, 0x4000), (Button::North, 0x8000),
            (Button::LeftTrigger, 0x0100), (Button::RightTrigger, 0x0200),
            (Button::Select, 0x0020), (Button::Start, 0x0010),
            (Button::LeftThumb, 0x0040), (Button::RightThumb, 0x0080),
            (Button::DPadUp, 0x0001), (Button::DPadDown, 0x0002),
            (Button::DPadLeft, 0x0004), (Button::DPadRight, 0x0008),
        ] {
            if pressed(button) { bits |= bit; }
        }
        bits
    }

    fn dpad(x: f32, y: f32) -> u16 {
        (if x < -0.5 { 0x0004 } else if x > 0.5 { 0x0008 } else { 0 })
            | (if y > 0.5 { 0x0001 } else if y < -0.5 { 0x0002 } else { 0 })
    }

    fn state(g: &Gamepad<'_>) -> PadState {
        // Gilrs normalizes stick Y to positive-up, like XInput.
        PadState {
            buttons: buttons(|button| g.is_pressed(button))
                | dpad(g.value(Axis::DPadX), g.value(Axis::DPadY)),
            triggers: [(Button::LeftTrigger2, Axis::LeftZ), (Button::RightTrigger2, Axis::RightZ)]
                .map(|(button, axis)| trigger(g.button_data(button).map(|data| data.value())
                    .or_else(|| g.axis_data(axis).map(|data| (data.value() + 1.) * 0.5))
                    .unwrap_or(0.))),
            left: [stick(g.value(Axis::LeftStickX)), stick(g.value(Axis::LeftStickY))],
            right: [stick(g.value(Axis::RightStickX)), stick(g.value(Axis::RightStickY))],
        }
    }

    // Retain occupied slots on disconnect; a new device fills the lowest vacancy.
    fn reconcile<T: Copy + Eq>(ids: &mut [Option<T>; 4], live: &[T]) {
        for slot in ids.iter_mut() {
            if slot.is_some_and(|id| !live.contains(&id)) { *slot = None; }
        }
        for &id in live {
            if ids.contains(&Some(id)) { continue; }
            if let Some(slot) = ids.iter_mut().find(|slot| slot.is_none()) { *slot = Some(id); }
        }
    }

    fn publish<T: Copy + Eq>(snapshot: &mut Snapshot, before: [Option<T>; 4], after: [Option<T>; 4], next: [Option<PadState>; 4], lifecycle: &[T]) {
        for i in 0..4 {
            if before[i] != after[i] || snapshot.slots[i] != next[i]
                || after[i].is_some_and(|id| lifecycle.contains(&id)) {
                snapshot.numbers[i] = snapshot.numbers[i].wrapping_add(1);
                snapshot.slots[i] = next[i];
            }
        }
    }

    fn run(shared: Arc<Mutex<Snapshot>>) {
        // Gameplay applies its own deadzone. Map axis D-pads above instead of enabling gilrs filters.
        let mut gilrs = match GilrsBuilder::new().with_default_filters(false).with_force_feedback(false).build() {
            Ok(gilrs) => gilrs,
            Err(error) => { eprintln!("Controller initialization failed: {error}"); return; }
        };
        let mut ids = [None; 4];
        loop {
            // Gilrs owns device discovery/events on this worker, never under the snapshot lock.
            // Timeout also samples initial state and clears disconnected controllers.
            let mut lifecycle = Vec::new();
            let first = gilrs.next_event_blocking(Some(Duration::from_millis(8)));
            for event in first.into_iter().chain((0..128).map_while(|_| gilrs.next_event())) {
                if matches!(event.event, EventType::Connected | EventType::Disconnected) {
                    lifecycle.push(event.id);
                }
            }
            let live: Vec<_> = gilrs.gamepads().map(|(id, _)| id).collect();
            let previous_ids = ids;
            reconcile(&mut ids, &live);
            let next = ids.map(|id| id.map(|id| state(&gilrs.gamepad(id))));
            let mut snapshot = shared.lock().unwrap_or_else(|e| e.into_inner());
            publish(&mut snapshot, previous_ids, ids, next, &lifecycle);
        }
    }

    fn shared() -> &'static Arc<Mutex<Snapshot>> {
        static STATE: OnceLock<Arc<Mutex<Snapshot>>> = OnceLock::new();
        STATE.get_or_init(|| {
            let state = Arc::new(Mutex::new(Snapshot::default()));
            let worker = Arc::clone(&state);
            if let Err(error) = std::thread::Builder::new().name("gilrs-input".into()).spawn(move || run(worker)) {
                eprintln!("Controller worker failed: {error}");
            }
            state
        })
    }

    pub(super) fn poll(index: usize, cache: &mut CapabilityCache) -> Result<DevicePacket, DeviceError> {
        let snapshot = shared().lock().unwrap_or_else(|e| e.into_inner());
        let Some(state) = snapshot.slots[index] else {
            cache.invalidate();
            return Err(DeviceError::Disconnected);
        };
        Ok(DevicePacket {
            number: snapshot.numbers[index],
            state: XboxState { buttons: state.buttons, triggers: state.triggers,
                left: state.left, right: state.right },
            subtype: 1,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn neutral_and_axis_endpoints() {
            assert_eq!(stick(0.), 0);
            assert_eq!(stick(1.), i16::MAX);
            assert_eq!(stick(-1.), -i16::MAX);
            assert_eq!(trigger(0.), 0);
            assert_eq!(trigger(1.), 255);
            assert_eq!(trigger(0.5), 128);
        }
        #[test]
        fn stable_slots_and_disconnect() {
            let (a, b, c) = (1, 2, 3);
            let mut ids = [None; 4];
            reconcile(&mut ids, &[a, b]);
            assert_eq!(ids, [Some(a), Some(b), None, None]);
            reconcile(&mut ids, &[b, c]);
            assert_eq!(ids, [Some(c), Some(b), None, None]);
            reconcile(&mut ids, &[]);
            assert_eq!(ids, [None; 4]);
        }
        #[test]
        fn controller_buttons_keep_xbox_layout() {
            assert_eq!(buttons(|_| false), 0);
            assert_eq!(buttons(|button| button == Button::South), 0x1000);
            assert_eq!(buttons(|button| button == Button::West), 0x4000);
            assert_eq!(buttons(|button| button == Button::DPadUp), 0x0001);
            assert_eq!(buttons(|button| button == Button::LeftTrigger), 0x0100);
            assert_eq!(dpad(0., 0.), 0);
            assert_eq!(dpad(-1., 1.), 0x0004 | 0x0001);
            assert_eq!(dpad(1., -1.), 0x0008 | 0x0002);
        }
        #[test]
        fn snapshot_tracks_changes_and_replacement_even_with_identical_buttons() {
            let mut snapshot = Snapshot::default();
            let neutral = PadState::default();
            publish(&mut snapshot, [None; 4], [Some(1); 4], [Some(neutral); 4], &[]);
            assert_eq!(snapshot.numbers, [1; 4]);
            publish(&mut snapshot, [Some(1); 4], [Some(1); 4], [Some(neutral); 4], &[]);
            assert_eq!(snapshot.numbers, [1; 4]);
            let pressed = PadState { buttons: 0x1000, ..neutral };
            publish(&mut snapshot, [Some(1); 4], [Some(1); 4], [Some(pressed); 4], &[]);
            assert_eq!(snapshot.numbers, [2; 4]);
            publish(&mut snapshot, [Some(1); 4], [None; 4], [None; 4], &[]);
            assert_eq!(snapshot.numbers, [3; 4]);
            publish(&mut snapshot, [None; 4], [Some(2); 4], [Some(neutral); 4], &[]);
            publish(&mut snapshot, [Some(2); 4], [Some(3); 4], [Some(neutral); 4], &[]);
            assert_eq!(snapshot.numbers, [5; 4]);
            publish(&mut snapshot, [Some(3); 4], [Some(3); 4], [Some(neutral); 4], &[3]);
            assert_eq!(snapshot.numbers, [6; 4]);
        }
    }
}

pub(crate) fn poll_cached(
    index: usize,
    cache: &mut CapabilityCache,
) -> Result<DevicePacket, DeviceError> {
    assert!(index < 4);
    #[cfg(windows)]
    return windows::poll(index as u32, cache);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    return gilrs_backend::poll(index, cache);
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
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

// Preserve the uncached API for menu-only polling.
pub(crate) fn poll(index: usize) -> Result<DevicePacket, DeviceError> {
    poll_cached(index, &mut CapabilityCache::default())
}
