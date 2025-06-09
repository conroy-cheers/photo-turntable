//! Bluetooth LE transport for the Revopoint Dual Axis Turntable using btleplug.

use anyhow::anyhow;
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral};
use std::time::Duration;
use tokio::time::sleep;

use super::command::Command;

/// UUIDs for the Revopoint turntable BLE service and characteristic.
const TURN_SERVICE_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x0000ffe1_0000_1000_8000_00805f9b34fb);
const TURN_CHAR_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x0000ffe1_0000_1000_8000_00805f9b34fb);

/// Wrapper around a connected turntable peripheral.
#[derive(Debug)]
pub struct RevopointBLE {
    peripheral: Peripheral,
}

impl RevopointBLE {
    async fn find_turntable(adapter: &Adapter) -> Option<Peripheral> {
        for p in adapter.peripherals().await.unwrap() {
            if p.properties()
                .await
                .unwrap()
                .unwrap()
                .services
                .contains(&TURN_SERVICE_UUID)
            {
                return Some(p);
            }
        }
        None
    }

    /// Discover and connect to the first turntable found.
    pub async fn connect() -> Result<Self, anyhow::Error> {
        // Initialize the manager and get the first bluetooth adapter
        let manager = Manager::new().await?;
        let adapter = manager
            .adapters()
            .await?
            .into_iter()
            .nth(0)
            .ok_or(anyhow!("No Bluetooth adapters found"))?;

        // Start scanning
        adapter.start_scan(ScanFilter::default()).await?;
        sleep(Duration::from_secs(3)).await; // give some time to discover

        // Find peripheral advertising our service
        let turntable = Self::find_turntable(&adapter)
            .await
            .ok_or(anyhow!("No turntable found"))?;

        // Connect and discover services
        turntable.connect().await?;
        turntable.discover_services().await?;

        Ok(RevopointBLE {
            peripheral: turntable,
        })
    }

    /// Send a command to the turntable over BLE.
    pub async fn send_command(&self, cmd: &Command) -> Result<(), anyhow::Error> {
        // Locate the characteristic
        let chars = self.peripheral.characteristics();
        let write_char = chars
            .iter()
            .find(|c| c.uuid == TURN_CHAR_UUID)
            .ok_or(anyhow!("Characteristic not found"))?;

        let data = cmd.to_string();
        self.peripheral
            .write(&write_char, data.as_bytes(), WriteType::WithoutResponse)
            .await?;
        Ok(())
    }

    /// Disconnect from the peripheral.
    pub async fn disconnect(&self) -> Result<(), anyhow::Error> {
        self.peripheral.disconnect().await?;
        Ok(())
    }
}
