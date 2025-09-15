use std::time::Duration;

use tokio::task;
use tracing::warn;
use windows::{
  core::PWSTR,
  Win32::{
    Foundation::{CloseHandle, HWND, RECT},
    Graphics::Dwm::{
      DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_BORDER_COLOR,
      DWMWA_CLOAKED, DWMWA_COLOR_NONE, DWMWA_EXTENDED_FRAME_BOUNDS,
      DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT, DWMWCP_DONOTROUND,
      DWMWCP_ROUND, DWMWCP_ROUNDSMALL,
    },
    System::Threading::{
      OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
      PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::{
      Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
      },
      WindowsAndMessaging::{
        GetClassNameW, GetLayeredWindowAttributes, GetWindow,
        GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed,
        SendNotifyMessageW, SetForegroundWindow,
        SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPlacement,
        SetWindowPos, ShowWindowAsync, GWL_EXSTYLE, GWL_STYLE, GW_OWNER,
        HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST,
        LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, LWA_COLORKEY,
        SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSENDCHANGING,
        SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_MAXIMIZE,
        SW_MINIMIZE, SW_RESTORE, SW_SHOWNA, WINDOWPLACEMENT,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WPF_ASYNCWINDOWPLACEMENT,
        WS_CAPTION, WS_CHILD, WS_DLGFRAME, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_THICKFRAME,
      },
    },
  },
};

use super::COM_INIT;
use crate::{
  Color, Delta, Dispatcher, LengthValue, OpacityValue, PlatformWindow,
  Rect, RectDelta, ZOrder,
};

pub type RawWindowId = isize;

pub enum CornerStyle {
  Default,
  Square,
  Rounded,
  SmallRounded,
}

pub enum HideMethod {
  /// Hides the window using `SW_HIDE`. The window can be shown again with
  /// `SW_SHOWNA`.
  Hide,
  /// Cloaks the window using the `IApplicationView::SetCloak` method.
  ///
  /// Cloaked windows are hidden from the user but can still be shown in
  /// the taskbar and alt+tab menu. Cloaking is not supported by all
  /// applications.
  Cloak,
}

#[ambassador::delegatable_trait]
pub trait NativeWindowWindowsExt {
  /// Gets the process name associated with the window.
  fn process_name(&self) -> crate::Result<String>;
  /// Gets the class name of the window.
  fn class_name(&self) -> crate::Result<String>;
  /// Marks the window as fullscreen.
  ///
  /// Causes the native Windows taskbar to be moved to the bottom of the
  /// z-order when this window is active.
  fn mark_fullscreen(&self, fullscreen: bool) -> crate::Result<()>;
  fn set_border_color(
    &self,
    color: Option<&crate::Color>,
  ) -> crate::Result<()>;
  fn set_corner_style(
    &self,
    corner_style: &crate::platform_impl::CornerStyle,
  ) -> crate::Result<()>;
  fn set_title_bar_visibility(&self, visible: bool) -> crate::Result<()>;
  fn set_transparency(
    &self,
    opacity_value: &crate::OpacityValue,
  ) -> crate::Result<()>;
  fn adjust_transparency(
    &self,
    opacity_delta: &crate::Delta<crate::OpacityValue>,
  ) -> crate::Result<()>;
  fn set_z_order(&self, z_order: &crate::ZOrder) -> crate::Result<()>;
  /// Adds or removes the window from the native taskbar.
  ///
  /// Hidden windows (`SW_HIDE`) cannot be forced to be shown in the
  /// taskbar. Cloaked windows are normally always shown in the taskbar,
  /// but can be manually toggled.
  fn set_taskbar_visibility(&self, visible: bool) -> crate::Result<()>;

  fn show(&self) -> crate::Result<()>;
}

/// Magic number used to identify programmatic mouse inputs from our own
/// process.
pub const FOREGROUND_INPUT_IDENTIFIER: u32 = 6379;

#[derive(Clone, Debug)]
pub struct NativeWindowInner {
  pub handle: isize,
}

impl PlatformWindow for NativeWindowInner {
  fn id(&self) -> crate::WindowId {
    crate::WindowId(self.handle)
  }

