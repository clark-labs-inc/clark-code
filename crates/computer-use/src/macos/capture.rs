use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};
use screencapturekit::shareable_content::SCShareableContent;
use screencapturekit::stream::configuration::SCStreamConfiguration;
use screencapturekit::stream::content_filter::SCContentFilter;

use crate::{encode_rgba_png, ComputerUseError, Screenshot, WindowInfo};

pub fn capture_window(window: &WindowInfo) -> Result<Screenshot, ComputerUseError> {
    let content = SCShareableContent::create()
        .with_exclude_desktop_windows(true)
        .with_on_screen_windows_only(true)
        .get()
        .map_err(|error| {
            ComputerUseError::Os(format!("ScreenCaptureKit window listing failed: {error}"))
        })?;
    let capture_window = content
        .windows()
        .into_iter()
        .find(|candidate| {
            candidate.window_id() == window.target.window_id
                && candidate.owning_application().is_some_and(|application| {
                    application.process_id() == window.target.pid
                        && application.bundle_identifier() == window.target.bundle_id
                })
        })
        .ok_or_else(|| {
            ComputerUseError::WindowNotFound(format!(
                "ScreenCaptureKit did not return {}:{}",
                window.target.pid, window.target.window_id
            ))
        })?;
    let filter = SCContentFilter::create()
        .with_window(&capture_window)
        .build();
    let point_scale = f64::from(filter.point_pixel_scale().max(1.0));
    let width = (window.frame.width * point_scale).round().max(1.0);
    let height = (window.frame.height * point_scale).round().max(1.0);
    let configuration = SCStreamConfiguration::new()
        .with_width(width as u32)
        .with_height(height as u32)
        .with_scales_to_fit(true)
        .with_shows_cursor(false);
    let image = SCScreenshotManager::capture_image(&filter, &configuration).map_err(|error| {
        ComputerUseError::Os(format!("ScreenCaptureKit screenshot failed: {error}"))
    })?;
    let width = image.width() as u32;
    let height = image.height() as u32;
    let rgba = image
        .rgba_data()
        .map_err(|error| ComputerUseError::Os(format!("screenshot decode failed: {error}")))?;
    Ok(Screenshot {
        width,
        height,
        png: encode_rgba_png(width, height, &rgba)?,
    })
}
