//! Bundled video playback. One decoder worker and one replaceable frame;
//! no browser, network, subprocess, phone connection or UI-thread decoding.
use openh264::formats::YUVSource;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};

pub const FPS: u32 = 12;
pub const FRAME_COUNT: u32 = 468;
pub const FRAME_WIDTH: usize = 960;
pub const FRAME_HEIGHT: usize = 600;
pub const FRAME_ASSET: &str = "stepx-release-video-frame";
const BYTES: &[u8] = include_bytes!(concat!(
    "../assets/release-notes/",
    env!("CARGO_PKG_VERSION"),
    "/tour.h264"
));

#[derive(Clone, Copy, Debug)]
pub enum Control {
    Play,
    Pause,
    Seek(u32),
}
#[derive(Default)]
pub struct Snapshot {
    pub revision: u64,
    pub frame: u32,
    pub playing: bool,
    pub ended: bool,
    pub error: Option<String>,
    pub jpeg: Option<Vec<u8>>,
}
struct Shared {
    state: Mutex<Snapshot>,
    stop: AtomicBool,
}
pub struct Player {
    tx: mpsc::SyncSender<Control>,
    shared: Arc<Shared>,
}
impl Player {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::sync_channel(16);
        let shared = Arc::new(Shared {
            state: Mutex::new(Snapshot::default()),
            stop: AtomicBool::new(false),
        });
        let worker = shared.clone();
        std::thread::spawn(move || {
            if let Err(error) = run(rx, &worker) {
                publish(&worker, |s| {
                    s.error = Some(error);
                    s.playing = false;
                });
            }
        });
        Self { tx, shared }
    }
    pub fn control(&self, control: Control) {
        let _ = self.tx.try_send(control);
    }
    /// Takes the only pending image. Slow consumers never accumulate frames.
    pub fn poll(&self, last_revision: u64) -> Option<Snapshot> {
        let mut s = self.shared.state.lock().ok()?;
        if s.revision == last_revision {
            return None;
        }
        Some(Snapshot {
            revision: s.revision,
            frame: s.frame,
            playing: s.playing,
            ended: s.ended,
            error: s.error.clone(),
            jpeg: s.jpeg.take(),
        })
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.tx.try_send(Control::Pause);
    }
}
fn publish(shared: &Shared, update: impl FnOnce(&mut Snapshot)) {
    if let Ok(mut s) = shared.state.lock() {
        update(&mut s);
        s.revision += 1;
    }
}
fn run(rx: mpsc::Receiver<Control>, shared: &Shared) -> Result<(), String> {
    use openh264::decoder::Decoder;
    let mut decoder = Decoder::new().map_err(|e| e.to_string())?;
    let nals: Vec<&[u8]> = openh264::nal_units(BYTES).collect();
    let mut cursor = 0usize;
    let mut frame = 0u32;
    let mut playing = false;
    let mut seek = None;
    let mut pending = None;
    let mut due = Instant::now();
    while !shared.stop.load(Ordering::Acquire) {
        if let Some(control) = pending.take().or_else(|| rx.try_recv().ok()) {
            match control {
                Control::Play => {
                    if cursor >= nals.len() {
                        cursor = 0;
                        frame = 0;
                        decoder = Decoder::new().map_err(|e| e.to_string())?;
                    }
                    playing = true;
                    due = Instant::now();
                    publish(shared, |s| {
                        s.playing = true;
                        s.ended = false;
                    });
                }
                Control::Pause => {
                    playing = false;
                    publish(shared, |s| s.playing = false);
                }
                Control::Seek(target) => {
                    decoder = Decoder::new().map_err(|e| e.to_string())?;
                    cursor = 0;
                    frame = 0;
                    due = Instant::now();
                    seek = Some(target.min(FRAME_COUNT - 1));
                    publish(shared, |s| s.ended = false);
                }
            }
            // Apply queued controls before spending time decoding.
            continue;
        }
        if !playing && seek.is_none() {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(c) => pending = Some(c),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
            continue;
        }
        if cursor >= nals.len() {
            playing = false;
            seek = None;
            publish(shared, |s| {
                s.playing = false;
                s.ended = true;
            });
            continue;
        }
        let nal = nals[cursor];
        cursor += 1;
        if let Some(yuv) = decoder.decode(nal).map_err(|e| e.to_string())? {
            let number = frame;
            frame += 1;
            if seek.is_some_and(|target| number < target) {
                continue;
            }
            seek = None;
            let (w, h) = yuv.dimensions();
            if w == 0 || h == 0 || w > 1280 || h > 900 {
                return Err("视频尺寸超出播放范围".into());
            }
            let mut rgb = vec![0; yuv.rgb8_len()];
            yuv.write_rgb8(&mut rgb);
            let mut jpeg = Vec::with_capacity(96 * 1024);
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 82)
                .encode(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)
                .map_err(|e| e.to_string())?;
            publish(shared, |s| {
                s.frame = number;
                s.jpeg = Some(jpeg);
            });
            if playing {
                due += Duration::from_micros(1_000_000 / u64::from(FPS));
                if Instant::now().saturating_duration_since(due) > Duration::from_millis(250) {
                    due = Instant::now();
                }
                match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                    Ok(c) => pending = Some(c),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(_) => {}
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait_for(p: &Player, mut condition: impl FnMut(&Snapshot) -> bool) -> Snapshot {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(s) = p.poll(u64::MAX) {
                assert!(s.error.is_none(), "{:?}", s.error);
                if condition(&s) {
                    return s;
                }
            }
            assert!(std::time::Instant::now() < deadline, "player timeout");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn bundled_video_decodes_the_declared_frames() {
        let mut d = openh264::decoder::Decoder::new().unwrap();
        let mut count = 0;
        for n in openh264::nal_units(BYTES) {
            if let Some(yuv) = d.decode(n).unwrap() {
                assert_eq!(yuv.dimensions(), (FRAME_WIDTH, FRAME_HEIGHT));
                count += 1;
            }
        }
        assert_eq!(count, FRAME_COUNT);
        assert!(BYTES.len() < 1024 * 1024);
    }
    #[test]
    fn player_is_idle_until_clicked_and_supports_pause_seek_resume_and_drop() {
        let p = Player::start();
        std::thread::sleep(Duration::from_millis(140));
        assert!(p.poll(0).is_none());
        p.control(Control::Play);
        let first = wait_for(&p, |s| s.frame >= 2 && s.jpeg.is_some());
        assert!(image::load_from_memory(first.jpeg.as_ref().unwrap()).is_ok());
        p.control(Control::Pause);
        let paused = wait_for(&p, |s| !s.playing);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(p.poll(u64::MAX).unwrap().frame, paused.frame);
        p.control(Control::Seek(12 * FPS));
        let sought = wait_for(&p, |s| s.frame == 12 * FPS && s.jpeg.is_some());
        assert!(!sought.playing);
        // Frame storage is a single replaceable slot and poll consumes it.
        assert!(p.poll(u64::MAX).unwrap().jpeg.is_none());
        p.control(Control::Play);
        wait_for(&p, |s| s.frame > 12 * FPS && s.playing);
        let shared = p.shared.clone();
        drop(p);
        assert!(shared.stop.load(Ordering::Acquire));
        let deadline = Instant::now() + Duration::from_secs(1);
        while Arc::strong_count(&shared) > 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            Arc::strong_count(&shared),
            1,
            "decoder worker must release its state after close"
        );
    }
}