  fn title(&self) -> crate::Result<String> {
    if !unsafe {
      windows::Win32::UI::WindowsAndMessaging::IsWindow(HWND(self.handle))
    }
    .as_bool()
    {
      return Err(crate::Error::WindowNotFound);
    }

    let mut text: [u16; 512] = [0; 512];
    let length = unsafe { GetWindowTextW(HWND(self.handle), &mut text) };

    #[allow(clippy::cast_sign_loss)]
    Ok(String::from_utf16_lossy(&text[..length as usize]))
  }

  fn is_visible(&self) -> crate::Result<bool> {
    let is_visible =
      unsafe { IsWindowVisible(HWND(self.handle)) }.as_bool();

    Ok(is_visible && !self.is_cloaked()?)
  }

  fn is_minimized(&self) -> crate::Result<bool> {
    Ok(unsafe { IsIconic(HWND(self.handle)) }.as_bool())
  }

  fn is_maximized(&self) -> crate::Result<bool> {
    Ok(unsafe { IsZoomed(HWND(self.handle)) }.as_bool())
  }

  fn frame(&self) -> crate::Result<Rect> {
    let mut rect = RECT::default();

    let dwm_res = unsafe {
      #[allow(clippy::cast_possible_truncation)]
      DwmGetWindowAttribute(
        HWND(self.handle),
        DWMWA_EXTENDED_FRAME_BOUNDS,
        std::ptr::from_mut(&mut rect).cast(),
        std::mem::size_of::<RECT>() as u32,
      )
    };

    if let Ok(()) = dwm_res {
      Ok(Rect::from_ltrb(
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
      ))
    } else {
      warn!("Failed to get window's frame position. Falling back to border position.");
      self.shadow_frame()
    }
  }

  fn set_frame(&self, rect: &Rect) -> crate::Result<()> {
    let swp_flags = SWP_NOACTIVATE
      | SWP_NOCOPYBITS
      | SWP_NOSENDCHANGING
      | SWP_ASYNCWINDOWPOS
      | SWP_NOZORDER;

    unsafe {
      SetWindowPos(
        HWND(self.handle),
        HWND(0),
        rect.left,
        rect.top,
        rect.width(),
        rect.height(),
        swp_flags,
      )
    }?;

    Ok(())
  }

  fn position(&self) -> crate::Result<(f64, f64)> {
    let frame = self.frame()?;
    Ok((frame.left as f64, frame.top as f64))
  }

  fn size(&self) -> crate::Result<(f64, f64)> {
    let frame = self.frame()?;
    Ok((frame.width() as f64, frame.height() as f64))
  }

  fn resize(&self, width: f64, height: f64) -> crate::Result<()> {
    let swp_flags = SWP_NOACTIVATE
      | SWP_NOCOPYBITS
      | SWP_NOSENDCHANGING
      | SWP_ASYNCWINDOWPOS
      | SWP_NOZORDER
      | SWP_NOMOVE;

    unsafe {
      SetWindowPos(
        HWND(self.handle),
        HWND(0),
        0,
        0,
        width as i32,
        height as i32,
        swp_flags,
      )
    }?;

    Ok(())
  }

  fn maximize(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(HWND(self.handle), SW_MAXIMIZE).ok() }?;
    Ok(())
  }

  fn minimize(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(HWND(self.handle), SW_MINIMIZE).ok() }?;
    Ok(())
  }

  fn close(&self) -> crate::Result<()> {
    unsafe {
      SendNotifyMessageW(HWND(self.handle), WM_CLOSE, None, None)
    }?;

    Ok(())
  }

  fn reposition(&self, x: f64, y: f64) -> crate::Result<()> {
    let swp_flags = SWP_NOACTIVATE
      | SWP_NOCOPYBITS
      | SWP_NOSENDCHANGING
      | SWP_ASYNCWINDOWPOS
      | SWP_NOSIZE
      | SWP_NOZORDER;

    unsafe {
      SetWindowPos(
        HWND(self.handle),
        HWND(0),
        x as i32,
        y as i32,
        0,
        0,
        swp_flags,
      )
    }?;

    //FIXME: Window needs to be positioned twice of there are pending DPI
    // adjustments.

    Ok(())
  }
}

