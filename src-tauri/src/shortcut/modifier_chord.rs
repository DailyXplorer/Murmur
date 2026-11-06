//! Shortcuts made of modifiers alone, such as Ctrl+Win, on Windows.
//!
//! `RegisterHotKey`, behind the global-shortcut plugin, needs a main key, so
//! these chords are read from a low-level keyboard hook (`WH_KEYBOARD_LL`).
//! The hook only watches: every key still reaches the app in front. A chord
//! is held while exactly its modifiers are down and no other key is; any
//! other key ends it, as in Ctrl+Win+D, until every key is released.

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};

use log::{error, info};
use tauri::AppHandle;

const CONTROL: u8 = 1;
const SHIFT: u8 = 2;
const ALT: u8 = 4;
const WIN: u8 = 8;

/// A set of modifiers held together, with no main key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord(u8);

impl Chord {
    /// Two or more distinct modifiers and nothing else, in the spelling of
    /// the global-shortcut plugin: `ctrl+super`, `Ctrl+Win`, `alt+shift`...
    /// A lone modifier is refused: it would fire on every Ctrl+C.
    pub fn parse(raw: &str) -> Option<Self> {
        let mut mask = 0;
        let mut count = 0;
        for part in raw.split('+') {
            let modifier = match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" => CONTROL,
                "shift" => SHIFT,
                "alt" | "option" => ALT,
                "super" | "win" | "windows" | "meta" | "command" | "cmd" => WIN,
                _ => return None,
            };
            if mask & modifier != 0 {
                return None;
            }
            mask |= modifier;
            count += 1;
        }
        (count >= 2).then_some(Self(mask))
    }

    /// Releasing Win or Alt alone opens the Start menu or the menu bar; a
    /// key event while it is held stops that, as AutoHotkey's menu mask key
    /// does.
    fn needs_menu_mask(self) -> bool {
        self.0 & (WIN | ALT) != 0
    }
}

