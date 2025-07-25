mod worker;

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use self::worker::{TurntableWorker, TurntableWorkerCommand, TurntableWorkerState};
use crate::app::worker::{
    CameraWorker, CameraWorkerCommand, CameraWorkerState, ExportJob, TurntableSteppingJob,
};
use crate::camera::CameraSpec;
use crate::turntable::Turntable;

use eframe::egui::load::SizedTexture;
use eframe::egui::{
    Color32, ColorImage, Context, ImageSource, Layout, Stroke, TextureHandle, Vec2,
};
use eframe::emath::Align;
use eframe::{egui, App, CreationContext, Frame};
use rfd::FileDialog;
use tokio::runtime::Runtime;
use tokio::sync::broadcast;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use anyhow::anyhow;

struct ImagePreview {
    seq: u32,
    path: PathBuf,
    thumb: Option<ColorImage>,
    texture: Option<TextureHandle>,
}

impl ImagePreview {
    /// Load and resize image, returning egui texture
    fn load(seq: u32, path: &Path) -> anyhow::Result<Self> {
        let jpeg_data = std::fs::read(path)?;

        // initialize a decompressor with the scaling factor
        let mut decompressor = turbojpeg::Decompressor::new()?;
        let scaling = turbojpeg::ScalingFactor::ONE_EIGHTH;
        decompressor.set_scaling_factor(scaling)?;

        // read the JPEG header and downscale the width and height
        let scaled_header = decompressor.read_header(&jpeg_data)?.scaled(scaling);

        // initialize the image (Image<Vec<u8>>)
        let mut image = turbojpeg::Image {
            pixels: vec![0; 4 * scaled_header.width * scaled_header.height],
            width: scaled_header.width,
            pitch: 4 * scaled_header.width, // size of one image row in memory
            height: scaled_header.height,
            format: turbojpeg::PixelFormat::RGBA,
        };

        // decompress the JPEG into the image
        // (we use as_deref_mut() to convert from &mut Image<Vec<u8>> into Image<&mut [u8]>)
        decompressor.decompress(&jpeg_data, image.as_deref_mut())?;

        let color_image =
            ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.pixels);
        Ok(Self {
            seq,
            path: path.to_path_buf(),
            thumb: Some(color_image),
            texture: None,
        })
    }

    fn load_texture<'a>(&mut self, ctx: &Context) -> anyhow::Result<()> {
        let texture_name = self
            .path
            .file_stem()
            .ok_or(anyhow!("Image path has no file stem"))?
            .to_str()
            .ok_or(anyhow!("Unable to convert image path to string"))?;
        let image = std::mem::replace(&mut self.thumb, None);
        let texture = ctx.load_texture(
            "preview_".to_string() + texture_name,
            image.unwrap(),
            egui::TextureOptions::default(),
        );
        self.texture = Some(texture);
        Ok(())
    }
}