impl NativeWindowWindowsExt for NativeWindowInner {
  fn process_name(&self) -> crate::Result<String> {
    let mut process_id = 0u32;
    unsafe {
      GetWindowThreadProcessId(
        HWND(self.handle),
        Some(&raw mut process_id),
      );
    }

    let process_handle = unsafe {
      OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
    }?;

    let mut buffer = [0u16; 256];
    let mut length = u32::try_from(buffer.len()).map_err(|_| {
      crate::Error::Platform("Failed to cast u32 to usize".into())
    })?;
    unsafe {
      QueryFullProcessImageNameW(
        process_handle,
        PROCESS_NAME_WIN32,
        PWSTR(buffer.as_mut_ptr()),
        &raw mut length,
      )?;

      CloseHandle(process_handle)?;
    };

    let exe_path = String::from_utf16_lossy(&buffer[..length as usize]);

    exe_path
      .split('\\')
      .next_back()
      .map(|file_name| {
        file_name.split('.').next().unwrap_or(file_name).to_string()
      })
      .ok_or(crate::Error::Platform(
        "Failed to parse process name.".into(),
      ))
  }

  fn class_name(&self) -> crate::Result<String> {
    let mut buffer = [0u16; 256];
    let result = unsafe { GetClassNameW(HWND(self.handle), &mut buffer) };

    if result == 0 {
      return Err(windows::core::Error::from_win32().into());
    }

    #[allow(clippy::cast_sign_loss)]
    let class_name = String::from_utf16_lossy(&buffer[..result as usize]);
    Ok(class_name)
  }

  fn mark_fullscreen(&self, fullscreen: bool) -> crate::Result<()> {
    COM_INIT.with(|com_init| -> crate::Result<()> {
      let taskbar_list = com_init.taskbar_list()?;

      unsafe {
        taskbar_list.MarkFullscreenWindow(HWND(self.handle), fullscreen)
      }?;

      Ok(())
    })
  }

  fn set_border_color(&self, color: Option<&Color>) -> crate::Result<()> {
    let bgr = match color {
      Some(color) => color.to_bgr(),
      None => DWMWA_COLOR_NONE,
    };

    unsafe {
      #[allow(clippy::cast_possible_truncation)]
      DwmSetWindowAttribute(
        HWND(self.handle),
        DWMWA_BORDER_COLOR,
        std::ptr::from_ref(&bgr).cast(),
        std::mem::size_of::<u32>() as u32,
      )?;
    }

    Ok(())
  }

  fn set_corner_style(
    &self,
    corner_style: &CornerStyle,
  ) -> crate::Result<()> {
    let corner_preference = match corner_style {
      CornerStyle::Default => DWMWCP_DEFAULT,
      CornerStyle::Square => DWMWCP_DONOTROUND,
      CornerStyle::Rounded => DWMWCP_ROUND,
      CornerStyle::SmallRounded => DWMWCP_ROUNDSMALL,
    };

    unsafe {
      #[allow(clippy::cast_possible_truncation)]
      DwmSetWindowAttribute(
        HWND(self.handle),
        DWMWA_WINDOW_CORNER_PREFERENCE,
        std::ptr::from_ref(&(corner_preference.0)).cast(),
        std::mem::size_of::<i32>() as u32,
      )?;
    }

    Ok(())
  }

  fn set_title_bar_visibility(&self, visible: bool) -> crate::Result<()> {
    let style = unsafe { GetWindowLongPtrW(HWND(self.handle), GWL_STYLE) };

    #[allow(clippy::cast_possible_wrap)]
    let new_style = if visible {
      style | (WS_DLGFRAME.0 as isize)
    } else {
      style & !(WS_DLGFRAME.0 as isize)
    };

    if new_style != style {
      unsafe {
        SetWindowLongPtrW(HWND(self.handle), GWL_STYLE, new_style);
        SetWindowPos(
          HWND(self.handle),
          HWND_NOTOPMOST,
          0,
          0,
          0,
          0,
          SWP_FRAMECHANGED
            | SWP_NOMOVE
            | SWP_NOSIZE
            | SWP_NOZORDER
            | SWP_NOOWNERZORDER
            | SWP_NOACTIVATE
            | SWP_NOCOPYBITS
            | SWP_NOSENDCHANGING
            | SWP_ASYNCWINDOWPOS,
        )?;
      }
    }

    Ok(())
  }

  fn set_transparency(
    &self,
    opacity_value: &OpacityValue,
  ) -> crate::Result<()> {
    // Make the window layered if it isn't already.
    self.add_window_style_ex(WS_EX_LAYERED);

    unsafe {
      SetLayeredWindowAttributes(
        HWND(self.handle),
        None,
        opacity_value.to_alpha(),
        LWA_ALPHA,
      )?;
    }

    Ok(())
  }

