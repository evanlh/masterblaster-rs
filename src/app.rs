//! Reusable App struct owning the GL/imgui stack.
//!
//! Used by both the real app (main.rs) and headed GUI tests.

use crate::ui::{self, GuiState};
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
use winit::dpi::LogicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

struct GlObjects {
    window: Window,
    surface: glutin::surface::Surface<WindowSurface>,
    context: glutin::context::PossiblyCurrentContext,
}

pub struct App {
    gl: GlObjects,
    imgui: imgui::Context,
    platform: WinitPlatform,
    renderer: AutoRenderer,
    pub gui: GuiState,
}

impl App {
    /// Create the app from an active event loop (call in `resumed()` or test setup).
    pub fn new(event_loop: &ActiveEventLoop, width: f32, height: f32) -> Self {
        let mut imgui = create_imgui_context();
        let platform = WinitPlatform::new(&mut imgui);
        let (window, gl_config) = create_gl_window(event_loop, width, height);

        let mut app = Self::init_gl(imgui, platform, window, &gl_config);
        app.platform
            .attach_window(app.imgui.io_mut(), &app.gl.window, HiDpiMode::Default);
        app
    }

    fn init_gl(
        mut imgui: imgui::Context,
        platform: WinitPlatform,
        window: Window,
        gl_config: &glutin::config::Config,
    ) -> Self {
        let (surface, context) = create_gl_surface(&window, gl_config);
        let glow_ctx = create_glow_context(gl_config);
        let renderer =
            AutoRenderer::new(glow_ctx, &mut imgui).expect("Failed to create imgui renderer");

        Self {
            gl: GlObjects {
                window,
                surface,
                context,
            },
            imgui,
            platform,
            renderer,
            gui: GuiState::default(),
        }
    }

    pub fn window(&self) -> &Window {
        &self.gl.window
    }

    /// Handle a winit event (forward to imgui platform).
    pub fn handle_event(&mut self, event: &winit::event::Event<()>) {
        self.platform
            .handle_event(self.imgui.io_mut(), &self.gl.window, event);
    }

    /// Handle window resize.
    pub fn handle_resize(&self, size: winit::dpi::PhysicalSize<u32>) {
        if size.width > 0 && size.height > 0 {
            self.gl.surface.resize(
                &self.gl.context,
                NonZeroU32::new(size.width).unwrap(),
                NonZeroU32::new(size.height).unwrap(),
            );
        }
    }

    /// Render one frame: build UI, render to GL, swap buffers.
    pub fn render_frame(&mut self) {
        self.platform
            .prepare_frame(self.imgui.io_mut(), &self.gl.window)
            .expect("prepare_frame failed");

        let ui = self.imgui.new_frame();
        ui::build_ui(ui, &mut self.gui);
        self.platform.prepare_render(ui, &self.gl.window);

        let draw_data = self.imgui.render();

        unsafe {
            let gl_ctx = self.renderer.gl_context();
            gl_ctx.clear_color(0.1, 0.1, 0.1, 1.0);
            gl_ctx.clear(glow::COLOR_BUFFER_BIT);
        }

        self.renderer
            .render(draw_data)
            .expect("imgui render failed");

        self.gl
            .surface
            .swap_buffers(&self.gl.context)
            .expect("swap_buffers failed");
    }

    /// Read the current framebuffer as RGBA pixels. Returns (data, width, height).
    pub fn capture_pixels(&self) -> (Vec<u8>, u32, u32) {
        let size = self.gl.window.inner_size();
        let (w, h) = (size.width, size.height);
        let mut pixels = vec![0u8; (w * h * 4) as usize];

        unsafe {
            let gl_ctx = self.renderer.gl_context();
            gl_ctx.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(&mut pixels),
            );
        }

        // glReadPixels returns bottom-up; flip vertically
        flip_rows_rgba(&mut pixels, w, h);
        (pixels, w, h)
    }

    /// Capture the framebuffer and save as a PNG file (test builds only).
    #[cfg(feature = "test-harness")]
    pub fn screenshot(&self, path: &str) {
        let (pixels, w, h) = self.capture_pixels();
        write_png(path, &pixels, w, h);
    }

    /// Inject editor actions programmatically (same code path as keyboard input).
    pub fn inject_actions(&mut self, actions: &[ui::input::EditorAction]) {
        ui::process_actions(&mut self.gui, actions);
    }
}

fn create_imgui_context() -> imgui::Context {
    let mut imgui = imgui::Context::create();
    imgui.set_ini_filename(None);
    imgui.style_mut().use_dark_colors();
    imgui.fonts().add_font(&[imgui::FontSource::DefaultFontData {
        config: Some(imgui::FontConfig {
            size_pixels: 14.0,
            ..Default::default()
        }),
    }]);
    imgui
}

fn create_gl_window(
    event_loop: &ActiveEventLoop,
    width: f32,
    height: f32,
) -> (Window, glutin::config::Config) {
    let window_attrs = WindowAttributes::default()
        .with_inner_size(LogicalSize::new(width, height))
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

/// Flip RGBA pixel buffer vertically (glReadPixels returns bottom-up).
fn flip_rows_rgba(pixels: &mut [u8], width: u32, height: u32) {
    let row_bytes = (width * 4) as usize;
    for y in 0..(height as usize / 2) {
        let top = y * row_bytes;
        let bot = (height as usize - 1 - y) * row_bytes;
        for x in 0..row_bytes {
            pixels.swap(top + x, bot + x);
        }
    }
}

/// Write RGBA pixel data as a PNG file (test builds only).
#[cfg(feature = "test-harness")]
fn write_png(path: &str, pixels: &[u8], width: u32, height: u32) {
    use std::fs::File;
    use std::io::BufWriter;

    std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap_or(std::path::Path::new(".")))
        .expect("Failed to create output directory");

    let file = File::create(path).expect("Failed to create PNG file");
    let w = BufWriter::new(file);

    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder.write_header().expect("Failed to write PNG header");
    writer.write_image_data(pixels).expect("Failed to write PNG data");
}
