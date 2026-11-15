//! macOS integration through AppKit and AVFoundation:
//!
//! - the application menu (App, Edit, Window), as main.js `macAppMenu` built it; the Edit items
//!   reach the text fields as the same events their shortcuts send,
//! - the camera (AVCaptureSession) for scanning a paper backup's QR codes, frames in BGRA,
//! - audio and video playback (AVPlayer) straight from memory: a resource loader hands the
//!   decrypted bytes to AVFoundation, so nothing is written to disk to play it.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_av_foundation::{
    AVAssetResourceLoader, AVAssetResourceLoaderDelegate, AVAssetResourceLoadingRequest, AVAuthorizationStatus,
    AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceInput, AVCaptureOutput, AVCaptureSession,
    AVCaptureSessionPreset1280x720, AVCaptureVideoDataOutput, AVCaptureVideoDataOutputSampleBufferDelegate,
    AVMediaTypeVideo, AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerItemVideoOutput, AVURLAsset,
};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{NSData, NSDictionary, NSNumber, NSString, NSURL};

static CTX: OnceLock<egui::Context> = OnceLock::new();

fn repaint() {
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

// -------------------------------------------------------------------------------------------
// Application menu
// -------------------------------------------------------------------------------------------
static MENU_EVENTS: Mutex<Vec<egui::Event>> = Mutex::new(Vec::new());

fn push(e: egui::Event) {
    if let Ok(mut q) = MENU_EVENTS.lock() {
        q.push(e);
    }
    repaint();
}

fn key(k: egui::Key, shift: bool) {
    let mut m = egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND;
    if shift {
        m = m | egui::Modifiers::SHIFT;
    }
    push(egui::Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
    push(egui::Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
}

/// Edit-menu actions queued since the last frame (fed in through `raw_input_hook`).
pub fn take_menu_events() -> Vec<egui::Event> {
    MENU_EVENTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and MenuTarget does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MVMenuTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(mvUndo:))]
        fn undo(&self, _sender: Option<&AnyObject>) {
            key(egui::Key::Z, false);
        }
        #[unsafe(method(mvRedo:))]
        fn redo(&self, _sender: Option<&AnyObject>) {
            key(egui::Key::Z, true);
        }
        #[unsafe(method(mvCut:))]
        fn cut(&self, _sender: Option<&AnyObject>) {
            push(egui::Event::Cut);
        }
        #[unsafe(method(mvCopy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            push(egui::Event::Copy);
        }
        #[unsafe(method(mvPaste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            if let Some(t) = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()) {
                push(egui::Event::Paste(t));
            }
        }
        #[unsafe(method(mvSelectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            key(egui::Key::A, false);
        }
    }

    unsafe impl NSObjectProtocol for MenuTarget {}
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn item(mtm: MainThreadMarker, title: &str, action: Option<objc2::runtime::Sel>, key: &str) -> Retained<NSMenuItem> {
    unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), action, &NSString::from_str(key)) }
}

/// The App, Edit and Window menus (no View menu: nothing to reload, no developer tools).
pub fn install_menu(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let target = MenuTarget::new(mtm);
    let main = NSMenu::new(mtm);
    let name = "Memoro Vault";

    let app_menu = NSMenu::new(mtm);
    app_menu.addItem(&item(mtm, &format!("About {name}"), Some(sel!(orderFrontStandardAboutPanel:)), ""));
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&item(mtm, &format!("Hide {name}"), Some(sel!(hide:)), "h"));
    let others = item(mtm, "Hide Others", Some(sel!(hideOtherApplications:)), "h");
    others.setKeyEquivalentModifierMask(NSEventModifierFlags::Option | NSEventModifierFlags::Command);
    app_menu.addItem(&others);
    app_menu.addItem(&item(mtm, "Show All", Some(sel!(unhideAllApplications:)), ""));
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&item(mtm, &format!("Quit {name}"), Some(sel!(terminate:)), "q"));
    let app_item = NSMenuItem::new(mtm);
    app_item.setSubmenu(Some(&app_menu));
    main.addItem(&app_item);

    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    for (title, action, k) in [
        ("Undo", sel!(mvUndo:), "z"),
        ("Redo", sel!(mvRedo:), "Z"),
        ("", sel!(mvUndo:), ""),
        ("Cut", sel!(mvCut:), "x"),
        ("Copy", sel!(mvCopy:), "c"),
        ("Paste", sel!(mvPaste:), "v"),
        ("Select All", sel!(mvSelectAll:), "a"),
    ] {
        if title.is_empty() {
            edit.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        let it = item(mtm, title, Some(action), k);
        unsafe { it.setTarget(Some(&target)) };
        edit.addItem(&it);
    }
    let edit_item = NSMenuItem::new(mtm);
    edit_item.setSubmenu(Some(&edit));
    main.addItem(&edit_item);

    let window = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Window"));
    window.addItem(&item(mtm, "Minimize", Some(sel!(performMiniaturize:)), "m"));
    window.addItem(&item(mtm, "Zoom", Some(sel!(performZoom:)), ""));
    window.addItem(&NSMenuItem::separatorItem(mtm));
    window.addItem(&item(mtm, "Close", Some(sel!(performClose:)), "w"));
    let window_item = NSMenuItem::new(mtm);
    window_item.setSubmenu(Some(&window));
    main.addItem(&window_item);

    app.setMainMenu(Some(&main));
    app.setWindowsMenu(Some(&window));
    // menu items hold their target weakly: it lives as long as the app
    std::mem::forget(target);
}

