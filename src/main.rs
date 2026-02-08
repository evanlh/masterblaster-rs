//! masterblaster - A Rust-based tracker with a compiler-like architecture.

use eframe::egui;

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
        Box::new(|_cc| Ok(Box::new(TrackerApp::new()))),
    )
}

struct TrackerApp {
    song: mb_ir::Song,
}

impl TrackerApp {
    fn new() -> Self {
        Self {
            song: mb_ir::Song::with_channels("Untitled", 4),
        }
    }
}

impl eframe::App for TrackerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top panel: transport controls
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("▶ Play").clicked() {
                    // TODO: Start playback
                }
                if ui.button("⏹ Stop").clicked() {
                    // TODO: Stop playback
                }
                ui.separator();
                ui.label(format!("Song: {}", self.song.title));
                ui.separator();
                ui.label(format!(
                    "Tempo: {} BPM | Speed: {}",
                    self.song.initial_tempo, self.song.initial_speed
                ));
            });
        });

        // Left panel: pattern list
        egui::SidePanel::left("patterns")
            .default_width(150.0)
            .show(ctx, |ui| {
                ui.heading("Patterns");
                ui.separator();

                for (i, _pattern) in self.song.patterns.iter().enumerate() {
                    ui.selectable_label(false, format!("Pattern {:02X}", i));
                }

                if ui.button("+ New Pattern").clicked() {
                    let channels = self.song.channels.len() as u8;
                    self.song.add_pattern(mb_ir::Pattern::new(64, channels));
                }
            });

        // Right panel: instruments
        egui::SidePanel::right("instruments")
            .default_width(200.0)
            .show(ctx, |ui| {
                ui.heading("Instruments");
                ui.separator();

                for (i, inst) in self.song.instruments.iter().enumerate() {
                    ui.label(format!("{:02X}: {}", i + 1, inst.name));
                }
            });

        // Central panel: pattern editor
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Pattern Editor");
            ui.separator();

            if self.song.patterns.is_empty() {
                ui.label("No patterns. Click '+ New Pattern' to create one.");
            } else {
                self.render_pattern(ui, 0);
            }
        });
    }
}

impl TrackerApp {
    fn render_pattern(&self, ui: &mut egui::Ui, pattern_idx: usize) {
        let Some(pattern) = self.song.patterns.get(pattern_idx) else {
            return;
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Header row
            ui.horizontal(|ui| {
                ui.monospace("Row");
                for ch in 0..pattern.channels {
                    ui.monospace(format!("  Ch{}  ", ch));
                }
            });
            ui.separator();

            // Pattern rows
            for row in 0..pattern.rows.min(64) {
                ui.horizontal(|ui| {
                    // Row number
                    let row_color = if row % 16 == 0 {
                        egui::Color32::from_rgb(100, 100, 150)
                    } else if row % 4 == 0 {
                        egui::Color32::from_rgb(80, 80, 100)
                    } else {
                        egui::Color32::from_rgb(60, 60, 70)
                    };
                    ui.colored_label(row_color, format!("{:02X}", row));

                    // Cells
                    for ch in 0..pattern.channels {
                        let cell = pattern.cell(row, ch);
                        let cell_text = format_cell(cell);
                        ui.monospace(cell_text);
                    }
                });
            }
        });
    }
}

fn format_cell(cell: &mb_ir::Cell) -> String {
    let note = match cell.note {
        mb_ir::Note::None => "---".to_string(),
        mb_ir::Note::On(n) => {
            let names = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];
            let octave = n / 12;
            let semitone = (n % 12) as usize;
            format!("{}{}", names[semitone], octave)
        }
        mb_ir::Note::Off => "===".to_string(),
        mb_ir::Note::Fade => "^^^".to_string(),
    };

    let inst = if cell.instrument > 0 {
        format!("{:02X}", cell.instrument)
    } else {
        "..".to_string()
    };

    let effect = match cell.effect {
        mb_ir::Effect::None => "...".to_string(),
        _ => "???".to_string(), // TODO: Format effects properly
    };

    format!("{} {} {}", note, inst, effect)
}
