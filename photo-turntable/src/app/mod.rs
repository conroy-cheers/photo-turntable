mod worker;

use self::worker::{Worker, WorkerCommand, WorkerState};
use crate::camera::Camera;
use crate::turntable::Turntable;

use eframe::egui::{Color32, Layout, Stroke, Vec2};
use eframe::emath::Align;
use eframe::{egui, App, CreationContext, Frame};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// UI state holding channels and current values
pub(crate) struct TurntableApp<T: Turntable> {
    worker_state: WorkerState,
    slider_steps: u16,
    tilt_slider_low_deg: i16,
    tilt_slider_high_deg: i16,
    tilt_steps: u16,
    cmd_tx: UnboundedSender<WorkerCommand>,
    state_rx: UnboundedReceiver<WorkerState>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Turntable> TurntableApp<T> {
    pub(crate) fn new(_cc: &CreationContext<'_>) -> Self {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (state_tx, state_rx) = unbounded_channel();

        // Spawn Tokio runtime on separate thread
        std::thread::spawn(move || {
            let rt = Runtime::new().unwrap();
            let worker = Worker::<T> {
                cmd_rx,
                state_tx,
                table: None,
                camera: Camera::new().expect("Couldn't create camera!"),
            };
            rt.block_on(worker.run());
        });

        Self {
            worker_state: WorkerState::Uninitialised,
            slider_steps: 24,
            tilt_slider_low_deg: 0,
            tilt_slider_high_deg: 10,
            tilt_steps: 1,
            cmd_tx,
            state_rx,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T: Turntable> App for TurntableApp<T> {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Drain any new worker states
        while let Ok(state) = self.state_rx.try_recv() {
            self.worker_state = state;
        }

        // Build UI
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(Layout::top_down_justified(Align::Center), |ui| {
                // Connect button
                ui.add_space(8.0);
                let (connect_btn, enabled, command) = match self.worker_state {
                    WorkerState::Uninitialised => (
                        egui::Button::new("Connect"),
                        true,
                        Some(WorkerCommand::Connect),
                    ),
                    WorkerState::Connecting => (egui::Button::new("Connecting..."), false, None),
                    _ => (
                        egui::Button::new("Disconnect"),
                        true,
                        Some(WorkerCommand::Disconnect),
                    ),
                };
                let connect_btn = connect_btn.min_size(egui::vec2(220.0, 36.0));
                if ui.add_enabled(enabled, connect_btn).clicked() && command.is_some() {
                    let _ = self.cmd_tx.send(command.unwrap());
                }

                // Progress indicator
                ui.add_space(12.0);
                let progress = match self.worker_state {
                    WorkerState::Uninitialised => 1.0,
                    WorkerState::Connecting => 1.0,
                    WorkerState::Connected => 1.0,
                    WorkerState::ReturningToResetPosition => 1.0,
                    WorkerState::Stepping {
                        step_count,
                        steps_total,
                    } => step_count as f32 / steps_total as f32,
                };

                let progress_bar = egui::ProgressBar::new(progress);
                ui.add(match self.worker_state {
                    WorkerState::Uninitialised => progress_bar.fill(Color32::DARK_GRAY),
                    WorkerState::Connecting => progress_bar,
                    WorkerState::Connected => progress_bar.fill(Color32::LIGHT_GREEN),
                    WorkerState::ReturningToResetPosition => progress_bar,
                    WorkerState::Stepping { .. } => progress_bar.show_percentage(),
                });

                // Reset/step controls
                ui.add_space(12.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 40.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        let enable_moves = match self.worker_state {
                            WorkerState::Connected => true,
                            _ => false,
                        };
                        ui.add_enabled_ui(enable_moves, |ui| {
                            if ui
                                .add_sized(
                                    [ui.available_width() / 2.0, 40.0],
                                    egui::Button::new("Reset Position"),
                                )
                                .clicked()
                            {
                                let _ = self.cmd_tx.send(WorkerCommand::ResetPosition);
                            }
                        });
                        ui.add_enabled_ui(enable_moves, |ui| {
                            if ui
                                .add_sized([ui.available_width(), 40.0], egui::Button::new("Step"))
                                .clicked()
                            {
                                let _ = self.cmd_tx.send(WorkerCommand::Step {
                                    rotation_steps: self.slider_steps,
                                    tilt_lower: self.tilt_slider_low_deg,
                                    tilt_upper: self.tilt_slider_high_deg,
                                    tilt_steps: self.tilt_steps,
                                });
                            }
                        });
                    },
                );

                // Step sliders
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.add(egui::Label::new("Rotation steps:"));
                    ui.horizontal(|ui| {
                        ui.style_mut().spacing.slider_width = ui.available_width() - 60.0;
                        ui.add(egui::Slider::new(&mut self.slider_steps, 1..=200).show_value(true));
                    });

                    ui.add_space(8.0);
                    ui.add(egui::Label::new(format!(
                        "Tilt Range: {:+} to {:+} deg in {} step{}",
                        self.tilt_slider_low_deg,
                        self.tilt_slider_high_deg,
                        self.tilt_steps,
                        if self.tilt_steps == 1 { "" } else { "s" }
                    )));
                    ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                        egui::ComboBox::from_label("Steps")
                            .selected_text(format!("{}", self.tilt_steps))
                            .show_ui(ui, |ui| {
                                for n in 1..=10 {
                                    ui.selectable_value(&mut self.tilt_steps, n, n.to_string());
                                }
                            });
                        ui.add(
                            egui_double_slider::DoubleSlider::new(
                                &mut self.tilt_slider_low_deg,
                                &mut self.tilt_slider_high_deg,
                                -30..=30,
                            )
                            .stroke(Stroke::new(7.0, ctx.style().visuals.selection.bg_fill))
                            .push_by_dragging(false)
                            .separation_distance(0)
                            .width(ui.available_width()),
                        );
                    });
                });

                // Debug status
                ui.add_space(12.0);
                ui.label(format!("State: {:?}", self.worker_state));
            });
        });

        // keep repainting so progress animates
        ctx.request_repaint();
    }
}