// -------------------------------------------------------------------------------------------
// Pixel buffers
// -------------------------------------------------------------------------------------------
/// A BGRA picture from the camera or a video.
#[derive(Clone)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    /// tightly packed BGRA
    pub bgra: Vec<u8>,
}

impl Frame {
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bgra.len());
        for p in self.bgra.chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        out
    }

    pub fn luma(&self) -> Vec<u8> {
        self.bgra.chunks_exact(4).map(|p| ((p[2] as u32 * 299 + p[1] as u32 * 587 + p[0] as u32 * 114) / 1000) as u8).collect()
    }
}

/// Copies a BGRA pixel buffer out.
fn copy_frame(pb: &CVPixelBuffer) -> Option<Frame> {
    unsafe {
        if CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) != 0 {
            return None;
        }
    }
    let (w, h, stride) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb), CVPixelBufferGetBytesPerRow(pb));
    let base = CVPixelBufferGetBaseAddress(pb) as *const u8;
    let frame = if base.is_null() || w == 0 || h == 0 || stride < w * 4 {
        None
    } else {
        let mut bgra = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row = unsafe { std::slice::from_raw_parts(base.add(y * stride), w * 4) };
            bgra.extend_from_slice(row);
        }
        Some(Frame { w, h, bgra })
    };
    unsafe { CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) };
    frame
}

fn bgra_settings() -> Retained<NSDictionary<NSString, AnyObject>> {
    // kCVPixelBufferPixelFormatTypeKey is a CFString, toll-free bridged to NSString
    let key: &NSString = unsafe { &*(kCVPixelBufferPixelFormatTypeKey as *const _ as *const NSString) };
    let value = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
    let value: &AnyObject = &value;
    NSDictionary::from_slices(&[key], &[value])
}

// -------------------------------------------------------------------------------------------
// Camera
// -------------------------------------------------------------------------------------------
struct SinkIvars {
    latest: Arc<Mutex<Option<Frame>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and FrameSink does not implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "MVFrameSink"]
    #[ivars = SinkIvars]
    struct FrameSink;

    unsafe impl NSObjectProtocol for FrameSink {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for FrameSink {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn did_output(&self, _output: &AVCaptureOutput, sample: &CMSampleBuffer, _connection: &AVCaptureConnection) {
            #[allow(deprecated)]
            let Some(img) = (unsafe { objc2_core_media::CMSampleBufferGetImageBuffer(sample) }) else { return };
            if let Some(f) = copy_frame(&img) {
                if let Ok(mut l) = self.ivars().latest.lock() {
                    *l = Some(f);
                }
                repaint();
            }
        }
    }
);

impl FrameSink {
    fn new(latest: Arc<Mutex<Option<Frame>>>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(SinkIvars { latest });
        unsafe { msg_send![super(this), init] }
    }
}

/// A running camera; frames arrive on a background queue, the newest kept.
pub struct Camera {
    session: Retained<AVCaptureSession>,
    _sink: Retained<FrameSink>,
    _queue: DispatchRetained<DispatchQueue>,
    latest: Arc<Mutex<Option<Frame>>>,
}

impl Camera {
    pub fn latest(&self) -> Option<Frame> {
        self.latest.lock().ok().and_then(|l| l.clone())
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        unsafe { self.session.stopRunning() };
    }
}

pub enum CameraStart {
    Running(Camera),
    /// macOS is asking the person; try again once they answer
    Asking,
    /// getUserMedia's error, as `name: message`
    Failed(String),
}

/// 0 = not asked yet, 1 = asking, 2 = granted, 3 = refused
static CAMERA_ANSWER: AtomicU8 = AtomicU8::new(0);

