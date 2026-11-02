//! Reuse thread-owned raster surfaces; share only immutable drawing commands.
use super::*;
use std::{
    sync::{OnceLock, mpsc},
    thread::JoinHandle,
};

#[derive(Default)]
pub(super) struct RasterPool {
    workers: Vec<Worker>,
    pub(super) bands: Option<Vec<(SkRect, skia_safe::Picture)>>,
}

struct Job {
    picture: skia_safe::Picture,
    transform: Matrix,
    size: (i32, i32),
    background: Color,
    band: SkRect,
    complete: bool,
    reply: mpsc::Sender<Result<Image>>,
}

struct Worker {
    sender: Option<mpsc::SyncSender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn start(index: usize) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let thread = std::thread::Builder::new()
            .name(format!("lazypencil-raster-{index}"))
            .spawn(move || {
                let mut surface = None;
                let mut size = (0, 0);
                while let Ok(job) = receiver.recv() {
                    let result = (|| {
                        if size != job.size {
                            surface =
                                Some(skia_safe::surfaces::raster_n32_premul(job.size).ok_or_else(
                                    || invalid("Cannot allocate a preview raster band"),
                                )?);
                            size = job.size;
                        }
                        let surface = surface
                            .as_mut()
                            .expect("Raster dimensions were initialized");
                        let canvas = surface.canvas();
                        canvas.save();
                        // Preselected commands keep full device bounds for exact filter pixels.
                        if !job.complete {
                            canvas.clip_rect(job.band, ClipOp::Intersect, false);
                        }
                        canvas.clear(job.background);
                        canvas.draw_picture(&job.picture, Some(&job.transform), None);
                        canvas.restore();
                        Ok(surface.image_snapshot())
                    })();
                    let _ = job.reply.send(result);
                }
            })
            .map_err(|error| invalid(format!("Cannot start preview raster worker: {error}")))?;
        Ok(Self {
            sender: Some(sender),
            thread: Some(thread),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl RasterPool {
    pub(super) fn parts(size: (i32, i32)) -> usize {
        static PARALLELISM: OnceLock<usize> = OnceLock::new();
        if size.0 as u64 * (size.1 as u64) < 512 * 512 {
            1
        } else {
            *PARALLELISM
                .get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(4)))
        }
    }
    pub(super) fn draw(
        &mut self,
        canvas: &Canvas,
        picture: &skia_safe::Picture,
        transform: &Matrix,
        size: (i32, i32),
        background: Color,
        budget: &Rc<backdrop::RasterBudget>,
    ) -> Result<()> {
        let parts = self
            .bands
            .as_ref()
            .map_or_else(|| Self::parts(size), Vec::len);
        if parts == 1 {
            self.workers.clear();
            canvas.draw_picture(picture, Some(transform), None);
            return Ok(());
        }
        let _reservation = budget.reserve(size.0 as u64 * size.1 as u64 * 4 * parts as u64)?;
        let result = (|| {
            self.workers.truncate(parts);
            while self.workers.len() < parts {
                self.workers.push(Worker::start(self.workers.len())?);
            }
            let mut pending = Vec::with_capacity(parts);
            for (part, worker) in self.workers.iter().enumerate() {
                let top = size.1 * part as i32 / parts as i32;
                let bottom = size.1 * (part + 1) as i32 / parts as i32;
                let band = self.bands.as_ref().map_or_else(
                    || SkRect::new(0.0, top as f32, size.0 as f32, bottom as f32),
                    |bands| bands[part].0,
                );
                let (reply, receiver) = mpsc::channel();
                worker
                    .sender
                    .as_ref()
                    .expect("Live raster worker")
                    .send(Job {
                        picture: self
                            .bands
                            .as_ref()
                            .map_or_else(|| picture.clone(), |bands| bands[part].1.clone()),
                        transform: *transform,
                        size,
                        background,
                        band,
                        complete: self.bands.is_some(),
                        reply,
                    })
                    .map_err(|_| invalid("Preview raster worker stopped"))?;
                pending.push((band, receiver));
            }
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::Src);
            for (band, receiver) in pending {
                let image = receiver
                    .recv()
                    .map_err(|_| invalid("Preview raster worker stopped"))??;
                canvas.save();
                canvas.clip_rect(band, ClipOp::Intersect, false);
                canvas.draw_image(&image, (0.0, 0.0), Some(&paint));
                canvas.restore();
            }
            Ok(())
        })();
        if result.is_err() {
            self.workers.clear();
        }
        result
    }
}
