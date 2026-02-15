//! Headed GUI integration tests (custom harness, runs on main thread).
//!
//! Run with:
//!   cargo test --features test-harness --test gui_tests -- [filter]
//!
//! Screenshots saved to tests/output/ (gitignored).
//! Uses harness=false so EventLoop runs on macOS main thread.

use masterblaster::app::App;
use masterblaster::ui::input::EditorAction;

use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::window::WindowId;

// ---------------------------------------------------------------------------
// Test framework
// ---------------------------------------------------------------------------

struct TestHarness {
    event_loop: EventLoop<()>,
    handler: TestHandler,
}

struct TestHandler {
    app: Option<App>,
    frames_rendered: usize,
    max_frames: usize,
}

impl TestHarness {
    fn new() -> Self {
        Self {
            event_loop: EventLoop::new().unwrap(),
            handler: TestHandler {
                app: None,
                frames_rendered: 0,
                max_frames: 5,
            },
        }
    }

    fn boot(&mut self) {
        for _ in 0..100 {
            self.event_loop
                .pump_app_events(Some(Duration::from_millis(16)), &mut self.handler);
            if self.handler.app.is_some() && self.handler.frames_rendered >= 3 {
                return;
            }
        }
        panic!("App did not become ready after pumping event loop");
    }

    fn app(&self) -> &App {
        self.handler.app.as_ref().unwrap()
    }

    fn app_mut(&mut self) -> &mut App {
        self.handler.app.as_mut().unwrap()
    }

    fn load_mod(&mut self, path: &str) {
        let data = std::fs::read(path).expect("Failed to read MOD file");
        self.app_mut()
            .gui
            .controller
            .load_mod(&data)
            .expect("Failed to parse MOD");
    }

    fn reset_editor(&mut self) {
        self.app_mut().gui.editor = Default::default();
    }

    fn inject(&mut self, actions: &[EditorAction]) {
        self.app_mut().inject_actions(actions);
    }

    fn render(&mut self, count: usize) {
        let target = self.handler.frames_rendered + count;
        self.handler.max_frames = target + 10;
        for _ in 0..count * 20 {
            if let Some(app) = &self.handler.app {
                app.window().request_redraw();
            }
            self.event_loop
                .pump_app_events(Some(Duration::from_millis(16)), &mut self.handler);
            if self.handler.frames_rendered >= target {
                return;
            }
        }
    }

    fn screenshot(&self, path: &str) {
        self.app().screenshot(path);
    }
}

