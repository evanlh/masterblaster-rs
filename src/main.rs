//! masterblaster - A Rust-based tracker with a compiler-like architecture.

use eframe::egui;
use mb_audio::{AudioOutput, CpalOutput};
use mb_engine::{Engine, Frame};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("masterblaster"),
        ..Default::default()
    };

    eframe::run_native(
        "masterblaster",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            Ok(Box::new(TrackerApp::new()))
        }),
    )
}

// --- Playback state ---

struct PlaybackState {
    stop_signal: Arc<AtomicBool>,
    current_tick: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

// --- App state ---

#[derive(Clone, Copy, PartialEq, Eq)]
enum CenterView {
    Pattern,
    Graph,
}

struct TrackerApp {
    song: mb_ir::Song,
    selected_pattern: usize,
    status: String,
    playback: Option<PlaybackState>,
    center_view: CenterView,
}

impl TrackerApp {
    fn new() -> Self {
        Self {
            song: mb_ir::Song::with_channels("Untitled", 4),
            selected_pattern: 0,
            status: String::new(),
            playback: None,
            center_view: CenterView::Pattern,
        }
    }

    fn is_playing(&self) -> bool {
        self.playback
            .as_ref()
            .is_some_and(|p| !p.finished.load(Ordering::Relaxed))
    }

    fn current_tick(&self) -> Option<u64> {
        self.playback.as_ref().map(|p| p.current_tick.load(Ordering::Relaxed))
    }

    fn playback_position(&self) -> Option<mb_ir::PlaybackPosition> {
        let tick = self.current_tick()?;
        if !self.is_playing() {
            return None;
        }
        mb_ir::tick_to_position(&self.song, tick)
    }

    fn load_mod_file(&mut self) {
        let file = rfd::FileDialog::new()
            .add_filter("MOD files", &["mod", "MOD"])
            .pick_file();

        let Some(path) = file else { return };

        self.stop_playback();

        match std::fs::read(&path) {
            Err(e) => self.status = format!("Read error: {}", e),
            Ok(data) => match mb_formats::load_mod(&data) {
                Err(e) => self.status = format!("Parse error: {:?}", e),
                Ok(song) => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    self.status = format!("Loaded {}", name);
                    self.song = song;
                    self.selected_pattern = 0;
                }
            },
        }
    }

    fn start_playback(&mut self) {
        self.stop_playback();

        let song = self.song.clone();
        let stop_signal = Arc::new(AtomicBool::new(false));
        let current_tick = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));

        let stop = stop_signal.clone();
        let tick = current_tick.clone();
        let done = finished.clone();

        let thread = std::thread::spawn(move || {
            audio_thread(song, stop, tick, done);
        });

        self.playback = Some(PlaybackState {
            stop_signal,
            current_tick,
            finished,
            thread: Some(thread),
        });
        self.status = "Playing...".to_string();
    }

    fn stop_playback(&mut self) {
        if let Some(mut pb) = self.playback.take() {
            pb.stop_signal.store(true, Ordering::Relaxed);
            if let Some(handle) = pb.thread.take() {
                let _ = handle.join();
            }
            self.status = "Stopped".to_string();
        }
    }
}

fn audio_thread(
    song: mb_ir::Song,
    stop_signal: Arc<AtomicBool>,
    current_tick: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
) {
    let Ok((mut output, consumer)) = CpalOutput::new() else {
        finished.store(true, Ordering::Relaxed);
        return;
    };

    let sample_rate = output.sample_rate();
    let mut engine = Engine::new(song, sample_rate);
    engine.schedule_song();
    engine.play();

    if output.build_stream(consumer).is_err() {
        finished.store(true, Ordering::Relaxed);
        return;
    }
    let _ = output.start();

    // Render loop — update tick every ~10ms
    let tick_interval = (sample_rate / 100) as u64;
    let mut frame_count: u64 = 0;

    while !engine.is_finished() && !stop_signal.load(Ordering::Relaxed) {
        output.write_spin(engine.render_frame());
        frame_count += 1;
        if frame_count % tick_interval == 0 {
            current_tick.store(engine.position().tick, Ordering::Relaxed);
        }
    }

    // Drain silence to flush ring buffer
    for _ in 0..sample_rate {
        output.write_spin(Frame::silence());
    }

    finished.store(true, Ordering::Relaxed);
}