/// `startPaperCamera`: the default camera at 1280x720 (the original asked for that size).
pub fn start_camera(ctx: &egui::Context) -> CameraStart {
    let _ = CTX.set(ctx.clone());
    let Some(video) = (unsafe { AVMediaTypeVideo }) else { return CameraStart::Failed("NotSupportedError: Not supported".into()) };
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(video) };
    if status == AVAuthorizationStatus::NotDetermined {
        match CAMERA_ANSWER.load(Ordering::SeqCst) {
            0 => {
                CAMERA_ANSWER.store(1, Ordering::SeqCst);
                let block = RcBlock::new(|granted: Bool| {
                    CAMERA_ANSWER.store(if granted.as_bool() { 2 } else { 3 }, Ordering::SeqCst);
                    repaint();
                });
                unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(video, &block) };
                return CameraStart::Asking;
            }
            1 => return CameraStart::Asking,
            3 => return CameraStart::Failed("NotAllowedError: Permission denied".into()),
            _ => {}
        }
    } else if status != AVAuthorizationStatus::Authorized {
        return CameraStart::Failed("NotAllowedError: Permission denied".into());
    }
    let Some(device) = (unsafe { AVCaptureDevice::defaultDeviceWithMediaType(video) }) else {
        return CameraStart::Failed("NotFoundError: Requested device not found".into());
    };
    let input = match unsafe { AVCaptureDeviceInput::deviceInputWithDevice_error(&device) } {
        Ok(i) => i,
        Err(_) => return CameraStart::Failed("NotReadableError: Could not start video source".into()),
    };
    let session = unsafe { AVCaptureSession::new() };
    unsafe {
        if session.canSetSessionPreset(AVCaptureSessionPreset1280x720) {
            session.setSessionPreset(AVCaptureSessionPreset1280x720);
        }
        if !session.canAddInput(&input) {
            return CameraStart::Failed("NotReadableError: Could not start video source".into());
        }
        session.addInput(&input);
    }
    let output = unsafe { AVCaptureVideoDataOutput::new() };
    let latest = Arc::new(Mutex::new(None));
    let sink = FrameSink::new(latest.clone());
    let queue = DispatchQueue::new("mvault.camera", None);
    unsafe {
        output.setVideoSettings(Some(&bgra_settings()));
        output.setAlwaysDiscardsLateVideoFrames(true);
        output.setSampleBufferDelegate_queue(Some(ProtocolObject::from_ref(&*sink)), Some(&queue));
        if !session.canAddOutput(&output) {
            return CameraStart::Failed("NotReadableError: Could not start video source".into());
        }
        session.addOutput(&output);
        session.startRunning();
    }
    CameraStart::Running(Camera { session, _sink: sink, _queue: queue, latest })
}

// -------------------------------------------------------------------------------------------
// Audio and video from memory
// -------------------------------------------------------------------------------------------
struct LoaderIvars {
    bytes: Arc<Vec<u8>>,
    uti: String,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and MemLoader does not implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "MVMemLoader"]
    #[ivars = LoaderIvars]
    struct MemLoader;

    unsafe impl NSObjectProtocol for MemLoader {}

    unsafe impl AVAssetResourceLoaderDelegate for MemLoader {
        #[unsafe(method(resourceLoader:shouldWaitForLoadingOfRequestedResource:))]
        fn should_wait(&self, _loader: &AVAssetResourceLoader, request: &AVAssetResourceLoadingRequest) -> bool {
            let iv = self.ivars();
            let len = iv.bytes.len();
            unsafe {
                if let Some(info) = request.contentInformationRequest() {
                    info.setContentType(Some(&NSString::from_str(&iv.uti)));
                    info.setContentLength(len as i64);
                    info.setByteRangeAccessSupported(true);
                }
                if let Some(data) = request.dataRequest() {
                    let start = (data.requestedOffset().max(0) as usize).min(len);
                    let want = if data.requestsAllDataToEndOfResource() { len - start } else { (data.requestedLength().max(0) as usize).min(len - start) };
                    // in pieces, so a long file is never copied whole at once
                    let mut at = start;
                    while at < start + want {
                        let end = (at + (1 << 20)).min(start + want);
                        data.respondWithData(&NSData::with_bytes(&iv.bytes[at..end]));
                        at = end;
                    }
                }
                request.finishLoading();
            }
            true
        }
    }
);

impl MemLoader {
    fn new(bytes: Arc<Vec<u8>>, uti: String) -> Retained<Self> {
        let this = Self::alloc().set_ivars(LoaderIvars { bytes, uti });
        unsafe { msg_send![super(this), init] }
    }
}

