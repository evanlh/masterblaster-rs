//! masterblaster - A Rust-based tracker with a compiler-like architecture.
//! Uses winit + glutin + glow + imgui-rs for the GUI.

use masterblaster::app::App;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut handler = AppHandler { app: None };
    event_loop.run_app(&mut handler).unwrap();
}

struct AppHandler {
    app: Option<App>,
}

impl ApplicationHandler for AppHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }
        self.app = Some(App::new(event_loop, 1200.0, 800.0));
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
            WindowEvent::Resized(size) => app.handle_resize(size),
            WindowEvent::RedrawRequested => app.render_frame(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = &self.app {
            app.window().request_redraw();
        }
    }
}