/// UI state holding channels and current values
pub(crate) struct TurntableApp<T: Turntable> {
    worker_state: TurntableWorkerState,
    camera_state: CameraWorkerState,
    slider_steps: u16,
    tilt_slider_low_deg: i16,
    tilt_slider_high_deg: i16,
    tilt_steps: u16,
    selected_camera_spec: Option<CameraSpec>,
    camera_select_box_open: bool,
    images: Vec<ImagePreview>,
    export_path: Arc<Mutex<Option<PathBuf>>>,
    file_picker_request: bool,
    capture_delay_ms: u64,
    table_cmd_tx: UnboundedSender<TurntableWorkerCommand>,
    table_state_rx: UnboundedReceiver<TurntableWorkerState>,
    camera_cmd_tx: UnboundedSender<CameraWorkerCommand>,
    camera_state_rx: broadcast::Receiver<CameraWorkerState>,
    image_rx: UnboundedReceiver<ImagePreview>,
    export_job_tx: UnboundedSender<ExportJob>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Turntable> TurntableApp<T> {
    pub(crate) fn new(_cc: &CreationContext<'_>) -> Self {
        let (camera_cmd_tx, camera_cmd_rx) = mpsc::unbounded_channel();
        let (camera_state_tx, camera_state_rx_1) = broadcast::channel(100);
        let camera_state_rx_2 = camera_state_tx.subscribe();
        let (camera_imagepath_tx, camera_imagepath_rx) = mpsc::unbounded_channel();

        let (image_tx, image_rx) = mpsc::unbounded_channel();

        let (export_job_tx, export_job_rx) = mpsc::unbounded_channel();

        let (table_cmd_tx, table_cmd_rx) = mpsc::unbounded_channel();
        let (table_state_tx, table_state_rx) = mpsc::unbounded_channel();

        // Spawn Tokio runtime for camera worker
        std::thread::spawn(move || {
            let rt = Runtime::new().unwrap();
            let worker = CameraWorker::new(camera_cmd_rx, camera_state_tx, camera_imagepath_tx)
                .expect("Could not create camera worker!");
            rt.block_on(worker.run());
        });

        // Spawn Tokio runtime for turntable worker
        let camera_cmd_tx_for_tt = camera_cmd_tx.clone();
        std::thread::spawn(move || {
            let rt = Runtime::new().unwrap();
            let worker = TurntableWorker::<T>::new(
                table_cmd_rx,
                table_state_tx,
                camera_cmd_tx_for_tt,
                camera_state_rx_1,
            );
            rt.block_on(worker.run());
        });

        // Spawn image loader
        std::thread::spawn(move || {
            let rt = Runtime::new().unwrap();
            rt.block_on(worker::image_loader(camera_imagepath_rx, image_tx));
        });

        // Spawn image exporter
        std::thread::spawn(move || {
            let rt = Runtime::new().unwrap();
            rt.block_on(worker::image_exporter(export_job_rx));
        });

        Self {
            worker_state: TurntableWorkerState::Uninitialised,
            camera_state: CameraWorkerState::Disconnected,
            slider_steps: 24,
            tilt_slider_low_deg: 0,
            tilt_slider_high_deg: 10,
            tilt_steps: 1,
            selected_camera_spec: None,
            camera_select_box_open: false,
            images: Vec::new(),
            export_path: Arc::new(Mutex::new(None)),
            file_picker_request: false,
            capture_delay_ms: 500,
            table_cmd_tx,
            table_state_rx,
            camera_cmd_tx,
            camera_state_rx: camera_state_rx_2,
            image_rx,
            export_job_tx,
            _marker: std::marker::PhantomData,
        }
    }

    fn export_jobs(&self) -> Vec<ExportJob> {
        let output_directory = self.export_path.lock().unwrap();
        match output_directory.deref() {
            Some(output_directory) => self
                .images
                .iter()
                .map(|img| ExportJob {
                    image_path: img.path.clone(),
                    seq: img.seq,
                    output_directory: output_directory.clone(),
                })
                .collect(),
            None => Vec::new(),
        }
    }

    fn next_seq(&self) -> u32 {
        match self.images.iter().map(|img| img.seq).max() {
            Some(max) => max + 1,
            None => 0,
        }
    }
}

impl<T: Turntable> App for TurntableApp<T> {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Drain any new worker states
        while let Ok(state) = self.table_state_rx.try_recv() {
            self.worker_state = state;
        }
        while let Ok(state) = self.camera_state_rx.try_recv() {
            self.camera_state = state;
        }
        // Receive any new images from worker
        while let Ok(mut image) = self.image_rx.try_recv() {
            match image.load_texture(ctx) {
                Ok(_) => self.images.push(image),
                Err(_) => eprintln!("Error loading decoded image {:?}", image.path),
            }
            self.images.sort_by_key(|img| img.seq);
        }

