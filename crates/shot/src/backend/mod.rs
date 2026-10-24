use crate::frame::Frame;

#[cfg(all(
    not(any(target_os = "windows", target_os = "macos")),
    feature = "wayland"
))]
pub mod wayland;
#[cfg(all(target_os = "windows", feature = "windows"))]
pub mod windows;
#[cfg(all(not(any(target_os = "windows", target_os = "macos")), feature = "x11"))]
pub mod x11;

#[derive(Clone, Copy)]
pub enum BackendTarget {
    Monitor(usize),
    All,
}

#[derive(Debug)]
pub(crate) struct PositionedFrame {
    pub frame: Frame,
    pub x: i32,
    pub y: i32,
}

/// Compose backend captures in display coordinates, preserving gaps and
/// layouts that extend into negative coordinates.
pub(crate) fn stitch_positioned(frames: &[PositionedFrame]) -> crate::Result<Frame> {
    if frames.is_empty() {
        return Err(crate::Error::NoMonitors);
    }

    let min_x = frames.iter().map(|item| i64::from(item.x)).min().unwrap();
    let min_y = frames.iter().map(|item| i64::from(item.y)).min().unwrap();
    let max_x = frames
        .iter()
        .map(|item| i64::from(item.x) + i64::from(item.frame.width))
        .max()
        .ok_or(crate::Error::InvalidImageBuffer)?;
    let max_y = frames
        .iter()
        .map(|item| i64::from(item.y) + i64::from(item.frame.height))
        .max()
        .ok_or(crate::Error::InvalidImageBuffer)?;

    let width = u32::try_from(max_x - min_x).map_err(|_| crate::Error::InvalidImageBuffer)?;
    let height = u32::try_from(max_y - min_y).map_err(|_| crate::Error::InvalidImageBuffer)?;
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or(crate::Error::InvalidImageBuffer)?;
    let capacity = row_bytes
        .checked_mul(height as usize)
        .ok_or(crate::Error::InvalidImageBuffer)?;
    let mut data = vec![0; capacity];

    for item in frames {
        let frame = &item.frame;
        let source_row_bytes = (frame.width as usize)
            .checked_mul(4)
            .ok_or(crate::Error::InvalidImageBuffer)?;
        let expected_len = source_row_bytes
            .checked_mul(frame.height as usize)
            .ok_or(crate::Error::InvalidImageBuffer)?;
        if frame.data.len() != expected_len {
            return Err(crate::Error::InvalidImageBuffer);
        }

        let destination_x = usize::try_from(i64::from(item.x) - min_x)
            .map_err(|_| crate::Error::InvalidImageBuffer)?;
        let destination_y = usize::try_from(i64::from(item.y) - min_y)
            .map_err(|_| crate::Error::InvalidImageBuffer)?;
        for row in 0..frame.height as usize {
            let source_start = row * source_row_bytes;
            let destination_start = (destination_y + row) * row_bytes + destination_x * 4;
            data[destination_start..destination_start + source_row_bytes]
                .copy_from_slice(&frame.data[source_start..source_start + source_row_bytes]);
        }
    }

    Ok(Frame {
        data,
        width,
        height,
    })
}

pub trait Backend {
    fn initialize(target: BackendTarget) -> crate::Result<Self>
    where
        Self: Sized;
    fn capture_output(&mut self) -> crate::Result<Frame>;
    fn capture_all_outputs(&mut self) -> crate::Result<Frame>;
    fn monitor_count(&self) -> usize;
    fn monitor_names(&self) -> Vec<String>;
    fn select_monitor(&mut self, monitor_index: usize) -> crate::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::{PositionedFrame, stitch_positioned};
    use crate::frame::Frame;

    #[test]
    fn stitches_frames_at_positions() {
        let result = stitch_positioned(&[
            PositionedFrame {
                frame: Frame {
                    data: [1, 2, 3, 4].repeat(2),
                    width: 2,
                    height: 1,
                },
                x: -1,
                y: 1,
            },
            PositionedFrame {
                frame: Frame {
                    data: [5, 6, 7, 8].repeat(2),
                    width: 1,
                    height: 2,
                },
                x: 2,
                y: 0,
            },
        ])
        .expect("positioned frames should stitch");

        assert_eq!((result.width, result.height), (4, 2));
        assert_eq!(&result.data[16..24], &[1, 2, 3, 4, 1, 2, 3, 4]);
        assert_eq!(&result.data[8..12], &[0, 0, 0, 0]);
        assert_eq!(&result.data[12..16], &[5, 6, 7, 8]);
    }
}
