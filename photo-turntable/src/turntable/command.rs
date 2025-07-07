/// Represents a command to send to the turntable.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum Command {
    /// Set rotation speed in units understood by device (e.g., 35.64 to 131)
    SetRotationSpeed(f32),
    /// Set tilt speed (e.g., 9 to 35)
    SetTiltSpeed(f32),

    /// Rotate by angle in degrees. Positive moves right, negative moves left.
    RotateBy(f32),
    /// Stop rotation immediately.
    StopRotation,
    /// Zero rotation angle (go to home).
    ZeroRotation,
    /// Continuous rotation: -1 for left infinite, 1 for right infinite.
    ContinuousRotation(i8),

    /// Tilt to position in degrees. Positive tilts right/up, negative tilts left/down.
    TiltTo(f32),
    /// Stop tilt immediately.
    StopTilt,
    /// Zero tilt value (go to neutral).
    ZeroTilt,

    /// Query current angle (returns +DATA=<angle>;)
    QueryAngle,

    /// Custom raw command string (must end with ';').
    Custom(String),
}

impl Command {
    /// Render the command as the ASCII string to send over BLE.
    pub fn to_string(&self) -> String {
        match self {
            Command::SetRotationSpeed(v) => format!("+CT,TURNSPEED={:.2};", v),
            Command::SetTiltSpeed(v) => format!("+CR,TILTSPEED={:.2};", v),
            Command::RotateBy(angle) => format!("+CT,TURNANGLE={:.2};", angle),
            Command::StopRotation => "+CT,STOP;".into(),
            Command::ZeroRotation => "+CT,TOZERO;".into(),
            Command::ContinuousRotation(dir) => format!("+CT,TURNCONTINUE={};", dir),
            Command::TiltTo(val) => format!("+CR,TILTVALUE={:.2};", val),
            Command::StopTilt => "+CR,STOP;".into(),
            Command::ZeroTilt => "+CR,TOZERO;".into(),
            Command::QueryAngle => "+QT,CHANGEANGLE;".into(),
            Command::Custom(s) => s.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_strings() {
        assert_eq!(
            Command::SetRotationSpeed(35.64).to_string(),
            "+CT,TURNSPEED=35.64;"
        );
        assert_eq!(
            Command::RotateBy(-30.5).to_string(),
            "+CT,TURNANGLE=-30.50;"
        );
        assert_eq!(Command::StopRotation.to_string(), "+CT,STOP;");
        assert_eq!(Command::ZeroTilt.to_string(), "+CR,TOZERO;");
        assert_eq!(Command::QueryAngle.to_string(), "+QT,CHANGEANGLE;");
        let custom = Command::Custom("+FOO,BAR;".into());
        assert_eq!(custom.to_string(), "+FOO,BAR;");
    }
}
