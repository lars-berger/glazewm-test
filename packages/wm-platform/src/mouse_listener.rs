use crate::Dispatcher;

/// A listener for system-wide mouse events.
pub struct MouseListener {
  _dispatcher: Dispatcher,
}

impl MouseListener {
  /// Creates a new mouse listener.
  pub fn new(dispatcher: &Dispatcher) -> crate::Result<Self> {
    // TODO: Implement platform-specific mouse listener setup
    Ok(Self {
      _dispatcher: dispatcher.clone(),
    })
  }

  /// Returns the next mouse event from the listener.
  ///
  /// This method will block until a mouse event is available.
  #[allow(clippy::unused_async)] //TODO: Remove when implemented.
  pub async fn next_event(
    &mut self,
  ) -> Option<crate::platform_event::MouseMoveEvent> {
    // TODO: Implement mouse event reception
    None
  }
}
