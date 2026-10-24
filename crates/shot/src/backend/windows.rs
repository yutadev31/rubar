use std::{mem, ptr};

use windows_sys::Win32::{
    Foundation::{BOOL, LPARAM, RECT},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetDIBits,
        GetMonitorInfoW, HBITMAP, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW, ReleaseDC, SRCCOPY,
        SelectObject,
    },
    UI::WindowsAndMessaging::SetProcessDPIAware,
};

use crate::{
    backend::{Backend, BackendTarget, PositionedFrame, stitch_positioned},
    frame::Frame,
};

pub struct WindowsBackend {
    monitors: Vec<Monitor>,
    selected_monitor: Option<usize>,
}

#[derive(Clone)]
struct Monitor {
    name: String,
    rect: RECT,
}

impl Backend for WindowsBackend {
    fn initialize(target: BackendTarget) -> crate::Result<Self> {
        Self::initialize_with_monitors(match target {
            BackendTarget::Monitor(index) => Some(index),
            BackendTarget::All => None,
        })
    }

    fn capture_output(&mut self) -> crate::Result<Frame> {
        let index = self.selected_monitor.unwrap_or(0);
        let monitor = self
            .monitors
            .get(index)
            .cloned()
            .ok_or(crate::Error::NoMonitors)?;
        capture_monitor(&monitor.rect)
    }

    fn capture_all_outputs(&mut self) -> crate::Result<Frame> {
        let mut frames = Vec::with_capacity(self.monitors.len());
        for monitor in self.monitors.clone() {
            frames.push(PositionedFrame {
                frame: capture_monitor(&monitor.rect)?,
                x: monitor.rect.left,
                y: monitor.rect.top,
            });
        }
        stitch_positioned(&frames)
    }

    fn monitor_count(&self) -> usize {
        self.monitors.len()
    }

    fn monitor_names(&self) -> Vec<String> {
        self.monitors
            .iter()
            .map(|monitor| monitor.name.clone())
            .collect()
    }

    fn select_monitor(&mut self, monitor_index: usize) -> crate::Result<()> {
        if monitor_index >= self.monitors.len() {
            return Err(crate::Error::MonitorOutOfRange {
                index: monitor_index,
                count: self.monitors.len(),
            });
        }
        self.selected_monitor = Some(monitor_index);
        Ok(())
    }
}

impl WindowsBackend {
    fn initialize_with_monitors(monitor_index: Option<usize>) -> crate::Result<Self> {
        // Prevent Windows DPI virtualization from changing the coordinates returned
        // by GetMonitorInfoW relative to the pixels captured by GDI.
        unsafe {
            SetProcessDPIAware();
        }

        let mut monitors = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                ptr::null_mut(),
                ptr::null(),
                Some(enumerate_monitor),
                &mut monitors as *mut Vec<Monitor> as LPARAM,
            );
        }
        if monitors.is_empty() {
            return Err(crate::Error::NoMonitors);
        }
        if let Some(index) = monitor_index
            && index >= monitors.len()
        {
            return Err(crate::Error::MonitorOutOfRange {
                index,
                count: monitors.len(),
            });
        }

        Ok(Self {
            monitors,
            selected_monitor: monitor_index,
        })
    }
}

unsafe extern "system" fn enumerate_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    unsafe {
        let monitors = &mut *(data as *mut Vec<Monitor>);
        let mut info = MONITORINFOEXW {
            monitorInfo: MONITORINFO {
                cbSize: mem::size_of::<MONITORINFOEXW>() as u32,
                ..mem::zeroed()
            },
            szDevice: [0; 32],
        };
        if GetMonitorInfoW(monitor, &mut info.monitorInfo) == 0 {
            return 1;
        }

        let name = String::from_utf16_lossy(
            &info.szDevice[..info
                .szDevice
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(info.szDevice.len())],
        );
        monitors.push(Monitor {
            name,
            rect: info.monitorInfo.rcMonitor,
        });
        1
    }
}

fn capture_monitor(rect: &RECT) -> crate::Result<Frame> {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return Err(crate::Error::WindowsCapture(
            "monitor has no area".to_string(),
        ));
    }

    unsafe {
        let screen_dc = GetDC(ptr::null_mut());
        if screen_dc.is_null() {
            return Err(crate::Error::WindowsCapture("GetDC failed".to_string()));
        }
        let result = capture_with_dc(screen_dc, rect, width, height);
        ReleaseDC(ptr::null_mut(), screen_dc);
        result
    }
}

unsafe fn capture_with_dc(
    screen_dc: HDC,
    rect: &RECT,
    width: i32,
    height: i32,
) -> crate::Result<Frame> {
    unsafe {
        let memory_dc = CreateCompatibleDC(screen_dc);
        if memory_dc.is_null() {
            return Err(crate::Error::WindowsCapture(
                "CreateCompatibleDC failed".to_string(),
            ));
        }
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        if bitmap.is_null() {
            DeleteDC(memory_dc);
            return Err(crate::Error::WindowsCapture(
                "CreateCompatibleBitmap failed".to_string(),
            ));
        }
        let old_bitmap = SelectObject(memory_dc, bitmap as _);

        let result = if BitBlt(
            memory_dc, 0, 0, width, height, screen_dc, rect.left, rect.top, SRCCOPY,
        ) == 0
        {
            Err(crate::Error::WindowsCapture("BitBlt failed".to_string()))
        } else {
            read_bitmap(memory_dc, bitmap, width, height)
        };

        SelectObject(memory_dc, old_bitmap);
        DeleteObject(bitmap as _);
        DeleteDC(memory_dc);
        result
    }
}

unsafe fn read_bitmap(dc: HDC, bitmap: HBITMAP, width: i32, height: i32) -> crate::Result<Frame> {
    unsafe {
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..mem::zeroed()
            },
            bmiColors: [mem::zeroed()],
        };
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(crate::Error::InvalidImageBuffer)?;
        let mut data = vec![0u8; size];
        if GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            data.as_mut_ptr() as *mut _,
            &mut info,
            DIB_RGB_COLORS,
        ) == 0
        {
            return Err(crate::Error::WindowsCapture("GetDIBits failed".to_string()));
        }
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Ok(Frame {
            data,
            width: width as u32,
            height: height as u32,
        })
    }
}