  fn adjust_transparency(
    &self,
    opacity_delta: &Delta<OpacityValue>,
  ) -> crate::Result<()> {
    let mut alpha = u8::MAX;
    let mut flag = LAYERED_WINDOW_ATTRIBUTES_FLAGS::default();

    unsafe {
      GetLayeredWindowAttributes(
        HWND(self.handle),
        None,
        Some(&raw mut alpha),
        Some(&raw mut flag),
      )?;
    }

    if flag.contains(LWA_COLORKEY) {
      return Err(crate::Error::Platform(
        "Window uses color key for its transparency and cannot be adjusted.".into())
      );
    }

    let target_alpha = if opacity_delta.is_negative {
      alpha.saturating_sub(opacity_delta.inner.to_alpha())
    } else {
      alpha.saturating_add(opacity_delta.inner.to_alpha())
    };

    self.set_transparency(&OpacityValue::from_alpha(target_alpha))
  }

  fn set_z_order(&self, z_order: &ZOrder) -> crate::Result<()> {
    let z_order = match z_order {
      ZOrder::TopMost => HWND_TOPMOST,
      ZOrder::Top => HWND_TOP,
      ZOrder::Normal => HWND_NOTOPMOST,
      ZOrder::AfterWindow(hwnd) => HWND(hwnd.0),
    };

    unsafe {
      SetWindowPos(
        HWND(self.handle),
        z_order,
        0,
        0,
        0,
        0,
        SWP_NOACTIVATE
          | SWP_NOCOPYBITS
          | SWP_ASYNCWINDOWPOS
          | SWP_SHOWWINDOW
          | SWP_NOMOVE
          | SWP_NOSIZE,
      )
    }?;

    let handle = self.handle;

    // Z-order can sometimes still be incorrect after the above call.
    task::spawn(async move {
      tokio::time::sleep(Duration::from_millis(10)).await;

      let _ = unsafe {
        SetWindowPos(
          HWND(handle),
          z_order,
          0,
          0,
          0,
          0,
          SWP_NOACTIVATE
            | SWP_NOCOPYBITS
            | SWP_ASYNCWINDOWPOS
            | SWP_SHOWWINDOW
            | SWP_NOMOVE
            | SWP_NOSIZE,
        )
      };
    });

    Ok(())
  }

  fn set_taskbar_visibility(&self, visible: bool) -> crate::Result<()> {
    COM_INIT.with(|com_init| -> crate::Result<()> {
      let taskbar_list = com_init.taskbar_list()?;

      if visible {
        unsafe { taskbar_list.AddTab(HWND(self.handle))? };
      } else {
        unsafe { taskbar_list.DeleteTab(HWND(self.handle))? };
      }

      Ok(())
    })
  }

  fn show(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(HWND(self.handle), SW_SHOWNA) }.ok()?;
    Ok(())
  }
}

impl NativeWindowInner {
  /// Creates a new `NativeWindow` instance with the given window handle.
  #[must_use]
  pub fn new(handle: isize) -> Self {
    Self { handle }
  }

  /// Whether the window is cloaked. For some UWP apps, `WS_VISIBLE` will
  /// be present even if the window isn't actually visible. The
  /// `DWMWA_CLOAKED` attribute is used to check whether these apps are
  /// visible.
  fn is_cloaked(&self) -> crate::Result<bool> {
    let mut cloaked = 0u32;

    unsafe {
      #[allow(clippy::cast_possible_truncation)]
      DwmGetWindowAttribute(
        HWND(self.handle),
        DWMWA_CLOAKED,
        std::ptr::from_mut::<u32>(&mut cloaked).cast(),
        std::mem::size_of::<u32>() as u32,
      )
    }?;

    Ok(cloaked != 0)
  }