// --- Panels ---

fn transport_panel(app: &mut TrackerApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.button("Load").clicked() {
            app.load_mod_file();
        }
        ui.separator();

        let playing = app.is_playing();

        if ui.add_enabled(!playing, egui::Button::new("Play")).clicked() {
            app.start_playback();
        }
        if ui.add_enabled(playing, egui::Button::new("Stop")).clicked() {
            app.stop_playback();
        }
        ui.separator();

        let view_label = match app.center_view {
            CenterView::Pattern => "Graph",
            CenterView::Graph => "Pattern",
        };
        if ui.button(view_label).clicked() {
            app.center_view = match app.center_view {
                CenterView::Pattern => CenterView::Graph,
                CenterView::Graph => CenterView::Pattern,
            };
        }
        ui.separator();

        ui.label(&app.song.title.to_string());
        ui.separator();
        ui.label(format!(
            "BPM: {} | Speed: {}",
            app.song.initial_tempo, app.song.initial_speed
        ));

        if let Some(pos) = app.playback_position() {
            ui.separator();
            ui.monospace(format!(
                "Ord: {:02X} | Pat: {:02X} | Row: {:02X}",
                pos.order_index, pos.pattern_index, pos.row
            ));
        }

        if !app.status.is_empty() {
            ui.separator();
            ui.label(&app.status);
        }
    });

    // Keep repainting while playing so tick counter updates
    if app.is_playing() {
        ctx.request_repaint();
    }

    // Auto-detect when playback finishes naturally
    if let Some(ref pb) = app.playback {
        if pb.finished.load(Ordering::Relaxed) && app.status == "Playing..." {
            app.status = "Finished".to_string();
        }
    }
}

fn patterns_panel(
    app: &mut TrackerApp,
    pos: Option<mb_ir::PlaybackPosition>,
    ui: &mut egui::Ui,
) {
    ui.heading("Patterns");
    ui.separator();

    for (i, _) in app.song.patterns.iter().enumerate() {
        if ui
            .selectable_label(app.selected_pattern == i, format!("Pattern {:02X}", i))
            .clicked()
        {
            app.selected_pattern = i;
        }
    }

    ui.separator();
    ui.heading("Order");
    ui.separator();

    let playing_order = pos.map(|p| p.order_index);
    for (i, entry) in app.song.order.iter().enumerate() {
        let text = match entry {
            mb_ir::OrderEntry::Pattern(idx) => format!("{:02}: Pat {:02X}", i, idx),
            mb_ir::OrderEntry::Skip => format!("{:02}: +++", i),
            mb_ir::OrderEntry::End => format!("{:02}: ---", i),
        };
        let is_playing = playing_order == Some(i);
        let color = if is_playing {
            egui::Color32::from_rgb(100, 200, 130)
        } else {
            egui::Color32::from_rgb(180, 180, 180)
        };
        ui.label(egui::RichText::new(text).color(color));
    }
}

fn samples_panel(app: &TrackerApp, ui: &mut egui::Ui) {
    ui.heading("Samples");
    ui.separator();

    for (i, sample) in app.song.samples.iter().enumerate() {
        if sample.is_empty() {
            continue;
        }
        let loop_tag = if sample.has_loop() { " [L]" } else { "" };
        ui.label(format!(
            "{:02X}: {} ({}){loop_tag}",
            i + 1,
            sample.name,
            sample.len(),
        ));
    }
}

