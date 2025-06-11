use anyhow::Error;
use gphoto2::Context;
use std::path::Path;

pub(crate) struct Camera {
    context: gphoto2::Context,
}

impl Camera {
    pub(crate) fn new() -> Result<Self, Error> {
        let context = Context::new()?;
        Ok(Self { context })
    }

    pub(crate) fn capture(&self) -> Result<(), Error> {
        for camera in self.context.list_cameras().wait()? {
            println!("{:?}", camera);
        }

        // Create a new context and detect the first camera from it
        let camera = self
            .context
            .autodetect_camera()
            .wait()
            .expect("Failed to autodetect camera");
        let camera_fs = camera.fs();

        // And take pictures
        let file_path = camera
            .capture_image()
            .wait()
            .expect("Could not capture image");
        camera_fs
            .download_to(
                &file_path.folder(),
                &file_path.name(),
                Path::new(&file_path.name().to_string()),
            )
            .wait()?;

        Ok(())
    }
}