  pub fn is_manageable(&self) -> crate::Result<bool> {
    // Ignore windows that are hidden.
    if !self.is_visible()? {
      return Ok(false);
    }

    // Ensure window has a valid process name, title, and class name.
    let process_name = self.process_name()?;
    let title = self.title()?;
    let _ = self.class_name()?;

    // TODO: Temporary fix for managing Flow Launcher until a force manage
    // command is added.
    if process_name == "Flow.Launcher" && title == "Flow.Launcher" {
      return Ok(true);
    }

    // Ensure window is top-level (i.e. not a child window). Ignore windows
    // that cannot be focused or if they're unavailable in task switcher
    // (alt+tab menu).
    let is_application_window = !self.has_window_style(WS_CHILD)
      && !self.has_window_style_ex(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW);

    if !is_application_window {
      return Ok(false);
    }

    // Ensure window position is accessible.
    _ = self.frame()?;

    // Some applications spawn top-level windows for menus that should be
    // ignored. This includes the autocomplete popup in Notepad++ and title
    // bar menu in Keepass. Although not foolproof, these can typically be
    // identified by having an owner window and no title bar.
    let is_menu_window =
      unsafe { GetWindow(HWND(self.handle), GW_OWNER) }.0 != 0
        && !self.has_window_style(WS_CAPTION);

    Ok(!is_menu_window)
  }

  /// Whether the window has resize handles.
  #[must_use]
  pub fn is_resizable(&self) -> bool {
    self.has_window_style(WS_THICKFRAME)
  }

  /// Whether the window is fullscreen.
  ///
  /// Returns `false` if the window is maximized.
  pub fn is_fullscreen(&self, monitor_rect: &Rect) -> crate::Result<bool> {
    if self.is_maximized()? {
      return Ok(false);
    }

    let position = self.frame()?;

    // Allow for 1px of leeway around edges of monitor.
    Ok(
      position.left <= monitor_rect.left + 1
        && position.top <= monitor_rect.top + 1
        && position.right >= monitor_rect.right - 1
        && position.bottom >= monitor_rect.bottom - 1,
    )
  }

  pub fn set_foreground(&self) -> crate::Result<()> {
    let input = [INPUT {
      r#type: INPUT_MOUSE,
      Anonymous: INPUT_0 {
        mi: MOUSEINPUT {
          dwExtraInfo: FOREGROUND_INPUT_IDENTIFIER as usize,
          ..Default::default()
        },
      },
    }];

    // Bypass restriction for setting the foreground window by sending an
    // input to our own process first.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    unsafe {
      SendInput(&input, std::mem::size_of::<INPUT>() as i32)
    };

    // Set as the foreground window.
    unsafe { SetForegroundWindow(HWND(self.handle)) }.ok()?;

