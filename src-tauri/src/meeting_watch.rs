//! Meeting detection: offer (or start) a recording when another app begins
//! using the microphone.
//!
//! The signal is CoreAudio's audio-process list. Every process that opens an
//! input stream appears there with `kAudioProcessPropertyIsRunningInput`, which
//! is the same event whether the call lives in Zoom, Slack, Meet, Teams or
//! FaceTime. Deliberately no bundle-id allowlist: browsers host Meet and Slack
//! huddles, PWAs masquerade as their parent browser, and Teams has shipped
//! under several ids — the microphone itself never lies.
//!
//! Off by default (`meeting_watch` = `off`); the Recording section of Settings
//! picks between `notify` and `record`.

use std::ffi::c_void;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::{db, menubar, AppState};

/// Settings key. Values: `off` (default), `notify`, `record`.
pub(crate) const SETTING_KEY: &str = "meeting_watch";

const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Consecutive polls another app must hold the mic before we act (~10 s).
/// Call UIs grab the device a beat before they're ready, and one poll is close
/// enough to a voice memo or a permission check to be worth ignoring.
const TRIGGER_POLLS: u32 = 2;

/// How long the mic has to stay free before the next meeting can trigger.
const REARM_POLLS: u32 = 2;

// ---------------------------------------------------------------------------
// CoreAudio process objects
// ---------------------------------------------------------------------------

const SYSTEM_OBJECT: u32 = 1;
const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const ELEMENT_MAIN: u32 = 0;
const PROCESS_OBJECT_LIST: u32 = fourcc(b"prs#");
const PROCESS_PID: u32 = fourcc(b"ppid");
const PROCESS_IS_RUNNING_INPUT: u32 = fourcc(b"piri");

const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyDataSize(
        object: u32,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> i32;
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}

fn global_address(selector: u32) -> PropertyAddress {
    PropertyAddress { selector, scope: SCOPE_GLOBAL, element: ELEMENT_MAIN }
}

fn read_u32(object: u32, selector: u32) -> Option<u32> {
    let address = global_address(selector);
    let mut size = std::mem::size_of::<u32>() as u32;
    let mut value: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut value as *mut u32 as *mut c_void,
        )
    };
    (status == 0).then_some(value)
}

fn process_objects() -> Vec<u32> {
    let address = global_address(PROCESS_OBJECT_LIST);
    let mut size: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &address, 0, std::ptr::null(), &mut size)
    };
    if status != 0 || size == 0 {
        return Vec::new();
    }
    let count = size as usize / std::mem::size_of::<u32>();
    let mut objects = vec![0u32; count];
    let mut written = size;
    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &address,
            0,
            std::ptr::null(),
            &mut written,
            objects.as_mut_ptr() as *mut c_void,
        )
    };
    if status != 0 {
        return Vec::new();
    }
    objects.truncate(written as usize / std::mem::size_of::<u32>());
    objects
}

/// Is a process other than Humla capturing audio input right now?
///
/// Humla's own capture is excluded by pid: once an auto-started recording is
/// running, it must not keep re-triggering the detector.
fn foreign_input_active() -> bool {
    let self_pid = std::process::id();
    process_objects().into_iter().any(|object| {
        read_u32(object, PROCESS_IS_RUNNING_INPUT).unwrap_or(0) != 0
            && read_u32(object, PROCESS_PID) != Some(self_pid)
    })
}

// ---------------------------------------------------------------------------
// Debounce
// ---------------------------------------------------------------------------

/// Turns a stream of "another app has the mic" polls into one trigger per
/// meeting. Pure so the timing rules are testable without audio hardware.
#[derive(Default)]
struct Debounce {
    active_polls: u32,
    idle_polls: u32,
    latched: bool,
}

impl Debounce {
    /// Feed one poll; returns true exactly once per meeting, when the mic has
    /// been held long enough to count.
    fn observe(&mut self, active: bool) -> bool {
        if active {
            self.idle_polls = 0;
            self.active_polls = self.active_polls.saturating_add(1);
            if !self.latched && self.active_polls >= TRIGGER_POLLS {
                self.latched = true;
                return true;
            }
            return false;
        }

        self.active_polls = 0;
        self.idle_polls = self.idle_polls.saturating_add(1);
        if self.latched && self.idle_polls >= REARM_POLLS {
            self.latched = false;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

fn mode(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    let conn = state.db.lock();
    db::get_setting(&conn, SETTING_KEY)
        .ok()
        .flatten()
        .unwrap_or_else(|| "off".to_string())
}

fn act(app: &AppHandle) {
    let mode = mode(app);
    if mode == "off" || menubar::capture_in_flight(app) {
        return;
    }
    if mode == "record" {
        menubar::notify(
            app,
            "Meeting detected",
            "Recording started. Press the record shortcut to stop.",
        );
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            menubar::headless_start(&app).await;
        });
        return;
    }
    menubar::notify(
        app,
        "Meeting detected",
        "Press the record shortcut (⌃⌘R by default) to capture it.",
    );
}

/// Watch for meeting apps taking the microphone for as long as the app runs.
///
/// Called from `setup`, so it must not assume tokio is attached yet.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut debounce = Debounce::default();
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            if debounce.observe(foreign_input_active()) {
                act(&app);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_trigger_per_meeting_and_rearms_after_the_mic_goes_quiet() {
        let mut d = Debounce::default();

        assert!(!d.observe(true), "a single poll is not a meeting");
        assert!(d.observe(true), "the second consecutive poll triggers");
        assert!(!d.observe(true), "a held mic must not trigger again");
        assert!(!d.observe(true));

        assert!(!d.observe(false), "one idle poll keeps the latch");
        assert!(!d.observe(false), "rearm poll resets the latch");
        assert!(!d.observe(true), "a new meeting needs its own two polls");
        assert!(d.observe(true), "and triggers once it has them");
    }

    #[test]
    fn a_blip_that_lets_go_of_the_mic_never_triggers() {
        let mut d = Debounce::default();
        for _ in 0..10 {
            assert!(!d.observe(true));
            assert!(!d.observe(false));
        }
    }
}
