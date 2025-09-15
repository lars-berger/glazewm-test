use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;
use tracing::warn;
use windows::Win32::{
  Foundation::HWND,
  UI::{
    Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
    WindowsAndMessaging::{
      EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
      EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE,
      EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND,
      EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART,
      EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, OBJID_WINDOW,
      WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    },
  },
};

use crate::{platform_impl::NativeWindowInner, Result, WindowEvent};

/// Global instance of `WindowEventHook`.
///
/// For use with hook procedure.
static WIN_EVENT_HOOK: OnceLock<WindowHooks> = OnceLock::new();

struct WindowHooks {
  event_tx: mpsc::UnboundedSender<WindowEvent>,
  pub(crate) hook_handles: Vec<HWINEVENTHOOK>,
}

#[derive(Debug)]
pub struct WindowListener {
  event_rx: mpsc::UnboundedReceiver<WindowEvent>,
}

impl WindowListener {
  /// Creates an instance of `WindowEventHook`.
  pub fn new(dispatcher: &crate::Dispatcher) -> crate::Result<Self> {
    tracing::debug!("Creating WindowListener.");
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    dispatcher.dispatch_sync(|| Self::hook_win_events(event_tx))??;

    Ok(Self { event_rx })
  }

  /// Creates several window event hooks via `SetWinEventHook`.
  fn hook_win_events(
    event_tx: mpsc::UnboundedSender<WindowEvent>,
  ) -> crate::Result<()> {
    if WIN_EVENT_HOOK.get().is_some() {
      return Err(crate::Error::Platform(
        "Window event hook already running.".into(),
      ));
    }
    tracing::debug!("Setting window event hooks.");

    let event_ranges = [
      (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
      (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
      (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
      (EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZEEND),
      (EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZESTART),
      (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
      (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE),
      (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
    ];

    // Create separate hooks for each event range. This is more performant
    // than creating a single hook for all events and filtering them.
    let handles = event_ranges.iter().try_fold(
      Vec::new(),
      |mut handles, event_range| -> crate::Result<Vec<HWINEVENTHOOK>> {
        let hook_handle =
          Self::hook_win_event(event_range.0, event_range.1)?;
        handles.push(hook_handle);
        Ok(handles)
      },
    )?;

    let hooks = WindowHooks {
      event_tx,
      hook_handles: handles,
    };

    WIN_EVENT_HOOK.set(hooks).map_err(|hooks| {
      for handle in hooks.hook_handles {
        unsafe { UnhookWinEvent(handle) }.ok().ok();
      }
      crate::Error::Platform("Window event hook already running.".into())
    })?;

    Ok(())
  }

  /// Creates a window hook for the specified event range.
  fn hook_win_event(
    event_min: u32,
    event_max: u32,
  ) -> Result<HWINEVENTHOOK> {
    let hook_handle = unsafe {
      SetWinEventHook(
        event_min,
        event_max,
        None,
        Some(window_event_hook_proc),
        0,
        0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
      )
    };

    if hook_handle.is_invalid() {
      Err(crate::Error::Platform(
        "Failed to set window event hook, invalid hook.".into(),
      ))
    } else {
      Ok(hook_handle)
    }
  }

  /// Returns the next event from the `WindowListener`.
  pub async fn next_event(&mut self) -> Option<WindowEvent> {
    self.event_rx.recv().await
  }
}

impl WindowHooks {
  /// Invoked by the hook procedure when a window event is received.
  fn handle_event(&self, event_type: u32, handle: isize) {
    let window = NativeWindowInner::new(handle);
    let window = crate::NativeWindow::from(window);

    let platform_event = match event_type {
      EVENT_OBJECT_DESTROY => WindowEvent::Destroy(window.id()),
      EVENT_SYSTEM_FOREGROUND => WindowEvent::Focus(window),
      EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED => {
        WindowEvent::Hide(window)
      }
      EVENT_OBJECT_LOCATIONCHANGE => WindowEvent::LocationChange(window),
      EVENT_SYSTEM_MINIMIZESTART => WindowEvent::Minimize(window),
      EVENT_SYSTEM_MINIMIZEEND => WindowEvent::MinimizeEnd(window),
      EVENT_SYSTEM_MOVESIZEEND => WindowEvent::MoveOrResizeEnd(window),
      EVENT_SYSTEM_MOVESIZESTART => WindowEvent::MoveOrResizeStart(window),
      EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED => {
        WindowEvent::Show(window)
      }
      EVENT_OBJECT_NAMECHANGE => WindowEvent::TitleChange(window),
      _ => return,
    };

    if let Err(err) = self.event_tx.send(platform_event) {
      warn!("Failed to send platform event '{}'.", err);
    }
  }
}

/// Callback passed to `SetWinEventHook` to handle window events.
///
/// This function is called on selected window events, and forwards them
/// through an MPSC channel for the WM to process.
extern "system" fn window_event_hook_proc(
  _hook: HWINEVENTHOOK,
  event_type: u32,
  handle: HWND,
  id_object: i32,
  id_child: i32,
  _event_thread: u32,
  _event_time: u32,
) {
  let is_window_event =
    id_object == OBJID_WINDOW.0 && id_child == 0 && handle != HWND(0);

  // Check whether the event is associated with a window object instead
  // of a UI control.
  if !is_window_event {
    return;
  }

  if let Some(hook) = WIN_EVENT_HOOK.get() {
    hook.handle_event(event_type, handle.0);
  }
}
