use std::cell::Cell;

use windows::Win32::{
  Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
  UI::{
    Input::KeyboardAndMouse::{
      GetKeyState, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL,
      VK_RMENU, VK_RSHIFT, VK_RWIN,
    },
    WindowsAndMessaging::{
      CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
      KBDLLHOOKSTRUCT, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
    },
  },
};

use crate::{Dispatcher, Key, KeyCode};

thread_local! {
  /// Stores the hook callback for the current thread.
  ///
  /// The hook callback is called for every keyboard event and returns
  /// `true` if the event should be intercepted.
  static HOOK: Cell<Option<Box<dyn FnMut(KeyEvent) -> bool>>> = Cell::default();
}

/// Windows-specific keyboard event.
#[derive(Clone, Debug)]
pub struct KeyEvent {
  /// The key that was pressed or released.
  pub key: Key,

  /// Key code that generated this event.
  pub key_code: KeyCode,

  /// Whether the event is for a key press or release.
  pub is_keypress: bool,
}

impl KeyEvent {
  /// Creates an instance of `KeyEvent`.
  pub(crate) fn new(
    key: Key,
    key_code: KeyCode,
    is_keypress: bool,
  ) -> Self {
    Self {
      is_keypress,
      key_code,
      key,
    }
  }

  /// Gets whether the specified key is currently pressed.
  pub fn is_key_down(&self, key: Key) -> bool {
    match key {
      Key::Cmd | Key::Win => {
        Self::is_key_down_raw(VK_LWIN.0)
          || Self::is_key_down_raw(VK_RWIN.0)
      }
      Key::Alt => {
        Self::is_key_down_raw(VK_LMENU.0)
          || Self::is_key_down_raw(VK_RMENU.0)
      }
      Key::Ctrl => {
        Self::is_key_down_raw(VK_LCONTROL.0)
          || Self::is_key_down_raw(VK_RCONTROL.0)
      }
      Key::Shift => {
        Self::is_key_down_raw(VK_LSHIFT.0)
          || Self::is_key_down_raw(VK_RSHIFT.0)
      }
      _ => {
        let key_code = match KeyCode::try_from(key) {
          Ok(code) => code,
          Err(e) => {
            tracing::warn!(
              "Key has no corresponding KeyCode: {e} | Cannot check if key is down"
            );
            return false;
          }
        };
        Self::is_key_down_raw(key_code.0)
      }
    }
  }

  /// Gets whether the specified key is currently down using the raw key
  /// code.
  fn is_key_down_raw(key: u16) -> bool {
    unsafe { (GetKeyState(key.into()) & 0x80) == 0x80 }
  }
}

/// Wrapper for the low-level keyboard hook API.
#[derive(Debug)]
pub struct KeyboardHook {
  handle: HHOOK,
}

impl KeyboardHook {
  /// Creates a new low-level keyboard hook for the main thread.
  ///
  /// # Panics
  ///
  /// Panics when attempting to register multiple hooks.
  #[must_use]
  pub fn new<F>(dispatcher: Dispatcher, callback: F) -> crate::Result<Self>
  where
    F: FnMut(KeyEvent) -> bool + Send + 'static,
  {
    let handle = dispatcher.dispatch_sync(move || {
      HOOK
        .with(|state| {
          assert!(
            state.take().is_none(),
            "Only one keyboard hook can be registered on the main thread."
          );

          state.set(Some(Box::new(callback)));

          unsafe {
            SetWindowsHookExW(
              WH_KEYBOARD_LL,
              Some(Self::hook_proc),
              HINSTANCE(0),
              0,
            )
          }
          .map_err(|e| {
            crate::Error::Platform(format!(
              "SetWindowsHookExW failed: {e}"
            ))
          })
        })
        .map_err(|e| {
          crate::Error::Platform(format!(
            "Failed to create KeyboardHook: {e}"
          ))
        })
    })??;
    Ok(KeyboardHook { handle })
  }

  /// Hook procedure for keyboard events.
  ///
  /// For use with `SetWindowsHookExW`.
  extern "system" fn hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
  ) -> LRESULT {
    // If the code is less than zero, the hook procedure must pass the hook
    // notification directly to other applications.
    if code != 0 {
      return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // Get struct with the keyboard input event.
    let input = unsafe { *(lparam.0 as *const KBDLLHOOKSTRUCT) };

    let key_code = KeyCode(input.vkCode as u16);
    let is_keydown =
      wparam.0 as u32 == WM_KEYDOWN || wparam.0 as u32 == WM_SYSKEYDOWN;

    let key = match Key::try_from(key_code) {
      Ok(key) => key,
      Err(e) => {
        tracing::warn!("Unrecognized key code: {e} | Skipping key event");
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
      }
    };

    let key_event = KeyEvent::new(key, key_code, is_keydown);

    let should_intercept = HOOK.with(|state| {
      if let Some(mut callback) = state.take() {
        let result = callback(key_event);
        state.set(Some(callback));
        result
      } else {
        false
      }
    });

    if should_intercept {
      return LRESULT(1);
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
  }

  /// Stops the keyboard hook by unregistering it.
  pub fn stop(&mut self) -> crate::Result<()> {
    unsafe { UnhookWindowsHookEx(self.handle) }?;
    HOOK.with(|state| state.take());
    Ok(())
  }
}

impl Drop for KeyboardHook {
  fn drop(&mut self) {
    let _ = self.stop();
  }
}