    Ok(())
  }

  fn add_window_style_ex(&self, style: WINDOW_EX_STYLE) {
    let current_style =
      unsafe { GetWindowLongPtrW(HWND(self.handle), GWL_EXSTYLE) };

    #[allow(clippy::cast_possible_wrap)]
    if current_style & style.0 as isize == 0 {
      let new_style = current_style | style.0 as isize;

      unsafe {
        SetWindowLongPtrW(HWND(self.handle), GWL_EXSTYLE, new_style)
      };
    }
  }
  /// Gets the window's position, including the window's frame and
  /// shadow borders.
  pub fn shadow_frame(&self) -> crate::Result<Rect> {
    let mut rect = RECT::default();

    unsafe {
      GetWindowRect(
        HWND(self.handle),
        std::ptr::from_mut(&mut rect).cast(),
      )
    }?;

    Ok(Rect::from_ltrb(
      rect.left,
      rect.top,
      rect.right,
      rect.bottom,
    ))
  }

  /// Gets the delta between the window's frame and the window's border.
  /// This represents the size of a window's shadow borders.
  pub fn shadow_border_delta(&self) -> crate::Result<RectDelta> {
    let border_pos = self.shadow_frame()?;
    let frame_pos = self.frame()?;

    Ok(RectDelta::new(
      LengthValue::from_px(frame_pos.left - border_pos.left),
      LengthValue::from_px(frame_pos.top - border_pos.top),
      LengthValue::from_px(border_pos.right - frame_pos.right),
      LengthValue::from_px(border_pos.bottom - frame_pos.bottom),
    ))
  }

  fn has_window_style(&self, style: WINDOW_STYLE) -> bool {
    let current_style =
      unsafe { GetWindowLongPtrW(HWND(self.handle), GWL_STYLE) };

    #[allow(clippy::cast_possible_wrap)]
    let style = style.0 as isize;
    (current_style & style) != 0
  }

  fn has_window_style_ex(&self, style: WINDOW_EX_STYLE) -> bool {
    let current_style =
      unsafe { GetWindowLongPtrW(HWND(self.handle), GWL_EXSTYLE) };

    #[allow(clippy::cast_possible_wrap)]
    let style = style.0 as isize;
    (current_style & style) != 0
  }

  pub fn restore_to_position(&self, rect: &Rect) -> crate::Result<()> {
    let placement = WINDOWPLACEMENT {
      #[allow(clippy::cast_possible_truncation)]
      length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
      flags: WPF_ASYNCWINDOWPLACEMENT,
      showCmd: SW_RESTORE.0 as u32,
      rcNormalPosition: RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
      },
      ..Default::default()
    };

    unsafe {
      SetWindowPlacement(HWND(self.handle), &raw const placement)
    }?;

    Ok(())
  }

  pub fn set_visible(
    &self,
    visible: bool,
    hide_method: &HideMethod,
  ) -> crate::Result<()> {
    match hide_method {
      HideMethod::Hide => {
        if visible {
          self.show()
        } else {
          self.hide()
        }
      }
      HideMethod::Cloak => self.set_cloaked(!visible),
    }
  }

  pub fn hide(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(HWND(self.handle), SW_HIDE) }.ok()?;
    Ok(())
  }

  pub fn set_cloaked(&self, cloaked: bool) -> crate::Result<()> {
    COM_INIT.with(|com_init| -> crate::Result<()> {
      let view_collection = com_init.application_view_collection()?;

      let mut view = None;
      unsafe {
        view_collection.get_view_for_hwnd(self.handle, &raw mut view)
      }
      .ok()?;

      let view = view.ok_or(crate::Error::Platform(
        "Unable to get application view by window handle.".into(),
      ))?;

      // Ref: https://github.com/Ciantic/AltTabAccessor/issues/1#issuecomment-1426877843
      unsafe { view.set_cloak(1, if cloaked { 2 } else { 0 }) }
        .ok()
        .map_err(|e| {
          crate::Error::Platform(format!("Failed to cloak window: {e}"))
        })
    })
  }

  pub fn cleanup(&self) {
    if let Err(err) = self.show() {
      warn!("Failed to show window: {:?}", err);
    }

    _ = self.set_taskbar_visibility(true);
    _ = self.set_border_color(None);
    _ = self.set_transparency(&OpacityValue::from_alpha(u8::MAX));
  }
}

impl PartialEq for NativeWindowInner {
  fn eq(&self, other: &Self) -> bool {
    self.handle == other.handle
  }
}

impl Eq for NativeWindowInner {}

pub fn all_windows(
  _: &Dispatcher,
) -> crate::Result<Vec<crate::NativeWindow>> {
  __private_window::all_windows().map(|windows| {
    windows.into_iter().map(crate::NativeWindow::from).collect()
  })
}

pub fn visible_windows(
  _: &Dispatcher,
) -> crate::Result<Vec<crate::NativeWindow>> {
  let windows = __private_window::all_windows()?;

  let visible_windows = windows
    .into_iter()
    .filter(|window| match window.is_visible() {
      Ok(visible) => visible,
      Err(err) => {
        warn!(
          "Failed to check if window (handle={}) is visible: {}",
          window.handle, err
        );
        false
      }
    })
    .map(crate::NativeWindow::from)
    .collect();

  Ok(visible_windows)
}

pub(crate) mod __private_window {
  use windows::Win32::{
    Foundation::{BOOL, HWND, LPARAM},
    UI::WindowsAndMessaging::EnumWindows,
  };

  use crate::platform_impl::NativeWindowInner;

  pub fn all_windows(
  ) -> crate::Result<Vec<crate::platform_impl::NativeWindowInner>> {
    available_window_handles()?
      .into_iter()
      .map(|handle| Ok(NativeWindowInner::new(handle)))
      .collect()
  }

  pub fn available_window_handles() -> crate::Result<Vec<isize>> {
    let mut handles: Vec<isize> = Vec::new();

    unsafe {
      EnumWindows(
        Some(available_window_handles_proc),
        LPARAM(std::ptr::from_mut(&mut handles) as _),
      )
    }?;

    Ok(handles)
  }

  extern "system" fn available_window_handles_proc(
    handle: HWND,
    data: LPARAM,
  ) -> BOOL {
    let handles = data.0 as *mut Vec<isize>;
    unsafe { (*handles).push(handle.0) };
    true.into()
  }
}