        // Build UI
        egui::SidePanel::left("Turntable").show(ctx, |ui| {
            ui.with_layout(Layout::top_down_justified(Align::Center), |ui| {
                // Connect button
                ui.add_space(8.0);
                let (connect_btn, enabled, command) = match self.worker_state {
                    TurntableWorkerState::Uninitialised => (
                        egui::Button::new("Connect"),
                        true,
                        Some(TurntableWorkerCommand::Connect),
                    ),
                    TurntableWorkerState::Connecting => {
                        (egui::Button::new("Connecting..."), false, None)
                    }
                    _ => (
                        egui::Button::new("Disconnect"),
                        true,
                        Some(TurntableWorkerCommand::Disconnect),
                    ),
                };
                let connect_btn = connect_btn.min_size(egui::vec2(220.0, 36.0));
                if ui.add_enabled(enabled, connect_btn).clicked() && command.is_some() {
                    let _ = self.table_cmd_tx.send(command.unwrap());
                }

                // Progress indicator
                ui.add_space(12.0);
                let progress = match &self.worker_state {
                    TurntableWorkerState::Uninitialised => 1.0,
                    TurntableWorkerState::Connecting => 1.0,
                    TurntableWorkerState::Connected => 1.0,
                    TurntableWorkerState::ReturningToResetPosition => 1.0,
                    TurntableWorkerState::Stepping(stepping_state) => stepping_state.progress(),
                    TurntableWorkerState::Paused(stepping_state) => stepping_state.progress(),
                };

                let progress_bar = egui::ProgressBar::new(progress);
                ui.add(match self.worker_state {
                    TurntableWorkerState::Uninitialised => progress_bar.fill(Color32::DARK_GRAY),
                    TurntableWorkerState::Connecting => progress_bar,
                    TurntableWorkerState::Connected => progress_bar.fill(Color32::LIGHT_GREEN),
                    TurntableWorkerState::ReturningToResetPosition => progress_bar,
                    TurntableWorkerState::Stepping(_) => progress_bar.show_percentage(),
                    TurntableWorkerState::Paused(_) => {
                        progress_bar.show_percentage().text("Paused")
                    }
                });

                // Reset/step controls
                ui.add_space(12.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 40.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        let enable_moves = match &self.worker_state {
                            TurntableWorkerState::Connected
                            | TurntableWorkerState::Stepping(_)
                            | TurntableWorkerState::Paused(_) => true,
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
                                let _ = self
                                    .table_cmd_tx
                                    .send(TurntableWorkerCommand::ResetPosition);
                            }
                        });
                        ui.add_enabled_ui(enable_moves, |ui| match &self.worker_state {
                            TurntableWorkerState::Stepping(_) => {
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 40.0],
                                        egui::Button::new("Pause"),
                                    )
                                    .clicked()
                                {
                                    let _ = self
                                        .table_cmd_tx
                                        .send(TurntableWorkerCommand::PauseStepping);
                                }
                            }
                            TurntableWorkerState::Paused(_) => {
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 40.0],
                                        egui::Button::new("Resume"),
                                    )
                                    .clicked()
                                {
                                    let _ = self
                                        .table_cmd_tx
                                        .send(TurntableWorkerCommand::ResumeStepping);
                                }
                            }
                            _ => {
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 40.0],
                                        egui::Button::new("Capture"),
                                    )
                                    .clicked()
                                {
                                    let _ = self.table_cmd_tx.send(TurntableWorkerCommand::Step {
                                        job: TurntableSteppingJob {
                                            rotation_steps: self.slider_steps,
                                            tilt_lower: self.tilt_slider_low_deg as f32,
                                            tilt_upper: self.tilt_slider_high_deg as f32,
                                            tilt_steps: self.tilt_steps,
                                            capture_delay_ms: self.capture_delay_ms,
                                        },
                                    });
                                }
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
        egui::CentralPanel::default().show(ctx, |ui| {
            let selected_camera_name = match &self.selected_camera_spec {
                Some(camera_spec) => camera_spec.name(),
                _ => "Select a camera...".to_string(),
            };

            // Unique ID to track ComboBox open/close state
            let combo_id = "camera_combo";
            let previous_selected_camera_spec = self.selected_camera_spec.clone();
            let mut camera_select_box_open = false;
            egui::ComboBox::new(combo_id, "Camera")
                .selected_text(format!("{}", selected_camera_name))
                .show_ui(ui, |ui| {
                    // This closure is only run when the combo box is open.
                    camera_select_box_open = true;
                    if !self.camera_select_box_open {
                        // Previous frame did not have the camera select box open, i.e. it was just opened.
                        match &self.camera_state {
                            CameraWorkerState::GettingCameraList => {}
                            _ => {
                                let _ = self.camera_cmd_tx.send(CameraWorkerCommand::ListCameras);
                            }
                        }
                    }
                    match &self.camera_state {
                        CameraWorkerState::Disconnected
                        | CameraWorkerState::Ready
                        | CameraWorkerState::Failed => {}
                        CameraWorkerState::GettingCameraList
                        | CameraWorkerState::CameraConnecting
                        | CameraWorkerState::Capturing { seq: _ } => {
                            ui.spinner();
                        }
                        CameraWorkerState::CamerasListed { cameras } => {
                            for camera in cameras {
                                ui.selectable_value(
                                    &mut self.selected_camera_spec,
                                    Some(camera.clone()),
                                    camera.name(),
                                )
                                .clicked();
                            }
                            if cameras.is_empty() {
                                ui.label("No cameras found");
                            }
                        }
                    }
                    match &self.camera_state {
                        CameraWorkerState::Ready
                        | CameraWorkerState::Failed
                        | CameraWorkerState::CamerasListed { .. } => {
                            if ui.small_button("Disconnect").clicked() {
                                self.selected_camera_spec = None;
                                let _ = self.camera_cmd_tx.send(CameraWorkerCommand::Disconnect);
                            }
                        }
                        _ => {}
                    }
                });
            self.camera_select_box_open = camera_select_box_open;

            if self.selected_camera_spec != previous_selected_camera_spec
                && self.selected_camera_spec.is_some()
            {
                let _ = self
                    .camera_cmd_tx
                    .send(CameraWorkerCommand::ConnectToCamera {
                        camera_spec: self.selected_camera_spec.clone().unwrap(),
                    });
            }

            ui.horizontal(|ui| {
                ui.add(egui::Label::new("Delay between captures (ms):"));
                ui.style_mut().spacing.slider_width = ui.available_width() - 50.0;
                ui.add(egui::Slider::new(&mut self.capture_delay_ms, 0..=2000).show_value(true));
            });
            let ui_width = ui.available_width() - 18.0;
            ui.allocate_ui_with_layout(
                Vec2::new(ui_width, 40.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    let item_width = ui_width / 3.0;
                    let (capture_button, capture_button_enabled, capture_command) =
                        match &self.camera_state {
                            CameraWorkerState::Ready => (
                                egui::Button::new("Capture"),
                                true,
                                Some(CameraWorkerCommand::CaptureImage {
                                    seq: self.next_seq(),
                                    extra_delay_ms: 0,
                                }),
                            ),
                            _ => (egui::Button::new("Capture"), false, None),
                        };
                    ui.add_enabled_ui(capture_button_enabled, |ui| {
                        if ui.add_sized([item_width, 40.0], capture_button).clicked()
                            && capture_command.is_some()
                        {
                            let _ = self.camera_cmd_tx.send(capture_command.unwrap());
                        }
                    });

                    let export_clear_button_enable = !self.images.is_empty();
                    ui.add_enabled_ui(export_clear_button_enable, |ui| {
                        if ui
                            .add_sized([item_width, 40.0], egui::Button::new("Clear"))
                            .clicked()
                        {
                            self.images.clear();
                        }
                    });
                    ui.add_enabled_ui(export_clear_button_enable, |ui| {
                        if ui
                            .add_sized([item_width, 40.0], egui::Button::new("Export..."))
                            .clicked()
                        {
                            self.file_picker_request = true;
                        }
                    });
                },
            );

            // Launch the file picker outside the UI context
            if self.file_picker_request {
                self.file_picker_request = false;
                let export_path_handle = self.export_path.clone();
                std::thread::spawn(move || {
                    let mut dialog = FileDialog::new().set_title("Export Images");
                    match export_path_handle.lock() {
                        Ok(export) => match &export.deref() {
                            Some(path) => {
                                dialog = dialog.set_directory(path.clone());
                            }
                            None => {}
                        },
                        Err(e) => {
                            eprintln!("Couldn't lock read old export path for reading: {:?}", e);
                        }
                    }
                    if let Some(path) = dialog.pick_folder() {
                        println!("Export path selected: {:?}", path.display());
                        let mut export_path = export_path_handle.lock().unwrap();
                        export_path.replace(path);
                    }
                });
            }

            // Dispatch export jobs, if they exist
            let export_jobs = self.export_jobs();
            if !export_jobs.is_empty() {
                for job in &export_jobs {
                    eprintln!(
                        "Running export job for image {:?} -> {:?}",
                        job.image_path, job.seq
                    );
                    let _ = self.export_job_tx.send(job.clone());
                }
                let mut export_path = self.export_path.lock().unwrap();
                *export_path = None;
                eprintln!("Exported {} images", export_jobs.len());
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                const COLUMNS: usize = 4;
                const SPACING: f32 = 10.0;
                let img_width = ui.available_width() / (COLUMNS as f32) - SPACING;

                egui::Grid::new("image_grid")
                    .num_columns(COLUMNS)
                    .spacing([SPACING, SPACING])
                    .show(ui, |ui| {
                        for (i, image) in self.images.iter().enumerate() {
                            match &image.texture {
                                Some(texture) => {
                                    ui.add(
                                        egui::Image::new(ImageSource::Texture(
                                            SizedTexture::from_handle(&texture),
                                        ))
                                        .max_width(img_width),
                                    );
                                    if i > 0 && (i + 1) % 4 == 0 {
                                        ui.end_row();
                                    }
                                }
                                None => {}
                            };
                        }
                    })
            })
        });

        // keep repainting so progress animates
        ctx.request_repaint();
    }
}
