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
pub(crate) struct TurntableSteppingJob {
    pub(crate) rotation_steps: u16,
    pub(crate) tilt_lower: f32,
    pub(crate) tilt_upper: f32,
    pub(crate) tilt_steps: u16,
    pub(crate) capture_delay_ms: u64,
}

impl TurntableSteppingJob {
    fn tilt_step_size(&self) -> f32 {
        (self.tilt_upper - self.tilt_lower) / self.tilt_steps as f32
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TurntableSteppingState {
    job: TurntableSteppingJob,
    rotation_step: u16,
    tilt_step: u16,
}

impl TurntableSteppingState {
    pub(crate) fn overall_step(&self) -> u32 {
        (self.job.rotation_steps as u32 * self.tilt_step as u32) + self.rotation_step as u32
    }

    pub(crate) fn total_steps(&self) -> u32 {
        self.job.rotation_steps as u32 * self.job.tilt_steps as u32
    }

    pub(crate) fn progress(&self) -> f32 {
        (self.overall_step() + 1) as f32 / self.total_steps() as f32
    }

    fn done(&self) -> bool {
        (self.tilt_step >= self.job.tilt_steps - 1)
            && (self.rotation_step >= self.job.rotation_steps - 1)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum TurntableWorkerState {
    Uninitialised,
    Connecting,
    Connected,
    ReturningToResetPosition,
    Stepping(TurntableSteppingState),
    Paused(TurntableSteppingState),
}

#[derive(Debug)]
pub(crate) enum TurntableWorkerCommand {
    Connect,
    Disconnect,
    ResetPosition,
    Step { job: TurntableSteppingJob },
    ResumeStepping,
    PauseStepping,
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

    async fn zero_position(&mut self, tilt_lower: f32) -> anyhow::Result<()> {
        let tbl = self.table.as_mut().ok_or(anyhow!("Table not present!"))?;
        // Zero tilt
        tbl.reset_tilt().await?;
        // Tilt to lower tilt position
        tbl.step_tilt(tilt_lower as f32).await?;
        Ok(())
    }

    /// Trigger taking a photo, and wait until it either succeeds or fails.
    async fn sync_take_photo(&mut self, state: &TurntableSteppingState) -> anyhow::Result<()> {
        let seq = state.overall_step();
        // Drain any unhandled camera states (e.g. from manual captures)
        while let Ok(_) = self.camera_state_rx.try_recv() {}
        match self.camera_cmd_tx.send(CameraWorkerCommand::CaptureImage {
            seq,
            extra_delay_ms: state.job.capture_delay_ms,
        }) {
            Ok(_) => {
                // Wait for camera worker state to first go to Capturing, then exit it
                eprintln!("Waiting for camera Capturing...");
                while match self.camera_state_rx.recv().await {
                    Ok(CameraWorkerState::Capturing { seq: recvd_seq }) => {
                        // If recvd_seq is our requested seq, break.
                        recvd_seq != seq
                    }
                    _ => true, // keep looping
                } {}
                // Wait for camera worker state to exit Capturing
                eprintln!("Waiting for camera exit Capturing...");
                while match self.camera_state_rx.recv().await {
                    Ok(CameraWorkerState::Capturing { seq: _ }) => true, // keep looping
                    Ok(CameraWorkerState::Ready) => {
                        // all done!
                        return Ok(());
                    }
                    Ok(CameraWorkerState::Failed) => {
                        return Err(anyhow!("Camera capture failed"));
                    }
                    _ => false,
                } {}
                Err(anyhow!("Something funny happened"))
            }
            Err(_) => Err(anyhow!("Unable to send command to camera worker")),
        }
    }

    /// Step the turntable once. Always rotates, and will also tilt after each complete rotation.
    /// Returns the new state after the step has been completed.
    async fn step_once(
        &mut self,
        from_state: &TurntableSteppingState,
    ) -> anyhow::Result<TurntableSteppingState> {
        match self.table.as_mut() {
            Some(tbl) => {
                tbl.step_horizontal(from_state.job.rotation_steps).await?;
                let new_rotation_step = from_state.rotation_step + 1;
                let (rotation_step, tilt_step) =
                    match new_rotation_step % from_state.job.rotation_steps {
                        0 => {
                            // Time to also tilt step
                            tbl.step_tilt(from_state.job.tilt_step_size()).await?;
                            (0, from_state.tilt_step + 1)
                        }
                        step => (step, from_state.tilt_step),
                    };

                Ok(TurntableSteppingState {
                    job: from_state.job.clone(),
                    rotation_step,
                    tilt_step,
                })
            }
            None => Err(anyhow!("Unable to reference turntable")),
        }
    }

    /// Attempts to capture an image, then step the turntable.
    /// After either success or failure, reports the new stepping state.
    async fn capture_step(
        &mut self,
        from_state: &TurntableSteppingState,
    ) -> Result<TurntableWorkerState, (TurntableWorkerState, anyhow::Error)> {
        let next_state = match self.sync_take_photo(from_state).await {
            Ok(_) => match self.step_once(from_state).await {
                // Success. Report continued stepping with the new state after step
                Ok(new_state) => Ok(TurntableWorkerState::Stepping(new_state)),
                // Failed to step turntable. Report paused state
                Err(e) => Err((TurntableWorkerState::Paused(from_state.clone()), e)),
            },
            Err(e) => {
                // Failed to take photo. Report paused state
                Err((TurntableWorkerState::Paused(from_state.clone()), e))
            }
        };
        if let Ok(TurntableWorkerState::Stepping(_)) = &next_state {
            if from_state.done() {
                Ok(TurntableWorkerState::Connected)
            } else {
                next_state
            }
        } else {
            next_state
        }
    }

    /// Handle a worker command.
    /// Returns the new worker state after handling the command. May publish state updates while running.
    async fn handle_command(
        &mut self,
        state: &TurntableWorkerState,
        cmd: &TurntableWorkerCommand,
    ) -> TurntableWorkerState {
        match cmd {
            TurntableWorkerCommand::Connect => {
                let _ = self.state_tx.send(TurntableWorkerState::Connecting);
                match T::connect().await {
                    Ok(mut tbl) => match tbl.configure().await {
                        Ok(_) => {
                            self.table = Some(tbl);
                            TurntableWorkerState::Connected
                        }
                        Err(e) => {
                            eprintln!("Configuration error: {:?}", e);
                            TurntableWorkerState::Uninitialised
                        }
                    },
                    Err(e) => {
                        eprintln!("Connect error: {:?}", e);
                        TurntableWorkerState::Uninitialised
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
                }
                TurntableWorkerState::Uninitialised
            }
            TurntableWorkerCommand::ResetPosition => {
                if let Some(tbl) = self.table.as_mut() {
                    let _ = self
                        .state_tx
                        .send(TurntableWorkerState::ReturningToResetPosition);
                    tbl.reset_pos().await.unwrap();
                }
                TurntableWorkerState::Connected
            }
            TurntableWorkerCommand::Step { job } => {
                if let TurntableWorkerState::Connected = state {
                    // Take first step
                    match self
                        .capture_step(&TurntableSteppingState {
                            job: job.clone(),
                            rotation_step: 0,
                            tilt_step: 0,
                        })
                        .await
                    {
                        Ok(new_state) => new_state,
                        Err(_) => state.clone(),
                    }
                } else {
                    state.clone()
                }
            }
            TurntableWorkerCommand::ResumeStepping => {
                if let TurntableWorkerState::Paused(stepping_state) = state {
                    // Resume stepping from the saved state
                    match self.capture_step(&stepping_state).await {
                        Ok(new_state) => new_state,
                        Err(_) => state.clone(),
                    }
                } else {
                    state.clone()
                }
            }
            TurntableWorkerCommand::PauseStepping => {
                if let TurntableWorkerState::Stepping(ref stepping_state) = state {
                    TurntableWorkerState::Paused(stepping_state.clone())
                } else {
                    state.clone()
                }
            }
        }
    }

    pub(crate) async fn run(mut self) {
        let mut state = TurntableWorkerState::Uninitialised;
        let _ = self.state_tx.send(state.clone());

        while let Some(cmd) = &self.cmd_rx.recv().await {
            // Handle the command, updating state
            state = self.handle_command(&mut state, cmd).await;
            let _ = self.state_tx.send(state.clone());

            // Inner loop to handle long-running tasks (i.e. stepping)
            loop {
                // If stepping after handling command, keep looping, otherwise break.
                if match state {
                    TurntableWorkerState::Stepping(ref stepping_state) => {
                        eprintln!("Took step. State: {:?}", stepping_state);
                        false
                    }
                    _ => true,
                } {
                    eprintln!("Stepping done. Exiting multi step");

                    break;
                }
                // If we didn't break, check for a pause command before continuing.
                match &self.cmd_rx.try_recv() {
                    Ok(TurntableWorkerCommand::PauseStepping) => {
                        eprintln!("Received pause step command");
                        state = self
                            .handle_command(&state, &TurntableWorkerCommand::PauseStepping)
                            .await;
                        let _ = self.state_tx.send(state.clone());
                    }
                    _ => {
                        // Continue stepping.
                        if let TurntableWorkerState::Stepping(stepping_state) = state {
                            state = match self.capture_step(&stepping_state).await {
                                Ok(new_state) => new_state,
                                Err((new_state, _)) => new_state,
                            };
                            let _ = self.state_tx.send(state.clone());
                        }
                    }
                }
            }
        }
    }
}