/// The modifier a virtual-key code stands for, left and right alike.
fn modifier_of(virtual_key: u32) -> Option<u8> {
    match virtual_key {
        0x10 | 0xA0 | 0xA1 => Some(SHIFT),
        0x11 | 0xA2 | 0xA3 => Some(CONTROL),
        0x12 | 0xA4 | 0xA5 => Some(ALT),
        0x5B | 0x5C => Some(WIN),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transition {
    Pressed(Chord),
    Released(Chord),
}

/// Which keys are down, and the chord they hold, if any.
#[derive(Default)]
struct Tracker {
    modifiers: Vec<u32>,
    others: Vec<u32>,
    active: Option<Chord>,
    /// Another key went down with modifiers held: no chord until all keys
    /// are up again.
    interrupted: bool,
}

impl Tracker {
    /// Forgets keys whose release the hook never saw: Windows does not show
    /// it the keys of the secure desktop, so Win+L leaves Win "down".
    fn forget_released(&mut self, is_down: impl Fn(u32) -> bool) {
        self.modifiers.retain(|key| is_down(*key));
        self.others.retain(|key| is_down(*key));
        if self.modifiers.is_empty() && self.others.is_empty() {
            self.interrupted = false;
        }
    }

    fn key(&mut self, virtual_key: u32, down: bool, chords: &[Chord]) -> Option<Transition> {
        let keys = if modifier_of(virtual_key).is_some() {
            &mut self.modifiers
        } else {
            if down && (!self.modifiers.is_empty() || self.active.is_some()) {
                self.interrupted = true;
            }
            &mut self.others
        };
        if down {
            if !keys.contains(&virtual_key) {
                keys.push(virtual_key);
            }
        } else {
            keys.retain(|key| *key != virtual_key);
        }

        let mask = self
            .modifiers
            .iter()
            .filter_map(|key| modifier_of(*key))
            .fold(0, |mask, modifier| mask | modifier);
        if mask == 0 && self.others.is_empty() {
            self.interrupted = false;
        }
        let clear = !self.interrupted && self.others.is_empty();
        if let Some(chord) = self.active {
            if chord.0 == mask && clear {
                return None;
            }
            self.active = None;
            return Some(Transition::Released(chord));
        }
        let chord = *chords.iter().find(|chord| clear && chord.0 == mask)?;
        self.active = Some(chord);
        Some(Transition::Pressed(chord))
    }
}

/// A registered chord's binding id and its text, as the shortcut handler
/// wants them.
type Binding = (String, String);

static REGISTERED: Mutex<Vec<(Chord, Binding)>> = Mutex::new(Vec::new());
static HOOK: OnceLock<Result<(), String>> = OnceLock::new();

fn registered() -> std::sync::MutexGuard<'static, Vec<(Chord, Binding)>> {
    REGISTERED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn is_registered(chord: Chord) -> bool {
    registered()
        .iter()
        .any(|(registered, _)| *registered == chord)
}

/// Starts delivering `chord` to the shortcut handler as `id`.
pub fn register(app: &AppHandle, id: &str, text: &str, chord: Chord) -> Result<(), String> {
    HOOK.get_or_init(|| hook::start(app.clone())).clone()?;
    let mut registered = registered();
    if registered
        .iter()
        .any(|(registered, _)| *registered == chord)
    {
        return Err(format!("Shortcut '{text}' is already in use"));
    }
    registered.push((chord, (id.to_string(), text.to_string())));
    Ok(())
}

pub fn unregister(chord: Chord) -> Result<(), String> {
    let mut registered = registered();
    let before = registered.len();
    registered.retain(|(registered, _)| *registered != chord);
    if registered.len() == before {
        return Err("Shortcut is not registered".to_string());
    }
    Ok(())
}

mod hook {
    use super::*;
    use std::cell::RefCell;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
        HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN,
        WM_SYSKEYDOWN,
    };

    /// An unassigned virtual key, the menu mask key AutoHotkey also uses.
    const MENU_MASK_KEY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);
    /// Asks the hook thread to send the menu mask once the hook returned:
    /// synthesizing input from inside the hook would re-enter it.
    const WM_SEND_MENU_MASK: u32 = WM_APP + 1;

    struct HookState {
        tracker: Tracker,
        /// The binding pressed, kept so its release reaches the handler even
        /// if the binding was unregistered in between.
        pressed: Option<Binding>,
        events: Sender<(Binding, bool)>,
        thread: u32,
    }

    thread_local! {
        static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
    }

    fn is_down(virtual_key: u32) -> bool {
        // SAFETY: plain query of the asynchronous key state.
        (unsafe { GetAsyncKeyState(virtual_key as i32) } as u16 & 0x8000) != 0
    }

    fn on_key(virtual_key: u32, down: bool) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let Some(state) = state.as_mut() else {
                return;
            };
            // The asynchronous state does not include this event yet.
            state
                .tracker
                .forget_released(|key| key == virtual_key || is_down(key));
            let registered = registered();
            let chords: Vec<Chord> = registered.iter().map(|(chord, _)| *chord).collect();
            match state.tracker.key(virtual_key, down, &chords) {
                Some(Transition::Pressed(chord)) => {
                    let Some((_, binding)) = registered.iter().find(|(c, _)| *c == chord) else {
                        return;
                    };
                    if chord.needs_menu_mask() {
                        // SAFETY: posts to this very thread's queue.
                        let _ = unsafe {
                            PostThreadMessageW(
                                state.thread,
                                WM_SEND_MENU_MASK,
                                WPARAM(0),
                                LPARAM(0),
                            )
                        };
                    }
                    state.pressed = Some(binding.clone());
                    let _ = state.events.send((binding.clone(), true));
                }
                Some(Transition::Released(_)) => {
                    if let Some(binding) = state.pressed.take() {
                        let _ = state.events.send((binding, false));
                    }
                }
                None => {}
            }
        });
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: for HC_ACTION, `lparam` points to a KBDLLHOOKSTRUCT.
            let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            // Murmur's own Ctrl+V and menu mask are injected.
            if !event.flags.contains(LLKHF_INJECTED) {
                let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                on_key(event.vkCode, down);
            }
        }
        // SAFETY: passes the event on unchanged; the hook never swallows keys.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn send_menu_mask() {
        let key = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: MENU_MASK_KEY,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
        // SAFETY: two initialized keyboard INPUTs.
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    }

    /// Starts the hook thread, and a dispatcher that hands chords to the
    /// shortcut handler off the hook: a slow hook is removed by Windows.
    pub(super) fn start(app: AppHandle) -> Result<(), String> {
        let (events, received) = mpsc::channel::<(Binding, bool)>();
        std::thread::Builder::new()
            .name("modifier-chords".into())
            .spawn(move || {
                for ((id, text), pressed) in received {
                    crate::shortcut::handler::handle_shortcut_event(&app, &id, &text, pressed);
                }
            })
            .map_err(|e| format!("Couldn't start the shortcut dispatcher: {e}"))?;

        let (ready, installed) = mpsc::channel();
        std::thread::Builder::new()
            .name("keyboard-hook".into())
            .spawn(move || {
                // SAFETY: installs a hook for this thread's lifetime and
                // pumps its messages, as low-level hooks require.
                unsafe {
                    let module = match GetModuleHandleW(PCWSTR::null()) {
                        Ok(module) => HINSTANCE(module.0),
                        Err(e) => {
                            let _ = ready.send(Err(format!("GetModuleHandle failed: {e}")));
                            return;
                        }
                    };
                    if let Err(e) =
                        SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(module), 0)
                    {
                        let _ = ready.send(Err(format!("Couldn't watch the keyboard: {e}")));
                        return;
                    }
                    STATE.with(|state| {
                        *state.borrow_mut() = Some(HookState {
                            tracker: Tracker::default(),
                            pressed: None,
                            events,
                            thread: GetCurrentThreadId(),
                        })
                    });
                    let _ = ready.send(Ok(()));
                    let mut message = MSG::default();
                    while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                        if message.message == WM_SEND_MENU_MASK {
                            send_menu_mask();
                        } else {
                            DispatchMessageW(&message);
                        }
                    }
                }
            })
            .map_err(|e| format!("Couldn't start the keyboard hook: {e}"))?;
        let result = installed
            .recv()
            .unwrap_or_else(|_| Err("The keyboard hook thread stopped".to_string()));
        match &result {
            Ok(()) => info!("Watching the keyboard for modifier-only shortcuts"),
            Err(e) => error!("{e}"),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT_CONTROL: u32 = 0xA2;
    const RIGHT_CONTROL: u32 = 0xA3;
    const LEFT_SHIFT: u32 = 0xA0;
    const LEFT_WIN: u32 = 0x5B;
    const KEY_D: u32 = 0x44;
    const KEY_A: u32 = 0x41;

    fn ctrl_win() -> Chord {
        Chord::parse("ctrl+super").unwrap()
    }

    /// Feeds key events and collects the transitions.
    fn play(tracker: &mut Tracker, events: &[(u32, bool)]) -> Vec<Transition> {
        events
            .iter()
            .filter_map(|(key, down)| tracker.key(*key, *down, &[ctrl_win()]))
            .collect()
    }

    #[test]
    fn chords_are_modifiers_alone_in_any_spelling_or_order() {
        assert_eq!(Chord::parse("Ctrl + Win"), Some(ctrl_win()));
        assert_eq!(Chord::parse("super+control"), Some(ctrl_win()));
        assert!(Chord::parse("ctrl+shift+alt").is_some());
        for refused in [
            "ctrl",
            "ctrl+ctrl",
            "ctrl+space",
            "ctrl+super+v",
            "",
            "fn+ctrl",
        ] {
            assert_eq!(Chord::parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn holding_ctrl_and_win_in_either_order_presses_the_chord() {
        for (first, second) in [(LEFT_CONTROL, LEFT_WIN), (LEFT_WIN, LEFT_CONTROL)] {
            let mut tracker = Tracker::default();
            assert_eq!(
                play(
                    &mut tracker,
                    &[(first, true), (second, true), (second, false)]
                ),
                [
                    Transition::Pressed(ctrl_win()),
                    Transition::Released(ctrl_win())
                ]
            );
        }
    }

    #[test]
    fn key_repeat_and_the_other_ctrl_key_keep_the_chord_held() {
        let mut tracker = Tracker::default();
        let events = [
            (LEFT_CONTROL, true),
            (LEFT_WIN, true),
            (LEFT_WIN, true),
            (RIGHT_CONTROL, true),
            (LEFT_CONTROL, false),
        ];
        assert_eq!(
            play(&mut tracker, &events),
            [Transition::Pressed(ctrl_win())]
        );
    }

    #[test]
    fn another_key_ends_the_chord_until_every_key_is_released() {
        let mut tracker = Tracker::default();
        let switch_desktop = [
            (LEFT_CONTROL, true),
            (LEFT_WIN, true),
            (KEY_D, true),
            (KEY_D, false),
            (LEFT_WIN, false),
            (LEFT_WIN, true),
        ];
        assert_eq!(
            play(&mut tracker, &switch_desktop),
            [
                Transition::Pressed(ctrl_win()),
                Transition::Released(ctrl_win())
            ]
        );
        let rearmed = [
            (LEFT_WIN, false),
            (LEFT_CONTROL, false),
            (LEFT_CONTROL, true),
            (LEFT_WIN, true),
        ];
        assert_eq!(
            play(&mut tracker, &rearmed),
            [Transition::Pressed(ctrl_win())]
        );
    }

    #[test]
    fn extra_modifiers_or_a_held_key_are_another_shortcut() {
        let mut tracker = Tracker::default();
        let with_shift = [(LEFT_CONTROL, true), (LEFT_SHIFT, true), (LEFT_WIN, true)];
        assert!(play(&mut tracker, &with_shift).is_empty());

        let mut tracker = Tracker::default();
        let while_typing = [(KEY_A, true), (LEFT_CONTROL, true), (LEFT_WIN, true)];
        assert!(play(&mut tracker, &while_typing).is_empty());

        let mut tracker = Tracker::default();
        let then_shift = [(LEFT_CONTROL, true), (LEFT_WIN, true), (LEFT_SHIFT, true)];
        assert_eq!(
            play(&mut tracker, &then_shift),
            [
                Transition::Pressed(ctrl_win()),
                Transition::Released(ctrl_win())
            ]
        );
    }

    #[test]
    fn a_win_key_released_on_the_lock_screen_is_forgotten() {
        let mut tracker = Tracker::default();
        // Win+L: the hook sees Win and L go down, never their release.
        play(&mut tracker, &[(LEFT_WIN, true), (0x4C, true)]);
        tracker.forget_released(|_| false);
        // Back on the desktop, Ctrl alone is not Ctrl+Win.
        assert!(play(&mut tracker, &[(LEFT_CONTROL, true)]).is_empty());
        assert_eq!(
            play(&mut tracker, &[(LEFT_WIN, true)]),
            [Transition::Pressed(ctrl_win())]
        );
    }

    #[test]
    fn only_chords_with_win_or_alt_mask_the_menus() {
        assert!(ctrl_win().needs_menu_mask());
        assert!(Chord::parse("ctrl+alt").unwrap().needs_menu_mask());
        assert!(!Chord::parse("ctrl+shift").unwrap().needs_menu_mask());
    }

    #[test]
    fn a_chord_registers_once() {
        let chord = Chord::parse("ctrl+shift+alt").unwrap();
        registered().push((chord, ("test".into(), "ctrl+shift+alt".into())));
        assert!(is_registered(chord));
        unregister(chord).unwrap();
        assert!(!is_registered(chord));
        assert!(unregister(chord).is_err());
    }
}
