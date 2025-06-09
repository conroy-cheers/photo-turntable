mod ble;
mod command;

use crate::turntable::command::Command;
use std::cmp::max;
use std::fmt::Debug;
use std::time::Duration;
use tokio::time::sleep;

const ROTATION_PACE: f32 = 35.64;
const TILT_PACE: f32 = 9.00;

#[derive(Debug)]
pub struct RevoTurntable {
    ble: ble::RevopointBLE,
}

pub trait Turntable: Sized + Send + Sync + 'static {
    async fn connect() -> Result<Self, anyhow::Error>;
    async fn disconnect(&mut self) -> Result<(), anyhow::Error>;
    async fn configure(&mut self) -> Result<(), anyhow::Error>;
    async fn reset_pos(&mut self) -> Result<(), anyhow::Error>;
    async fn reset_tilt(&mut self) -> Result<(), anyhow::Error>;
    async fn step_horizontal(&mut self, horizontal_steps: u16) -> Result<(), anyhow::Error>;
    async fn step_tilt(&mut self, tilt_step_deg: f32) -> Result<(), anyhow::Error>;
    fn compute_tilt_step(tilt_lower: i16, tilt_upper: i16, tilt_steps: u16) -> f32;
}

impl Turntable for RevoTurntable {
    async fn connect() -> Result<Self, anyhow::Error> {
        let ble = ble::RevopointBLE::connect().await?;
        Ok(Self { ble })
    }

    async fn disconnect(&mut self) -> Result<(), anyhow::Error> {
        self.ble.disconnect().await
    }

    async fn configure(&mut self) -> Result<(), anyhow::Error> {
        self.ble
            .send_command(&Command::SetRotationSpeed(ROTATION_PACE))
            .await?;
        self.ble
            .send_command(&Command::SetTiltSpeed(TILT_PACE))
            .await?;
        sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    async fn reset_pos(&mut self) -> Result<(), anyhow::Error> {
        self.ble.send_command(&Command::ZeroRotation).await?;
        self.ble.send_command(&Command::ZeroTilt).await?;
        let sleep_duration: u64 = max((ROTATION_PACE * 500.0) as u64, 3500);
        sleep(Duration::from_millis(sleep_duration)).await;
        Ok(())
    }

    async fn reset_tilt(&mut self) -> Result<(), anyhow::Error> {
        self.ble.send_command(&Command::ZeroTilt).await?;
        sleep(Duration::from_millis(3500)).await;
        Ok(())
    }

    async fn step_horizontal(&mut self, horizontal_steps: u16) -> Result<(), anyhow::Error> {
        self.ble
            .send_command(&Command::RotateBy(360.0 / (horizontal_steps as f32)))
            .await?;
        sleep(Duration::from_millis(
            (1000.0 * ROTATION_PACE / horizontal_steps as f32) as u64,
        ))
        .await;
        Ok(())
    }

    async fn step_tilt(&mut self, tilt_step_deg: f32) -> Result<(), anyhow::Error> {
        self.ble
            .send_command(&Command::TiltBy(tilt_step_deg))
            .await?;
        sleep(Duration::from_millis(
            (7000.0 / (60.0 / tilt_step_deg)) as u64,
        ))
        .await;
        Ok(())
    }

    fn compute_tilt_step(tilt_lower: i16, tilt_upper: i16, tilt_steps: u16) -> f32 {
        (tilt_upper - tilt_lower) as f32 / tilt_steps as f32
    }
}
