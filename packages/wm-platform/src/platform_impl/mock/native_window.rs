pub use NativeWindowMock as NativeWindow;
pub type RawWindowId = usize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowState {
  Normal,
  Minimized,
  Maximized,
}

#[derive(Clone, Debug)]
pub struct NativeWindowMock {
  id: usize,
  title: String,
  frame: crate::Rect,
  hidden: bool,
  state: WindowState,
}

impl NativeWindowMock {
  pub fn id(&self) -> crate::WindowId {
    crate::WindowId(self.id)
  }

  pub fn title(&self) -> crate::Result<String> {
    Ok(self.title.clone())
  }

  pub fn frame(&self) -> crate::Result<crate::Rect> {
    Ok(self.frame.clone())
  }

  pub fn position(&self) -> crate::Result<(f64, f64)> {
    Ok((self.frame.x() as f64, self.frame.y() as f64))
  }

  pub fn size(&self) -> crate::Result<(f64, f64)> {
    Ok((self.frame.width() as f64, self.frame.height() as f64))
  }

  pub fn is_visible(&self) -> crate::Result<bool> {
    Ok(!self.hidden && self.state != WindowState::Minimized)
  }

  pub fn is_minimized(&self) -> crate::Result<bool> {
    Ok(self.state == WindowState::Minimized)
  }

  pub fn is_maximized(&self) -> crate::Result<bool> {
    Ok(matches!(self.state, WindowState::Maximized))
  }

  pub fn resize(&mut self, width: f64, height: f64) -> crate::Result<()> {
    let width = width as i32;
    let height = height as i32;

    let current_width = self.frame.width();
    let current_height = self.frame.height();

    let width_delta = width - current_width;
    let height_delta = height - current_height;

    self.frame.right += width_delta;
    self.frame.bottom += height_delta;

    Ok(())
  }

  pub fn reposition(&mut self, x: f64, y: f64) -> crate::Result<()> {
    let x = x as i32;
    let y = y as i32;

    self.frame.left = x;
    self.frame.top = y;

    Ok(())
  }

  pub fn set_frame(&mut self, rect: &crate::Rect) -> crate::Result<()> {
    self.frame = rect.clone();
    Ok(())
  }

  pub fn minimize(&mut self) -> crate::Result<()> {
    self.state = WindowState::Minimized;
    Ok(())
  }

  pub fn maximize(&mut self) -> crate::Result<()> {
    self.state = WindowState::Maximized;
    Ok(())
  }

  pub fn close(&self) -> crate::Result<()> {
    todo!()
  }
}