/// The uniform type AVFoundation needs to pick a demuxer.
fn uti_for(name: &str, mime: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match (ext.as_str(), mime) {
        ("mp4", _) | (_, "video/mp4") => "public.mpeg-4",
        ("m4v", _) | (_, "video/x-m4v") => "com.apple.m4v-video",
        ("mov", _) | (_, "video/quicktime") => "com.apple.quicktime-movie",
        ("mp3", _) | (_, "audio/mpeg") | (_, "audio/mp3") => "public.mp3",
        ("m4a", _) | (_, "audio/mp4") | (_, "audio/x-m4a") => "com.apple.m4a-audio",
        ("aac", _) | (_, "audio/aac") => "public.aac-audio",
        ("wav", _) | (_, "audio/wav") | (_, "audio/x-wav") | (_, "audio/wave") => "com.microsoft.waveform-audio",
        ("aif" | "aiff", _) | (_, "audio/aiff") => "public.aiff-audio",
        ("flac", _) | (_, "audio/flac") | (_, "audio/x-flac") => "org.xiph.flac",
        ("caf", _) => "com.apple.coreaudio-format",
        ("3gp", _) => "public.3gpp",
        (e, m) if m.starts_with("video/") || ["webm", "ogv"].contains(&e) => "public.movie",
        _ => "public.audio",
    }
}

pub enum PlayerState {
    Loading,
    Ready,
    Failed(String),
}

/// An AVPlayer playing one in-memory file; video frames come out as BGRA.
pub struct Player {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    output: Retained<AVPlayerItemVideoOutput>,
    _asset: Retained<AVURLAsset>,
    _loader: Retained<MemLoader>,
    _queue: DispatchRetained<DispatchQueue>,
}

impl Player {
    /// Must run on the main thread.
    pub fn open(bytes: Arc<Vec<u8>>, name: &str, mime: &str, ctx: &egui::Context) -> Result<Player, String> {
        let _ = CTX.set(ctx.clone());
        let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
        let ext = name.rsplit('.').next().filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("bin");
        let url = NSURL::URLWithString(&NSString::from_str(&format!("mvault-mem://file/media.{ext}"))).ok_or("bad url")?;
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
        let loader = MemLoader::new(bytes, uti_for(name, mime).to_string());
        let queue = DispatchQueue::new("mvault.media", None);
        unsafe { asset.resourceLoader().setDelegate_queue(Some(ProtocolObject::from_ref(&*loader)), Some(&queue)) };
        let item = unsafe { AVPlayerItem::playerItemWithAsset(&asset, mtm) };
        let output = unsafe { AVPlayerItemVideoOutput::initWithPixelBufferAttributes(AVPlayerItemVideoOutput::alloc(), Some(&bgra_settings())) };
        unsafe { item.addOutput(&output) };
        let player = unsafe { AVPlayer::playerWithPlayerItem(Some(&item), mtm) };
        // the data comes from memory: no network-style waiting for "enough" to buffer
        unsafe { player.setAutomaticallyWaitsToMinimizeStalling(false) };
        Ok(Player { player, item, output, _asset: asset, _loader: loader, _queue: queue })
    }

    pub fn state(&self) -> PlayerState {
        match unsafe { self.item.status() } {
            AVPlayerItemStatus::ReadyToPlay => PlayerState::Ready,
            AVPlayerItemStatus::Failed => PlayerState::Failed(
                unsafe { self.item.error() }.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "unknown error".into()),
            ),
            _ => PlayerState::Loading,
        }
    }

    pub fn duration(&self) -> f64 {
        unsafe { self.item.duration().seconds() }
    }

    pub fn time(&self) -> f64 {
        unsafe { self.item.currentTime().seconds() }
    }

    pub fn playing(&self) -> bool {
        unsafe { self.player.rate() != 0.0 }
    }

    pub fn play(&self) {
        let (t, d) = (self.time(), self.duration());
        if d.is_finite() && t >= d - 0.05 {
            self.seek(0.0);
        }
        unsafe { self.player.play() };
    }

    pub fn pause(&self) {
        unsafe { self.player.pause() };
    }

    pub fn seek(&self, secs: f64) {
        let t = unsafe { CMTime::with_seconds(secs.max(0.0), 600) };
        unsafe { self.player.seekToTime_toleranceBefore_toleranceAfter(t, CMTime::with_seconds(0.0, 600), CMTime::with_seconds(0.0, 600)) };
    }

    pub fn muted(&self) -> bool {
        unsafe { self.player.isMuted() }
    }

    pub fn set_muted(&self, m: bool) {
        unsafe { self.player.setMuted(m) };
    }

    /// The video's display size (0 x 0 for sound only).
    pub fn video_size(&self) -> (f32, f32) {
        let s = unsafe { self.item.presentationSize() };
        (s.width as f32, s.height as f32)
    }

    /// A new video frame, when one is due.
    pub fn frame(&self) -> Option<Frame> {
        unsafe {
            let t = self.item.currentTime();
            if !self.output.hasNewPixelBufferForItemTime(t) {
                return None;
            }
            let pb = self.output.copyPixelBufferForItemTime_itemTimeForDisplay(t, std::ptr::null_mut())?;
            copy_frame(&pb)
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            self.player.pause();
            self.player.replaceCurrentItemWithPlayerItem(None);
        }
    }
}