fn pattern_editor(
    app: &TrackerApp,
    pos: Option<mb_ir::PlaybackPosition>,
    ui: &mut egui::Ui,
) {
    let Some(pattern) = app.song.patterns.get(app.selected_pattern) else {
        ui.label("No patterns loaded.");
        return;
    };

    let playing_row = pos
        .filter(|p| p.pattern_index as usize == app.selected_pattern)
        .map(|p| p.row);

    ui.horizontal(|ui| {
        ui.heading(format!("Pattern {:02X}", app.selected_pattern));
        ui.label(format!(
            "({} rows, {} channels)",
            pattern.rows, pattern.channels
        ));
    });
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        // Header — match widths: row label is 2 chars, each cell is 10 chars
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("  ").monospace());
            for ch in 0..pattern.channels {
                ui.label(
                    egui::RichText::new(format!("  Chan {:02} ", ch)).monospace(),
                );
            }
        });
        ui.separator();

        // Rows
        for row in 0..pattern.rows {
            ui.horizontal(|ui| {
                let is_playing = playing_row == Some(row);
                let row_label_color = if is_playing {
                    egui::Color32::from_rgb(100, 200, 130)
                } else {
                    row_color(row)
                };
                ui.label(
                    egui::RichText::new(format!("{:02X}", row))
                        .monospace()
                        .color(row_label_color),
                );

                for ch in 0..pattern.channels {
                    let cell = pattern.cell(row, ch);
                    let text = format_cell(cell);
                    let color = if is_playing {
                        egui::Color32::from_rgb(100, 200, 130)
                    } else if cell.is_empty() {
                        egui::Color32::from_rgb(60, 60, 70)
                    } else {
                        egui::Color32::from_rgb(200, 200, 200)
                    };
                    ui.label(egui::RichText::new(text).monospace().color(color));
                }
            });
        }
    });
}

fn graph_panel(graph: &mb_ir::AudioGraph, ui: &mut egui::Ui) {
    let layers = compute_graph_layers(graph);
    if layers.is_empty() {
        ui.label("No graph nodes.");
        return;
    }

    let (response, painter) =
        ui.allocate_painter(ui.available_size(), egui::Sense::hover());
    let rect = response.rect;

    let node_w = 70.0_f32;
    let node_h = 28.0_f32;
    let num_layers = layers.len();
    let layer_spacing = rect.height() / num_layers.max(2) as f32;

    // Compute node centers: (node_id → Pos2)
    let mut node_centers: std::collections::HashMap<u16, egui::Pos2> =
        std::collections::HashMap::new();

    for (layer_idx, layer) in layers.iter().enumerate() {
        let y = rect.top() + layer_spacing * 0.5 + layer_idx as f32 * layer_spacing;
        let count = layer.len() as f32;
        let total_w = count * node_w + (count - 1.0).max(0.0) * 16.0;
        let x_start = rect.center().x - total_w / 2.0 + node_w / 2.0;

        for (i, &node_id) in layer.iter().enumerate() {
            let x = x_start + i as f32 * (node_w + 16.0);
            node_centers.insert(node_id, egui::pos2(x, y));
        }
    }

    // Draw connections first (behind nodes)
    let line_color = egui::Color32::from_rgb(80, 100, 80);
    for conn in &graph.connections {
        if let (Some(&from_pos), Some(&to_pos)) =
            (node_centers.get(&conn.from), node_centers.get(&conn.to))
        {
            let start = egui::pos2(from_pos.x, from_pos.y + node_h / 2.0);
            let end = egui::pos2(to_pos.x, to_pos.y - node_h / 2.0);
            painter.line_segment([start, end], egui::Stroke::new(1.5, line_color));
        }
    }

    // Draw nodes
    for node in &graph.nodes {
        let Some(&center) = node_centers.get(&node.id) else {
            continue;
        };
        let node_rect = egui::Rect::from_center_size(
            center,
            egui::vec2(node_w, node_h),
        );

        let (bg, border) = match &node.node_type {
            mb_ir::NodeType::Master => (
                egui::Color32::from_rgb(50, 50, 70),
                egui::Color32::from_rgb(120, 120, 180),
            ),
            _ => (
                egui::Color32::from_rgb(40, 55, 40),
                egui::Color32::from_rgb(90, 140, 90),
            ),
        };

        painter.rect_filled(node_rect, 4.0, bg);
        painter.rect_stroke(
            node_rect,
            4.0,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Outside,
        );
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            node.node_type.label(),
            egui::FontId::monospace(11.0),
            egui::Color32::from_rgb(200, 200, 200),
        );
    }
}

