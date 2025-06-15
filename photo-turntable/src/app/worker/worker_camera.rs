use std::{env, path::PathBuf};

use crate::camera::{Camera, CameraContext, CameraSpec};
use anyhow::Error;
use mime2ext::mime2ext;
use tokio::{
    fs::{self},
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use uuid::Uuid;

#[derive(Clone, PartialEq)]
pub(crate) enum CameraWorkerState {
    Disconnected,
    GettingCameraList,
    CamerasListed { cameras: Vec<CameraSpec> },
    CameraConnecting,
    CameraConnected,
    Capturing { seq: u32 },
}

#[derive(Debug, Clone)]
pub(crate) enum CameraWorkerCommand {
    ListCameras,
    ConnectToCamera { camera_spec: CameraSpec },
    CaptureImage { seq: u32 },
}

struct CameraWorkerStateData {
    /// Receiver for fetching commands
    cmd_rx: UnboundedReceiver<CameraWorkerCommand>,
    /// Sender for pushing state to the UI
    state_tx: UnboundedSender<CameraWorkerState>,
    /// Sender for pushing state to another worker
    worker_state_tx: UnboundedSender<CameraWorkerState>,
    state: CameraWorkerState,
}

impl CameraWorkerStateData {
    fn update(&mut self, new_state: CameraWorkerState) {
        self.state = new_state;
        let _ = self.state_tx.send(self.state.clone());
        let _ = self.worker_state_tx.send(self.state.clone());
    }
}

/// Tokio worker for managing cameras
pub(crate) struct CameraWorker {
    state: CameraWorkerStateData,
    camera_context: CameraContext,
    camera: Option<Camera>,
}

impl CameraWorker {
    pub(crate) fn new(
        cmd_rx: UnboundedReceiver<CameraWorkerCommand>,
        state_tx: UnboundedSender<CameraWorkerState>,
        worker_state_tx: UnboundedSender<CameraWorkerState>,
    ) -> Result<Self, Error> {
        Ok(Self {
            state: CameraWorkerStateData {
                cmd_rx,
                state_tx,
                worker_state_tx,
                state: CameraWorkerState::Disconnected,
            },
            camera_context: CameraContext::new()?,
            camera: None,
        })
    }

    fn generate_temp_image_path(&self) -> PathBuf {
        let filename = format!("image_{}", Uuid::new_v4());
        env::temp_dir().join(filename)
    }

    pub(crate) async fn run(mut self) {
        self.state.update(CameraWorkerState::Disconnected);
        while let Some(cmd) = self.state.cmd_rx.recv().await {
            eprintln!("Received command {:?}", cmd);
            match cmd {
                CameraWorkerCommand::ListCameras => {
                    self.state.update(CameraWorkerState::GettingCameraList);
                    match self.camera_context.list_cameras() {
                        Ok(camera_specs) => {
                            self.state.update(CameraWorkerState::CamerasListed {
                                cameras: camera_specs,
                            });
                        }
                        Err(e) => {
                            eprintln!("Error listing cameras: {:?}", e);
                            self.state.update(CameraWorkerState::Disconnected);
                        }
                    };
                }
                CameraWorkerCommand::ConnectToCamera { camera_spec } => {
                    self.state.update(CameraWorkerState::CameraConnecting);
                    match camera_spec.connect(&self.camera_context) {
                        Ok(camera) => {
                            self.camera = Some(camera);
                            self.state.update(CameraWorkerState::CameraConnected);
                        }
                        Err(e) => {
                            eprintln!("Error connecting to camera {}: {:?}", camera_spec.name(), e);
                            self.state.update(CameraWorkerState::Disconnected);
                        }
                    }
                }
                CameraWorkerCommand::CaptureImage { seq } => {
                    match (&self.state.state, &self.camera) {
                        (CameraWorkerState::CameraConnected, Some(camera)) => {
                            self.state.update(CameraWorkerState::Capturing { seq });
                            let image_path = self.generate_temp_image_path();
                            match camera.capture(&image_path) {
                                Ok(camera_file) => {
                                    match async || -> anyhow::Result<PathBuf> {
                                        // Rename output file with appropriate extension, if available
                                        let new_path = match mime2ext(camera_file.mime_type()) {
                                            Some(ext) => {
                                                let path_with_ext = image_path.with_extension(ext);
                                                fs::rename(image_path, &path_with_ext).await?;
                                                path_with_ext
                                            }
                                            None => image_path,
                                        };
                                        Ok(new_path)
                                    }()
                                    .await
                                    {
                                        Ok(path) => {
                                            eprintln!("Wrote image to {:?}", path);
                                            self.state.update(CameraWorkerState::CameraConnected);
                                        }
                                        Err(e) => {
                                            eprintln!("Failed to download image: {:?}", e);
                                            self.state.update(CameraWorkerState::CameraConnected);
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Failed to capture image from camera: {:?}", e);
                                }
                            }
                        }
                        _ => {
                            eprintln!("Requested image capture, but no camera is connected!");
                        }
                    }
                }
            }
        }
    }
}
