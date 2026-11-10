use anyhow::{Context, Result};
use ssx_assets::display::{
    Backend, DisplayList, MAX_MODES, MAX_MONITORS, MonitorInfo, VideoMode, monitor_name_valid, scale_permille,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::raw_window_handle::{HandleError, HasDisplayHandle, RawDisplayHandle};
use winit::window::WindowId;

pub fn backend(handle: Result<winit::raw_window_handle::DisplayHandle<'_>, HandleError>) -> Backend {
    match handle.map(|h| h.as_raw()) {
        Ok(RawDisplayHandle::Windows(_)) => Backend::Windows,
        Ok(RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)) => Backend::X11,
        Ok(RawDisplayHandle::Wayland(_)) => Backend::Wayland,
        Ok(RawDisplayHandle::AppKit(_)) => Backend::MacOs,
        _ => Backend::Other,
    }
}

pub fn list_displays() -> Result<DisplayList> {
    struct Lister(Option<DisplayList>);
    impl ApplicationHandler for Lister {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.0.get_or_insert_with(|| collect(event_loop));
            event_loop.exit();
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            self.0.get_or_insert_with(|| collect(event_loop));
            event_loop.exit();
        }
    }
    let event_loop = EventLoop::new().context("the windowing system")?;
    let mut lister = Lister(None);
    event_loop.run_app(&mut lister).context("the windowing system's event loop")?;
    lister.0.context("the event loop ended before the monitors were read")
}

fn collect(event_loop: &ActiveEventLoop) -> DisplayList {
    let primary = event_loop.primary_monitor();
    let monitors = event_loop
        .available_monitors()
        .take(MAX_MONITORS)
        .map(|m| {
            let (size, position) = (m.size(), m.position());
            MonitorInfo {
                name: m.name().filter(|n| monitor_name_valid(n)),
                position: (position.x, position.y),
                size: (size.width, size.height),
                scale_permille: scale_permille(m.scale_factor()),
                refresh_mhz: m.refresh_rate_millihertz(),
                primary: primary.as_ref() == Some(&m),
                modes: m
                    .video_modes()
                    .take(MAX_MODES)
                    .map(|v| VideoMode {
                        width: v.size().width,
                        height: v.size().height,
                        refresh_mhz: v.refresh_rate_millihertz(),
                    })
                    .filter(VideoMode::valid)
                    .collect(),
            }
        })
        .map(|mut m: MonitorInfo| {
            m.modes.sort_by(|a, b| b.cmp(a));
            m.modes.dedup();
            m
        })
        .collect();
    DisplayList { backend: backend(event_loop.display_handle()), monitors }
}