/// Assign nodes to layers by longest path from sources.
fn compute_graph_layers(graph: &mb_ir::AudioGraph) -> Vec<Vec<u16>> {
    let n = graph.nodes.len();
    if n == 0 {
        return Vec::new();
    }

    // Build in-degree for topo sort
    let mut in_degree = vec![0u32; n];
    for conn in &graph.connections {
        if (conn.to as usize) < n {
            in_degree[conn.to as usize] += 1;
        }
    }

    // Kahn's topo sort (inline to avoid engine dependency)
    let mut queue: Vec<u16> = (0..n as u16)
        .filter(|&id| in_degree[id as usize] == 0)
        .collect();
    let mut topo = Vec::with_capacity(n);
    while let Some(id) = queue.pop() {
        topo.push(id);
        for conn in &graph.connections {
            if conn.from == id && (conn.to as usize) < n {
                in_degree[conn.to as usize] -= 1;
                if in_degree[conn.to as usize] == 0 {
                    queue.push(conn.to);
                }
            }
        }
    }

    // Assign depth = longest path from any source
    let mut depth = vec![0usize; n];
    for &id in &topo {
        for conn in &graph.connections {
            if conn.from == id && (conn.to as usize) < n {
                depth[conn.to as usize] = depth[conn.to as usize].max(depth[id as usize] + 1);
            }
        }
    }

    // Group by depth
    let max_depth = depth.iter().copied().max().unwrap_or(0);
    let mut layers = vec![Vec::new(); max_depth + 1];
    for (id, &d) in depth.iter().enumerate() {
        layers[d].push(id as u16);
    }
    layers
}

fn row_color(row: u16) -> egui::Color32 {
    if row % 16 == 0 {
        egui::Color32::from_rgb(100, 100, 150)
    } else if row % 4 == 0 {
        egui::Color32::from_rgb(80, 80, 100)
    } else {
        egui::Color32::from_rgb(60, 60, 70)
    }
}

// --- eframe::App ---

impl eframe::App for TrackerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let pos = self.playback_position();

        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            transport_panel(self, ctx, ui);
        });

        egui::SidePanel::left("patterns")
            .default_width(150.0)
            .show(ctx, |ui| patterns_panel(self, pos, ui));

        egui::SidePanel::right("samples")
            .default_width(200.0)
            .show(ctx, |ui| samples_panel(self, ui));

        egui::CentralPanel::default().show(ctx, |ui| match self.center_view {
            CenterView::Pattern => pattern_editor(self, pos, ui),
            CenterView::Graph => graph_panel(&self.song.graph, ui),
        });
    }
}

// --- Cell formatting ---

fn format_cell(cell: &mb_ir::Cell) -> String {
    format!(
        "{} {} {}",
        format_note(cell.note),
        format_instrument(cell.instrument),
        format_effect(&cell.effect),
    )
}

fn format_note(note: mb_ir::Note) -> &'static str {
    match note {
        mb_ir::Note::None => "---",
        mb_ir::Note::Off => "===",
        mb_ir::Note::Fade => "^^^",
        mb_ir::Note::On(n) => note_name(n),
    }
}

