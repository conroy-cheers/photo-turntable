use crate::{camera::Camera, turntable::Turntable};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[derive(Debug, Clone)]
pub(super) enum WorkerState {
    Uninitialised,
    Connecting,
    Connected,
    ReturningToResetPosition,
    Stepping { step_count: u16, steps_total: u16 },
}

#[derive(Debug)]
pub(super) enum WorkerCommand {
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
pub(super) struct Worker<T: Turntable> {
    pub(super) cmd_rx: UnboundedReceiver<WorkerCommand>,
    pub(super) state_tx: UnboundedSender<WorkerState>,
    pub(super) table: Option<T>,
    pub(super) camera: Camera,
}

impl<T: Turntable> Worker<T> {
    pub(super) async fn run(mut self) {
        let mut state = WorkerState::Uninitialised;
        let _ = self.state_tx.send(state.clone());

        while let Some(cmd) = self.cmd_rx.recv().await {
            match cmd {
                WorkerCommand::Connect => {
                    state = WorkerState::Connecting;
                    let _ = self.state_tx.send(state.clone());
                    match T::connect().await {
                        Ok(mut tbl) => match tbl.configure().await {
                            Ok(_) => {
                                self.table = Some(tbl);
                                state = WorkerState::Connected;
                                let _ = self.state_tx.send(state.clone());
                            }
                            Err(e) => {
                                eprintln!("Configuration error: {:?}", e);
                                state = WorkerState::Uninitialised;
                                let _ = self.state_tx.send(state.clone());
                            }
                        },
                        Err(e) => {
                            eprintln!("Connect error: {:?}", e);
                            state = WorkerState::Uninitialised;
                            let _ = self.state_tx.send(state.clone());
                        }
                    }
                }
                WorkerCommand::Disconnect => {
                    if let Some(tbl) = self.table.as_mut() {
                        match tbl.disconnect().await {
                            Ok(_) => {}
                            Err(_) => {}
                        };
                        self.table = None;
                        state = WorkerState::Uninitialised;
                        let _ = self.state_tx.send(state.clone());
                    }
                }
                WorkerCommand::ResetPosition => {
                    if let Some(tbl) = self.table.as_mut() {
                        state = WorkerState::ReturningToResetPosition;
                        let _ = self.state_tx.send(state.clone());
                        tbl.reset_pos().await.unwrap();
                        state = WorkerState::Connected;
                        let _ = self.state_tx.send(state.clone());
                    }
                }
                WorkerCommand::Step {
                    rotation_steps,
                    tilt_lower,
                    tilt_upper,
                    tilt_steps,
                } => {
                    if let Some(tbl) = self.table.as_mut() {
                        let tilt_step_size =
                            T::compute_tilt_step(tilt_lower, tilt_upper, tilt_steps);
                        let total_steps = rotation_steps * tilt_steps;

                        let _ = self.state_tx.send(WorkerState::Stepping {
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
                                    state = WorkerState::Stepping {
                                        step_count: (i_tilt * rotation_steps) + i_rotate,
                                        steps_total: total_steps,
                                    };
                                    let _ = self.state_tx.send(state.clone());
                                    self.camera.capture().unwrap();
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

                        state = WorkerState::Connected;
                        let _ = self.state_tx.send(state.clone());
                    }
                }
            }
        }
    }
}
