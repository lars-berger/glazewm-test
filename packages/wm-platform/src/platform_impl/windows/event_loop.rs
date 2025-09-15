use std::{
  sync::{atomic::AtomicBool, Arc, LazyLock},
  thread::{self, JoinHandle},
};

use windows::{
  core::w,
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{
      DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
      PostMessageW, PostThreadMessageW, RegisterWindowMessageW,
      TranslateMessage, MSG, WM_QUIT,
    },
  },
};

use crate::{DispatchFn, Dispatcher};

/// Custom message ID for dispatching closures to be run on the event
/// loop thread.
///
/// `WPARAM` contains a `Box<Box<dyn FnOnce()>>` that must be retrieved
/// with `Box::from_raw`. `LPARAM` is unused.
///
/// This message is sent using `PostMessageW` and handled in
/// [`EventLoop::window_proc`].
static WM_DISPATCH_CALLBACK: LazyLock<u32> = LazyLock::new(|| unsafe {
  RegisterWindowMessageW(w!("GlazeWM:Dispatch"))
});

#[derive(Clone)]
pub(crate) struct EventLoopSource {
  message_window_handle: crate::WindowId,
  thread_id: u32,
}

impl EventLoopSource {
  pub fn send_dispatch(
    &self,
    dispatch_fn: Box<DispatchFn>,
  ) -> crate::Result<()> {
    // Double box the callback to avoid `STATUS_ACCESS_VIOLATION` on
    // Windows. Ref Tao's implementation: https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/windows/event_loop.rs#L596
    let dispatch_fn = Box::new(dispatch_fn);

    // Leak to a raw pointer to then be passed as `WPARAM` in the message.
    let callback_ptr = Box::into_raw(dispatch_fn);

    unsafe {
      if PostMessageW(
        HWND(self.message_window_handle.0),
        *WM_DISPATCH_CALLBACK,
        WPARAM(callback_ptr as _),
        LPARAM(0),
      )
      .is_ok()
      {
        Ok(())
      } else {
        // If `PostMessage` fails, we need to clean up the callback.
        let _ = Box::from_raw(callback_ptr);
        Err(crate::Error::WindowMessage(
          "Failed to post message".to_string(),
        ))
      }
    }
  }

  pub fn send_stop(&self) -> crate::Result<()> {
    unsafe {
      PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0))
    }
    .map_err(|_| {
      crate::Error::WindowMessage(
        "Failed to post quit message".to_string(),
      )
    })
  }

  pub fn is_main_thread(&self) -> bool {
    let thread_id = unsafe { GetCurrentThreadId() };
    self.thread_id == thread_id
  }
}

/// Windows-specific implementation of [`EventLoop`].
pub(crate) struct EventLoop {
  message_window_handle: crate::WindowId,
  thread_handle: Option<JoinHandle<crate::Result<()>>>,
  thread_id: u32,
}

impl EventLoop {
  pub fn new() -> crate::Result<(Self, crate::Dispatcher)> {
    let (sender, receiver) =
      tokio::sync::oneshot::channel::<(crate::WindowId, u32)>();

    let thread_handle = thread::spawn(move || -> crate::Result<()> {
      // Create a hidden message window on the current thread.
      let window_handle =
        super::Platform::create_message_window(Some(Self::window_proc))?;

      let thread_id = unsafe { GetCurrentThreadId() };

      // Send the window handle and thread ID back to the main thread. Will
      // only fail if the receiver was closed, which would be due to the
      // main thread erroring - so just bail.
      if sender
        .send((crate::WindowId(window_handle), thread_id))
        .is_err()
      {
        unsafe { DestroyWindow(HWND(window_handle)) }?;
        return Err(crate::Error::Platform("Failed to send window handle back to main thread, channel was closed.".into()));
      }

      // Run the message loop. This will block until `WM_QUIT` is
      // dispatched.
      Self::run_message_loop();

      tracing::info!("Event loop thread exiting.");
      unsafe { DestroyWindow(HWND(window_handle)) }?;

      Ok(())
    });

    // Wait for the window handle and thread ID.
    let (window_handle, thread_id) =
      receiver.blocking_recv().map_err(|e| {
        crate::Error::Platform(
          "Event Loop failed to start and return thread id".into(),
        )
      })?;

    let event_loop = EventLoop {
      message_window_handle: window_handle,
      thread_handle: Some(thread_handle),
      thread_id,
    };

    let event_loop_source = EventLoopSource {
      message_window_handle: window_handle,
      thread_id,
    };

    let stopped = Arc::new(AtomicBool::new(false));
    let dispatcher = Dispatcher::new(Some(event_loop_source), stopped);

    Ok((event_loop, dispatcher))
  }

  /// Runs the event loop, blocking until shutdown.
  ///
  /// This method will block the current thread until the event loop is
  /// stopped.
  pub fn run(mut self) -> crate::Result<()> {
    tracing::info!("Starting Windows event loop.");

    // Join the thread to wait for completion
    if let Some(thread_handle) = self.thread_handle.take() {
      thread_handle
        .join()
        .map_err(|_| {
          crate::Error::Thread("Event loop thread panicked".to_string())
        })?
        .map_err(|e| crate::Error::Platform(e.to_string()))?;
    }

    tracing::info!("Windows event loop exiting.");
    Ok(())
  }

  /// Shuts down the event loop gracefully.
  pub fn shutdown(&mut self) -> crate::Result<()> {
    tracing::info!("Shutting down event loop.");

    // Wait for the spawned thread to finish.
    if let Some(thread_handle) = self.thread_handle.take() {
      crate::platform_impl::Platform::kill_message_loop(&thread_handle)?;

      thread_handle.join().map_err(|_| {
        crate::Error::Thread("Message thread failed to rejoin".into())
      })??;
    }

    Ok(())
  }

  /// Returns whether the event loop is still running.
  #[must_use]
  pub fn is_running(&self) -> bool {
    if let Some(ref handle) = self.thread_handle {
      !handle.is_finished()
    } else {
      false
    }
  }

  /// Returns the thread ID of the message loop thread.
  #[must_use]
  pub fn thread_id(&self) -> u32 {
    self.thread_id
  }

  /// Returns the window handle of the message loop.
  #[must_use]
  pub fn message_window_handle(&self) -> crate::WindowId {
    self.message_window_handle
  }

  /// Window procedure for handling messages.
  unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
  ) -> LRESULT {
    // TODO: Allow listeners to pre-process messages.
    if msg == *WM_DISPATCH_CALLBACK {
      // Convert the `WPARAM` fn pointer back to a double boxed function.
      let dispatch_fn: Box<Box<DispatchFn>> =
        Box::from_raw(wparam.0 as *mut _);
      dispatch_fn();
      LRESULT(0)
    }
    // `WM_QUIT` is handled for us by the message loop and should be
    // forwarded along with other messages we don't care about.
    else {
      DefWindowProcW(hwnd, msg, wparam, lparam)
    }
  }

  /// Starts a message loop on the current thread.
  ///
  /// This function will block until the message loop is killed. Use
  /// `Platform::kill_message_loop` to terminate the message loop.
  fn run_message_loop() {
    let mut msg = MSG::default();

    loop {
      if unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.as_bool() {
        unsafe {
          TranslateMessage(&raw const msg);
          DispatchMessageW(&raw const msg);
        }
      } else {
        break;
      }
    }
  }
}

impl Drop for EventLoop {
  fn drop(&mut self) {
    if let Err(err) = self.shutdown() {
      tracing::warn!("Failed to shut down event loop: {err}");
    }
  }
}
