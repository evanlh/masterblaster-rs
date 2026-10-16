//! masterblaster - A Rust-based tracker with a compiler-like architecture.
//! Uses winit + glutin + glow + imgui-rs for the GUI.

use masterblaster::app::App;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

/// Idle timeout — redraw at least this often when stopped (for cursor blink, etc.)
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Playback throttle — redraw the UI at most this often while playing. The
/// position modeline and playing-row highlight don't need vsync, and spinning
/// at vsync steals CPU from the audio thread (design 026 T1).
const PLAYBACK_POLL_INTERVAL: Duration = Duration::from_millis(33);

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut handler = AppHandler {
        app: None,
        needs_redraw: false,
    };
    event_loop.run_app(&mut handler).unwrap();
}

struct AppHandler {
    app: Option<App>,
    needs_redraw: bool,
}

impl AppHandler {
    fn is_playing(&self) -> bool {
        self.app.as_ref().is_some_and(|app| app.gui.controller.is_playing())
    }
}

impl ApplicationHandler for AppHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }
        self.app = Some(App::new(event_loop, 1200.0, 800.0));
        self.needs_redraw = true;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(app) = &mut self.app else { return };

        let wrapped = winit::event::Event::WindowEvent {
            window_id,
            event: event.clone(),
        };
        app.handle_event(&wrapped);

        match event {
            WindowEvent::CloseRequested => {
                app.gui.controller.stop();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                app.handle_resize(size);
                self.needs_redraw = true;
            }
            WindowEvent::RedrawRequested => app.render_frame(),
            // Any input or state change triggers a redraw
            WindowEvent::KeyboardInput { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::Focused(_)
            | WindowEvent::DroppedFile(_)
            | WindowEvent::ModifiersChanged(_) => {
                self.needs_redraw = true;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(app) = &self.app else { return };

        if self.needs_redraw {
            // Input or state change: redraw promptly.
            self.needs_redraw = false;
            app.window().request_redraw();
            event_loop.set_control_flow(ControlFlow::Poll);
        } else if self.is_playing() {
            // Playback: redraw at a throttled rate instead of spinning at vsync.
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + PLAYBACK_POLL_INTERVAL,
            ));
            app.window().request_redraw();
        } else {
            // Idle: sleep until input or the idle timeout (cursor blink).
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + IDLE_POLL_INTERVAL,
            ));
            app.window().request_redraw();
        }
    }
}
