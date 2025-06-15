use crate::{
    app::worker::worker_camera::{CameraWorkerCommand, CameraWorkerState},
    turntable::Turntable,
};
use anyhow::anyhow;
use tokio::sync::{
    broadcast,
    mpsc::{UnboundedReceiver, UnboundedSender},
};

#[derive(Debug, Clone)]
pub(crate) enum TurntableWorkerState {
    Uninitialised,
    Connecting,
    Connected,
    ReturningToResetPosition,
    Stepping { step_count: u16, steps_total: u16 },
}

#[derive(Debug)]
pub(crate) enum TurntableWorkerCommand {
    Connect,
    Disconnect,
    ResetPosition,
    Step {
        rotation_steps: u16,
        tilt_lower: i16,
        tilt_upper: i16,
        tilt_steps: u16,
    },
}

/// Tokio worker for managing a Turntable instance
pub(crate) struct TurntableWorker<T: Turntable> {
    cmd_rx: UnboundedReceiver<TurntableWorkerCommand>,
    state_tx: UnboundedSender<TurntableWorkerState>,
    camera_cmd_tx: UnboundedSender<CameraWorkerCommand>,
    camera_state_rx: broadcast::Receiver<CameraWorkerState>,
    table: Option<T>,
}

impl<T: Turntable> TurntableWorker<T> {
    pub(crate) fn new(
        cmd_rx: UnboundedReceiver<TurntableWorkerCommand>,
        state_tx: UnboundedSender<TurntableWorkerState>,
        camera_cmd_tx: UnboundedSender<CameraWorkerCommand>,
        camera_state_rx: broadcast::Receiver<CameraWorkerState>,
    ) -> Self {
        Self {
            cmd_rx,
            state_tx,
            camera_cmd_tx,
            camera_state_rx,
            table: None,
        }
    }

    pub(crate) async fn run(mut self) {
        let mut state = TurntableWorkerState::Uninitialised;
        let _ = self.state_tx.send(state.clone());

        while let Some(cmd) = self.cmd_rx.recv().await {
            match cmd {
                TurntableWorkerCommand::Connect => {
                    state = TurntableWorkerState::Connecting;
                    let _ = self.state_tx.send(state.clone());
                    match T::connect().await {
                        Ok(mut tbl) => match tbl.configure().await {
                            Ok(_) => {
                                self.table = Some(tbl);
                                state = TurntableWorkerState::Connected;
                                let _ = self.state_tx.send(state.clone());
                            }
                            Err(e) => {
                                eprintln!("Configuration error: {:?}", e);
                                state = TurntableWorkerState::Uninitialised;
                                let _ = self.state_tx.send(state.clone());
                            }
                        },
                        Err(e) => {
                            eprintln!("Connect error: {:?}", e);
                            state = TurntableWorkerState::Uninitialised;
                            let _ = self.state_tx.send(state.clone());
                        }
                    }
                }
                TurntableWorkerCommand::Disconnect => {
                    if let Some(tbl) = self.table.as_mut() {
                        match tbl.disconnect().await {
                            Ok(_) => {}
                            Err(_) => {}
                        };
                        self.table = None;
                        state = TurntableWorkerState::Uninitialised;
                        let _ = self.state_tx.send(state.clone());
                    }
                }
                TurntableWorkerCommand::ResetPosition => {
                    if let Some(tbl) = self.table.as_mut() {
                        state = TurntableWorkerState::ReturningToResetPosition;
                        let _ = self.state_tx.send(state.clone());
                        tbl.reset_pos().await.unwrap();
                        state = TurntableWorkerState::Connected;
                        let _ = self.state_tx.send(state.clone());
                    }
                }
                TurntableWorkerCommand::Step {
                    rotation_steps,
                    tilt_lower,
                    tilt_upper,
                    tilt_steps,
                } => {
                    if let Some(tbl) = self.table.as_mut() {
                        let tilt_step_size =
                            T::compute_tilt_step(tilt_lower, tilt_upper, tilt_steps);
                        let total_steps = rotation_steps * tilt_steps;

                        let _ = self.state_tx.send(TurntableWorkerState::Stepping {
                            step_count: 0,
                            steps_total: total_steps,
                        });

                        match async || -> anyhow::Result<()> {
                            // Zero tilt
                            tbl.reset_tilt().await?;
                            // Tilt to lower tilt position
                            tbl.step_tilt(tilt_lower as f32).await?;
                            for i_tilt in 0..tilt_steps {
                                // Perform full rotation
                                for i_rotate in 1..=rotation_steps {
                                    state = TurntableWorkerState::Stepping {
                                        step_count: (i_tilt * rotation_steps) + i_rotate,
                                        steps_total: total_steps,
                                    };
                                    let _ = self.state_tx.send(state.clone());

                                    // Wait for the camera to be ready before we capture
                                    while match self.camera_state_rx.recv().await {
                                        Ok(CameraWorkerState::Ready) => false, // break
                                        _ => true,                             // keep looping
                                    } {}

                                    let seq = i_rotate as u32;
                                    match self
                                        .camera_cmd_tx
                                        .send(CameraWorkerCommand::CaptureImage { seq })
                                    {
                                        Ok(_) => {
                                            // Wait for camera worker state to first go to Capturing, then exit it
                                            while match self.camera_state_rx.recv().await {
                                                Ok(CameraWorkerState::Capturing {
                                                    seq: recvd_seq,
                                                }) => {
                                                    // If recvd_seq is our requested seq, break.
                                                    recvd_seq != seq
                                                }
                                                _ => true, // keep looping
                                            } {}
                                            // Wait for camera worker state to exit Capturing
                                            while match self.camera_state_rx.recv().await {
                                                Ok(CameraWorkerState::Capturing { seq: _ }) => true, // keep looping
                                                _ => false, // keep looping
                                            } {}
                                        }
                                        Err(_) => {
                                            return Err(anyhow!(
                                                "Unable to send command to camera worker"
                                            ));
                                        }
                                    };
                                    tbl.step_horizontal(rotation_steps).await?;
                                }
                                if i_tilt < (tilt_steps - 1) {
                                    // Step tilt to next tilt position
                                    tbl.step_tilt(tilt_step_size).await?;
                                }
                            }

                            // Zero tilt again
                            tbl.reset_tilt().await?;
                            Ok(())
                        }()
                        .await
                        {
                            Ok(_) => {}
                            Err(e) => {
                                eprintln!("Error when stepping turntable: {:?}", e);
                            }
                        }

                        state = TurntableWorkerState::Connected;
                        let _ = self.state_tx.send(state.clone());
                    }
                }
            }
        }
    }
}
