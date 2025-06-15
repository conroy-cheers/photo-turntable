use std::path::Path;

use anyhow::Error;
use gphoto2::{file::CameraFile, list::CameraListIter, Context};

pub(crate) struct CameraContext {
    pub(super) context: gphoto2::Context,
}

impl CameraContext {
    pub(crate) fn new() -> Result<Self, Error> {
        let context = Context::new()?;
        Ok(Self { context })
    }

    pub(crate) fn list_cameras(&self) -> Result<Vec<CameraSpec>, Error> {
        let cameras: CameraListIter = self.context.list_cameras().wait()?;
        Ok(cameras
            .map(|descriptor| CameraSpec { descriptor })
            .collect())
    }
}

/// A detected camera that can be connected to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CameraSpec {
    descriptor: gphoto2::list::CameraDescriptor,
}

impl CameraSpec {
    pub(crate) fn connect(&self, context: &CameraContext) -> Result<Camera, Error> {
        let device = context.context.get_camera(&self.descriptor).wait()?;
        Ok(Camera { device })
    }

    pub(crate) fn name(&self) -> String {
        self.descriptor.model.clone()
    }
}

/// An open connection to a camera.
#[derive(Clone)]
pub(crate) struct Camera {
    device: gphoto2::Camera,
}

impl Camera {
    pub(crate) fn capture(&self, path: &Path) -> Result<CameraFile, Error> {
        let camera_fs = self.device.fs();

        // And take pictures
        let file_path = self.device.capture_image().wait()?;
        // let preview = camera_fs
        //     .download_preview(&file_path.folder(), &file_path.name())
        //     .wait()?;
        let file = camera_fs
            .download_to(&file_path.folder(), &file_path.name(), path)
            .wait()?;

        Ok(file)
    }
}
