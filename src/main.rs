//! masterblaster - A Rust-based tracker with a compiler-like architecture.
//! Uses winit + glutin + glow + imgui-rs for the GUI.

mod app;
mod ui;

use app::TrackerApp;
use std::num::NonZeroU32;

use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::DisplayBuilder;

use imgui_glow_renderer::AutoRenderer;
use imgui_winit_support::{HiDpiMode, WinitPlatform};

use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

fn main() {
    let event_loop = EventLoop::new().unwrap();

    let mut imgui = imgui::Context::create();
    imgui.set_ini_filename(None);
    imgui.style_mut().use_dark_colors();

    imgui.fonts().add_font(&[imgui::FontSource::DefaultFontData {
        config: Some(imgui::FontConfig {
            size_pixels: 14.0,
            ..Default::default()
        }),
    }]);

    let platform = WinitPlatform::new(&mut imgui);

    let mut state = AppState {
        gl: None,
        imgui,
        platform,
        renderer: None,
        app: TrackerApp::new(),
    };

    event_loop.run_app(&mut state).unwrap();
}

struct GlObjects {
    window: Window,
    surface: glutin::surface::Surface<WindowSurface>,
    context: glutin::context::PossiblyCurrentContext,
}

struct AppState {
    gl: Option<GlObjects>,
    imgui: imgui::Context,
    platform: WinitPlatform,
    renderer: Option<AutoRenderer>,
    app: TrackerApp,
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gl.is_some() {
            return;
        }

        let (window, gl_config) = create_gl_window(event_loop);

        self.platform
            .attach_window(self.imgui.io_mut(), &window, HiDpiMode::Default);

        let (surface, context) = create_gl_surface(&window, &gl_config);
        let glow_ctx = create_glow_context(&gl_config);
        let renderer = AutoRenderer::new(glow_ctx, &mut self.imgui)
            .expect("Failed to create imgui renderer");

        self.gl = Some(GlObjects {
            window,
            surface,
            context,
        });
        self.renderer = Some(renderer);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(gl) = &self.gl {
            let wrapped: winit::event::Event<()> = winit::event::Event::WindowEvent {
                window_id,
                event: event.clone(),
            };
            self.platform
                .handle_event(self.imgui.io_mut(), &gl.window, &wrapped);
        }

        match event {
            WindowEvent::CloseRequested => {
                self.app.stop_playback();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(gl) = &self.gl {
                    if size.width > 0 && size.height > 0 {
                        gl.surface.resize(
                            &gl.context,
                            NonZeroU32::new(size.width).unwrap(),
                            NonZeroU32::new(size.height).unwrap(),
                        );
                    }
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gl) = &self.gl {
            gl.window.request_redraw();
        }
    }
}

impl AppState {
    fn render(&mut self) {
        let Some(gl) = &self.gl else { return };
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        self.platform
            .prepare_frame(self.imgui.io_mut(), &gl.window)
            .expect("prepare_frame failed");

        let ui = self.imgui.new_frame();
        ui::build_ui(ui, &mut self.app);
        self.platform.prepare_render(ui, &gl.window);

        let draw_data = self.imgui.render();

        unsafe {
            let gl_ctx = renderer.gl_context();
            gl_ctx.clear_color(0.1, 0.1, 0.1, 1.0);
            gl_ctx.clear(glow::COLOR_BUFFER_BIT);
        }

        renderer
            .render(draw_data)
            .expect("imgui render failed");

        gl.surface
            .swap_buffers(&gl.context)
            .expect("swap_buffers failed");
    }
}

fn create_gl_window(event_loop: &ActiveEventLoop) -> (Window, glutin::config::Config) {
    let window_attrs = WindowAttributes::default()
        .with_inner_size(LogicalSize::new(1200.0_f32, 800.0))
        .with_title("masterblaster");

    let template = ConfigTemplateBuilder::new();
    let display_builder = DisplayBuilder::new().with_window_attributes(Some(window_attrs));

    let (window, gl_config) = display_builder
        .build(event_loop, template, |configs| {
            configs
                .reduce(|a, b| {
                    if a.num_samples() > b.num_samples() {
                        a
                    } else {
                        b
                    }
                })
                .unwrap()
        })
        .expect("Failed to create GL window");

    (window.expect("No window created"), gl_config)
}

fn create_gl_surface(
    window: &Window,
    gl_config: &glutin::config::Config,
) -> (
    glutin::surface::Surface<WindowSurface>,
    glutin::context::PossiblyCurrentContext,
) {
    let raw_handle = window
        .window_handle()
        .expect("Failed to get window handle")
        .as_raw();

    let gl_display = gl_config.display();

    let context_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(Some(raw_handle));

    let gl_context = unsafe {
        gl_display
            .create_context(gl_config, &context_attrs)
            .expect("Failed to create GL context")
    };

    let size = window.inner_size();
    let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw_handle,
        NonZeroU32::new(size.width.max(1)).unwrap(),
        NonZeroU32::new(size.height.max(1)).unwrap(),
    );

    let surface = unsafe {
        gl_display
            .create_window_surface(gl_config, &surface_attrs)
            .expect("Failed to create GL surface")
    };

    let context = gl_context
        .make_current(&surface)
        .expect("Failed to make GL context current");

    (surface, context)
}

fn create_glow_context(gl_config: &glutin::config::Config) -> glow::Context {
    let gl_display = gl_config.display();
    unsafe { glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s)) }
}
