use std::path::PathBuf;

use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinSet,
};

use crate::app::{worker::worker_camera::ImageHandle, ImagePreview};

/// Image loader task.
/// - `camera_imagepath_rx` delivers `ImageHandle`s.  
/// - `image_tx` is where previews get sent.  
/// - `ctx` is your egui context (must be `Clone + Send + Sync`).  
/// - `max_width`/`max_height` cap the thumbnail dimensions.
pub async fn image_loader(
    mut camera_imagepath_rx: UnboundedReceiver<ImageHandle>,
    image_tx: UnboundedSender<ImagePreview>,
) {
    // Keep track of all in‐flight loads
    let mut join_set: JoinSet<()> = JoinSet::new();

    // Drain incoming handles
    while let Some(handle) = camera_imagepath_rx.recv().await {
        let tx = image_tx.clone();
        // let ctx = ctx.clone();
        let path = handle.path.clone();

        // Spawn blocking work for image decoding & resizing
        join_set.spawn_blocking(move || {
            match ImagePreview::load(handle.seq, &path).and_then(|preview| {
                tx.send(preview)
                    .map_err(|e| anyhow::anyhow!("Send error: {}", e))
            }) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Error occurred loading image: {:?}", e)
                }
            }
        });
    }

    // Wait for all remaining tasks to finish
    while let Some(join_res) = join_set.join_next().await {
        if let Err(join_err) = join_res {
            // join_err is a JoinError (panic or cancellation)
            eprintln!("Image‐load task failed: {:?}", join_err);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportJob {
    pub image_path: PathBuf,
    pub seq: u32,
    pub output_directory: PathBuf,
}

pub async fn image_exporter(mut job_rx: UnboundedReceiver<ExportJob>) {
    let mut join_set: JoinSet<()> = JoinSet::new();

    while let Some(job) = job_rx.recv().await {
        join_set.spawn_blocking(move || {
            let dest_path = job
                .output_directory
                .join(format!("image_{}", job.seq))
                .with_extension("jpg");
            match std::fs::copy(&job.image_path, &dest_path) {
                Ok(_) => {}
                Err(e) => println!(
                    "Something went wrong trying to copy {:?} to {:?}: {:?}",
                    job.image_path, dest_path, e
                ),
            };
        });
    }

    // Wait for all remaining tasks to finish
    while let Some(join_res) = join_set.join_next().await {
        if let Err(join_err) = join_res {
            // join_err is a JoinError (panic or cancellation)
            eprintln!("Image export task failed: {:?}", join_err);
        }
    }
}