fn note_name(n: u8) -> &'static str {
    const NAMES: [&str; 120] = [
        "C-0","C#0","D-0","D#0","E-0","F-0","F#0","G-0","G#0","A-0","A#0","B-0",
        "C-1","C#1","D-1","D#1","E-1","F-1","F#1","G-1","G#1","A-1","A#1","B-1",
        "C-2","C#2","D-2","D#2","E-2","F-2","F#2","G-2","G#2","A-2","A#2","B-2",
        "C-3","C#3","D-3","D#3","E-3","F-3","F#3","G-3","G#3","A-3","A#3","B-3",
        "C-4","C#4","D-4","D#4","E-4","F-4","F#4","G-4","G#4","A-4","A#4","B-4",
        "C-5","C#5","D-5","D#5","E-5","F-5","F#5","G-5","G#5","A-5","A#5","B-5",
        "C-6","C#6","D-6","D#6","E-6","F-6","F#6","G-6","G#6","A-6","A#6","B-6",
        "C-7","C#7","D-7","D#7","E-7","F-7","F#7","G-7","G#7","A-7","A#7","B-7",
        "C-8","C#8","D-8","D#8","E-8","F-8","F#8","G-8","G#8","A-8","A#8","B-8",
        "C-9","C#9","D-9","D#9","E-9","F-9","F#9","G-9","G#9","A-9","A#9","B-9",
    ];
    NAMES.get(n as usize).unwrap_or(&"???")
}

fn format_instrument(inst: u8) -> String {
    if inst > 0 {
        format!("{:02X}", inst)
    } else {
        "..".to_string()
    }
}

fn format_effect(effect: &mb_ir::Effect) -> String {
    use mb_ir::Effect::*;
    match effect {
        None => "...".to_string(),
        Arpeggio { x, y } => format!("0{:X}{:X}", x, y),
        PortaUp(v) => format!("1{:02X}", v),
        PortaDown(v) => format!("2{:02X}", v),
        TonePorta(v) => format!("3{:02X}", v),
        Vibrato { speed, depth } => format!("4{:X}{:X}", speed, depth),
        TonePortaVolSlide(v) => format!("5{}", vol_slide_param(*v)),
        VibratoVolSlide(v) => format!("6{}", vol_slide_param(*v)),
        Tremolo { speed, depth } => format!("7{:X}{:X}", speed, depth),
        SetPan(v) => format!("8{:02X}", v),
        SampleOffset(v) => format!("9{:02X}", v),
        VolumeSlide(v) => format!("A{}", vol_slide_param(*v)),
        PositionJump(v) => format!("B{:02X}", v),
        SetVolume(v) => format!("C{:02X}", v),
        PatternBreak(v) => format!("D{:02X}", v),
        // Extended effects (E-subcommands)
        FinePortaUp(v) => format!("E1{:X}", v),
        FinePortaDown(v) => format!("E2{:X}", v),
        SetVibratoWaveform(v) => format!("E4{:X}", v),
        SetFinetune(v) => format!("E5{:X}", *v as u8 & 0xF),
        PatternLoop(v) => format!("E6{:X}", v),
        SetTremoloWaveform(v) => format!("E7{:X}", v),
        SetPanPosition(v) => format!("E8{:X}", v),
        RetriggerNote(v) => format!("E9{:X}", v),
        FineVolumeSlideUp(v) => format!("EA{:X}", v),
        FineVolumeSlideDown(v) => format!("EB{:X}", v),
        NoteCut(v) => format!("EC{:X}", v),
        NoteDelay(v) => format!("ED{:X}", v),
        PatternDelay(v) => format!("EE{:X}", v),
        // Speed/Tempo
        SetSpeed(v) => format!("F{:02X}", v),
        SetTempo(v) => format!("F{:02X}", v),
        // Non-MOD effects: use name
        other => format!("{:.3}", other.name()),
    }
}

/// Format a signed volume slide value as XY hex nibbles.
fn vol_slide_param(v: i8) -> String {
    if v >= 0 {
        format!("{:X}0", v)
    } else {
        format!("0{:X}", -v)
    }
}