impl ApplicationHandler for TestHandler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }
        self.app = Some(App::new(event_loop, 800.0, 600.0));
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(app) = &mut self.app else { return };

        let wrapped = winit::event::Event::WindowEvent {
            window_id,
            event: event.clone(),
        };
        app.handle_event(&wrapped);

        if let WindowEvent::RedrawRequested = event {
            app.render_frame();
            self.frames_rendered += 1;
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(app) = &self.app {
            if self.frames_rendered < self.max_frames {
                app.window().request_redraw();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Test runner
// ---------------------------------------------------------------------------

type TestFn = fn(&mut TestHarness);

const TESTS: &[(&str, TestFn)] = &[
    ("app_boots_and_renders", test_app_boots_and_renders),
    ("load_mod_and_render_pattern", test_load_mod_and_render_pattern),
    ("scroll_tracks_cursor_to_bottom", test_scroll_tracks_cursor_to_bottom),
    ("cursor_wraps_around", test_cursor_wraps_around),
    ("page_down_navigation", test_page_down_navigation),
];

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();

    let mut h = TestHarness::new();
    h.boot();

    let mut passed = 0;
    let mut failed = 0;

    for (name, test_fn) in TESTS {
        if !filter.is_empty() && !name.contains(filter.as_str()) {
            continue;
        }
        eprint!("  {name} ... ");
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test_fn(&mut h))) {
            Ok(()) => {
                eprintln!("ok");
                passed += 1;
            }
            Err(e) => {
                let msg = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                eprintln!("FAILED: {msg}");
                failed += 1;
            }
        }
    }

    eprintln!("\n{passed} passed, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn test_app_boots_and_renders(h: &mut TestHarness) {
    let (pixels, w, height) = h.app().capture_pixels();
    assert!(w > 0 && height > 0, "Window has nonzero dimensions");
    assert_eq!(pixels.len(), (w * height * 4) as usize);
    assert!(pixels.iter().any(|&p| p != 0), "Framebuffer not all-black");
    h.screenshot("tests/output/boot.png");
}

fn test_load_mod_and_render_pattern(h: &mut TestHarness) {
    h.load_mod("tests/fixtures/mod/musiklinjen.mod");
    h.render(3);

    assert_eq!(h.app().gui.editor.cursor.row, 0);
    assert_eq!(h.app().gui.editor.cursor.channel, 0);
    h.screenshot("tests/output/pattern_loaded.png");
}

fn test_scroll_tracks_cursor_to_bottom(h: &mut TestHarness) {
    h.load_mod("tests/fixtures/mod/musiklinjen.mod");
    h.reset_editor();
    h.render(3);

    // Navigate cursor to row 63 (0x3F) — last row in 64-row pattern
    for _ in 0..63 {
        h.inject(&[EditorAction::MoveCursor {
            drow: 1,
            dchannel: 0,
            dcolumn: 0,
        }]);
    }
    h.render(5);

    let editor = &h.app().gui.editor;
    assert_eq!(editor.cursor.row, 63, "Cursor should be at row 63");

    h.screenshot("tests/output/scroll_bottom.png");

    // The cursor highlight (blue) must be VISIBLE in the bottom quarter of the
    // center panel. Search x: 20%-75% (center panel, skip left clip list + right
    // samples panel), y: 75%-100% (bottom quarter). If scroll is broken, the
    // cursor at row 0x3F is off-screen and no blue pixels will be found.
    // Check cursor screen Y is within the window (not clipped off-screen).
    // Use logical height (physical / scale_factor) since imgui coordinates are logical.
    let editor = &h.app().gui.editor;
    let scale = h.app().window().scale_factor() as f32;
    let logical_height = h.app().window().inner_size().height as f32 / scale;
    eprintln!(
        "    [debug] cursor_screen_y={:.0} logical_h={:.0} vis={:02X}-{:02X} scroll={:.0}/{:.0}",
        editor.debug_cursor_screen_y,
        logical_height,
        editor.debug_vis_start,
        editor.debug_vis_end,
        editor.debug_scroll_y,
        editor.debug_scroll_max_y,
    );
    assert!(
        editor.debug_cursor_screen_y >= 0.0 && editor.debug_cursor_screen_y < logical_height,
        "Cursor at row 0x3F screen_y={:.0} should be within logical window height {:.0} (scroll bug)",
        editor.debug_cursor_screen_y,
        logical_height,
    );
}

fn test_cursor_wraps_around(h: &mut TestHarness) {
    h.load_mod("tests/fixtures/mod/musiklinjen.mod");
    h.reset_editor();
    h.render(3);

    for _ in 0..64 {
        h.inject(&[EditorAction::MoveCursor {
            drow: 1,
            dchannel: 0,
            dcolumn: 0,
        }]);
    }
    h.render(3);

    assert_eq!(h.app().gui.editor.cursor.row, 0, "Cursor wraps to row 0");
    h.screenshot("tests/output/cursor_wrapped.png");
}

fn test_page_down_navigation(h: &mut TestHarness) {
    h.load_mod("tests/fixtures/mod/musiklinjen.mod");
    h.reset_editor();
    h.render(3);

    h.inject(&[EditorAction::PageDown]);
    h.render(3);

    assert_eq!(h.app().gui.editor.cursor.row, 16, "PageDown moves 16 rows");
    h.screenshot("tests/output/page_down.png");
}
