//! Audio and video the system player cannot read, decoded in Rust: WebM and Matroska (VP8, VP9,
//! AV1 video; Opus, Vorbis audio) and Ogg (Theora video; Opus, Vorbis, FLAC audio).
//!
//! The sound track is decoded to PCM up front and handed to the system player as an in-memory
//! WAV, which plays it and keeps the clock. Video frames are decoded on a background thread a
//! few ahead of that clock. Everything stays in memory.

use std::collections::VecDeque;
use std::io::Cursor;
use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// An RGBA picture.
#[derive(Clone)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// WebM / Matroska (EBML) or Ogg: the formats this module plays.
pub fn is_soft(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) || bytes.starts_with(b"OggS")
}

// -------------------------------------------------------------------------------------------
// Sound: decoded once to PCM, played as WAV
// -------------------------------------------------------------------------------------------

/// The first sound track, decoded to 16-bit PCM in a WAV file (at most two channels).
pub fn decode_audio_wav(bytes: &Arc<Vec<u8>>) -> Result<Option<(Vec<u8>, f64)>, String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::codecs::registry::CodecRegistry;
    use symphonia::core::common::Tier;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mut codecs = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut codecs);
    codecs.register_audio_decoder_at_tier::<symphonia_opus::OpusDecoder>(Tier::Fallback);

    let src = Cursor::new(bytes.as_ref().clone());
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let mut format = match symphonia::default::get_probe().probe(&Hint::new(), mss, FormatOptions::default(), MetadataOptions::default()) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let Some(track) = format.default_track(TrackType::Audio) else { return Ok(None) };
    let track_id = track.id;
    let Some(params) = track.codec_params.as_ref().and_then(|p| p.audio()).cloned() else { return Ok(None) };
    let mut decoder = codecs.make_audio_decoder(&params, &AudioDecoderOptions::default()).map_err(|e| format!("{e}"))?;
    let mut rate = params.sample_rate.unwrap_or(48_000);
    let mut out: Vec<i16> = Vec::new();
    let mut buf: Vec<i16> = Vec::new();
    let mut channels = 0usize;
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::ResetRequired) => break,
            Err(_) => break,
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = audio.spec();
                rate = spec.rate();
                let ch = spec.channels().count().max(1);
                channels = ch;
                buf.resize(audio.samples_interleaved(), 0);
                audio.copy_to_slice_interleaved(&mut buf);
                if ch <= 2 {
                    out.extend_from_slice(&buf);
                } else {
                    // front left and right of a wider layout
                    for f in buf.chunks_exact(ch) {
                        out.push(f[0]);
                        out.push(f[1]);
                    }
                }
            }
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("{e}")),
        }
    }
    if out.is_empty() {
        return Ok(None);
    }
    let ch = channels.clamp(1, 2) as u16;
    let secs = out.len() as f64 / ch as f64 / rate as f64;
    Ok(Some((wav(&out, rate, ch), secs)))
}

fn wav(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    w.extend_from_slice(&(channels * 2).to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

// -------------------------------------------------------------------------------------------
// Video: demuxed up front, decoded on demand
// -------------------------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VCodec {
    Vp8,
    Vp9,
    Av1,
    Theora,
}

pub struct VPacket {
    pub pts: f64,
    pub key: bool,
    pub data: Vec<u8>,
}

pub struct VideoTrack {
    pub codec: VCodec,
    pub w: u32,
    pub h: u32,
    pub packets: Vec<VPacket>,
    /// a frame's time on screen
    pub frame_dur: f64,
    /// Theora's three header packets
    headers: Vec<Vec<u8>>,
}

impl VideoTrack {
    pub fn end(&self) -> f64 {
        self.packets.last().map(|p| p.pts + self.frame_dur).unwrap_or(0.0)
    }

    /// The packet to start decoding at for time `t`: the last keyframe at or before it.
    fn start_index(&self, t: f64) -> usize {
        let mut k = 0;
        for (i, p) in self.packets.iter().enumerate() {
            if p.pts > t + 1e-6 {
                break;
            }
            if p.key {
                k = i;
            }
        }
        k
    }
}

/// The first video track of a WebM/Matroska or Ogg file, with the file's duration.
pub fn demux_video(bytes: &[u8]) -> Result<(Option<VideoTrack>, f64), String> {
    if bytes.starts_with(b"OggS") { demux_ogg(bytes) } else { demux_mkv(bytes) }
}

fn demux_mkv(bytes: &[u8]) -> Result<(Option<VideoTrack>, f64), String> {
    use matroska_demuxer::{Frame, MatroskaFile, TrackType};
    let mut mkv = MatroskaFile::open(Cursor::new(bytes)).map_err(|e| format!("{e}"))?;
    let scale = mkv.info().timestamp_scale().get() as f64 / 1e9;
    let duration = mkv.info().duration().map(|d| d * scale).unwrap_or(0.0);
    let track = mkv.tracks().iter().find(|t| t.track_type() == TrackType::Video && matches!(t.codec_id(), "V_VP8" | "V_VP9" | "V_AV1"));
    let Some(track) = track else { return Ok((None, duration)) };
    let codec = match track.codec_id() {
        "V_VP8" => VCodec::Vp8,
        "V_VP9" => VCodec::Vp9,
        _ => VCodec::Av1,
    };
    let number = track.track_number().get();
    let (w, h) = track.video().map(|v| (v.pixel_width().get() as u32, v.pixel_height().get() as u32)).unwrap_or((0, 0));
    let default_dur = track.default_duration().map(|d| d.get() as f64 / 1e9);
    let mut packets = Vec::new();
    let mut f = Frame::default();
    while mkv.next_frame(&mut f).map_err(|e| format!("{e}"))? {
        if f.track != number || f.data.is_empty() {
            continue;
        }
        let pts = f.timestamp as f64 * scale;
        let key = f.is_keyframe.unwrap_or_else(|| bitstream_key(codec, &f.data));
        packets.push(VPacket { pts, key, data: std::mem::take(&mut f.data) });
    }
    if let Some(p) = packets.first_mut() {
        p.key = true;
    }
    let frame_dur = default_dur.unwrap_or_else(|| typical_gap(&packets));
    let end = packets.last().map(|p| p.pts + frame_dur).unwrap_or(0.0);
    Ok((Some(VideoTrack { codec, w, h, packets, frame_dur, headers: vec![] }), duration.max(end)))
}

/// Keyframe from the bitstream when the container does not say (BlockGroups).
fn bitstream_key(codec: VCodec, d: &[u8]) -> bool {
    match codec {
        VCodec::Vp8 => d.first().is_some_and(|b| b & 1 == 0),
        // frame marker (2), profile (2, plus a reserved bit for profile 3), show_existing (1),
        // then frame_type 0 = key
        VCodec::Vp9 => d.first().is_some_and(|b| {
            let profile = ((b >> 5) & 1) | (((b >> 4) & 1) << 1);
            let bit = if profile == 3 { 2 } else { 3 };
            (b >> bit) & 1 == 0 && (b >> (bit - 1)) & 1 == 0
        }),
        _ => false,
    }
}

fn typical_gap(p: &[VPacket]) -> f64 {
    let mut gaps: Vec<f64> = p.windows(2).map(|w| w[1].pts - w[0].pts).filter(|g| *g > 0.0).collect();
    if gaps.is_empty() {
        return 1.0 / 30.0;
    }
    gaps.sort_by(|a, b| a.total_cmp(b));
    gaps[gaps.len() / 2]
}

fn demux_ogg(bytes: &[u8]) -> Result<(Option<VideoTrack>, f64), String> {
    let mut reader = ogg::reading::PacketReader::new(Cursor::new(bytes));
    let mut serial = None;
    let mut parser = theora_rs::HeaderParser::new();
    let mut headers = Vec::new();
    let mut packets = Vec::new();
    let mut n = 0u64;
    let mut fps = 0.0;
    let mut last_audio_gp = 0.0f64;
    let mut audio_rate = 0.0f64;
    while let Some(pkt) = reader.read_packet().map_err(|e| format!("{e}"))? {
        let s = pkt.stream_serial();
        let first = pkt.first_in_stream();
        let gp_page = pkt.absgp_page();
        let data = pkt.data;
        if first && serial.is_none() && data.len() >= 7 && data[0] == 0x80 && &data[1..7] == b"theora" {
            serial = Some(s);
        }
        // the sound track's length, from its last granule position
        if Some(s) != serial {
            if data.starts_with(b"\x01vorbis") && data.len() >= 16 {
                audio_rate = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as f64;
            } else if data.starts_with(b"OpusHead") {
                audio_rate = 48_000.0;
            }
            let gp = gp_page;
            if audio_rate > 0.0 && gp != u64::MAX {
                last_audio_gp = last_audio_gp.max(gp as f64 / audio_rate);
            }
            continue;
        }
        if headers.len() < 3 {
            let op = theora_rs::OggPacket { packet: data.clone(), b_o_s: headers.is_empty(), e_o_s: false, granulepos: 0, packetno: headers.len() as i64 };
            parser.push(&op).map_err(|e| format!("Theora header: {e}"))?;
            headers.push(data);
            if headers.len() == 3 {
                let i = &parser.info;
                fps = if i.fps_denominator > 0 { i.fps_numerator as f64 / i.fps_denominator as f64 } else { 25.0 };
            }
            continue;
        }
        let pts = n as f64 / fps.max(1.0);
        n += 1;
        // a data packet's second bit is 0 for an intra frame; an empty packet repeats the last
        let key = !data.is_empty() && data[0] & 0x40 == 0;
        packets.push(VPacket { pts, key, data });
    }
    let Some(_) = serial else { return Ok((None, last_audio_gp)) };
    if headers.len() < 3 {
        return Err("the Theora headers are incomplete".into());
    }
    if let Some(p) = packets.first_mut() {
        p.key = true;
    }
    let i = &parser.info;
    let (w, h) = (if i.pic_width > 0 { i.pic_width } else { i.frame_width }, if i.pic_height > 0 { i.pic_height } else { i.frame_height });
    let frame_dur = 1.0 / fps.max(1.0);
    let end = packets.len() as f64 * frame_dur;
    Ok((Some(VideoTrack { codec: VCodec::Theora, w, h, packets, frame_dur, headers }), end.max(last_audio_gp)))
}

// -------------------------------------------------------------------------------------------
// Frame decoders
// -------------------------------------------------------------------------------------------
enum Dec {
    Vp8(Box<vp8_decoder::Decoder>),
    Vp9(Box<rusty_vp9::Vp9Decoder>),
    Av1(Av1),
    Theora(Box<theora_rs::DecoderContext>, Box<theora_rs::Info>, i64),
}

impl Dec {
    fn new(t: &VideoTrack) -> Result<Dec, String> {
        Ok(match t.codec {
            VCodec::Vp8 => Dec::Vp8(Box::new(vp8_decoder::Decoder::new())),
            VCodec::Vp9 => Dec::Vp9(Box::new(rusty_vp9::Vp9Decoder::new())),
            VCodec::Av1 => Dec::Av1(Av1::new()?),
            VCodec::Theora => {
                let mut p = theora_rs::HeaderParser::new();
                for (k, h) in t.headers.iter().enumerate() {
                    let op = theora_rs::OggPacket { packet: h.clone(), b_o_s: k == 0, e_o_s: false, granulepos: 0, packetno: k as i64 };
                    p.push(&op).map_err(|e| format!("{e}"))?;
                }
                let d = p.decoder().map_err(|e| format!("{e}"))?;
                Dec::Theora(Box::new(d), Box::new(p.info.clone()), 3)
            }
        })
    }

    /// One packet in; the picture it shows, if any.
    fn decode(&mut self, pkt: &VPacket) -> Option<Image> {
        match self {
            Dec::Vp8(d) => match d.decode(&pkt.data) {
                Ok(Some(p)) => Some(yuv_to_rgba(p.width as usize, p.height as usize, &Planes::Eight {
                    y: (p.y, p.y_stride), u: (p.u, p.uv_stride), v: (p.v, p.uv_stride),
                }, (1, 1), Matrix::Bt601)),
                _ => None,
            },
            Dec::Vp9(d) => {
                d.push(&pkt.data, None).ok()?;
                let mut shown = None;
                while let Ok(f) = d.next_frame() {
                    shown = Some(f);
                }
                let f = shown?;
                let m = if f.height >= 720 { Matrix::Bt709 } else { Matrix::Bt601 };
                let planes = if f.bit_depth > 8 {
                    Planes::Wide { y: (&f.planes[0], f.strides[0]), u: (&f.planes[1], f.strides[1]), v: (&f.planes[2], f.strides[2]), shift: f.bit_depth - 8 }
                } else {
                    Planes::Eight { y: (&f.planes[0], f.strides[0]), u: (&f.planes[1], f.strides[1]), v: (&f.planes[2], f.strides[2]) }
                };
                Some(yuv_to_rgba(f.width as usize, f.height as usize, &planes, (f.subsampling_x, f.subsampling_y), m))
            }
            Dec::Av1(d) => d.decode(&pkt.data),
            Dec::Theora(d, info, packetno) => {
                let op = theora_rs::OggPacket { packet: pkt.data.clone(), b_o_s: false, e_o_s: false, granulepos: -1, packetno: *packetno };
                *packetno += 1;
                theora_rs::th_decode_packetin(d, &op).ok()?;
                let buf = theora_rs::th_decode_ycbcr_out(d).ok()?;
                Some(theora_rgba(&buf, info))
            }
        }
    }
}

enum Planes<'a> {
    Eight { y: (&'a [u8], usize), u: (&'a [u8], usize), v: (&'a [u8], usize) },
    /// little-endian u16 samples, `shift` bits above 8
    Wide { y: (&'a [u8], usize), u: (&'a [u8], usize), v: (&'a [u8], usize), shift: u32 },
}

#[derive(Clone, Copy)]
enum Matrix {
    Bt601,
    Bt709,
}

fn yuv_to_rgba(w: usize, h: usize, p: &Planes<'_>, (sx, sy): (u32, u32), m: Matrix) -> Image {
    let sample = |plane: (&[u8], usize), x: usize, y: usize, wide: Option<u32>| -> i32 {
        match wide {
            None => plane.0.get(y * plane.1 + x).copied().unwrap_or(128) as i32,
            Some(shift) => {
                let i = y * plane.1 + x * 2;
                let v = u16::from_le_bytes([plane.0.get(i).copied().unwrap_or(0), plane.0.get(i + 1).copied().unwrap_or(0)]);
                (v >> shift) as i32
            }
        }
    };
    let (yp, up, vp, wide) = match *p {
        Planes::Eight { y, u, v } => (y, u, v, None),
        Planes::Wide { y, u, v, shift } => (y, u, v, Some(shift)),
    };
    // limited-range coefficients, in 1/1024
    let (rv, gu, gv, bu) = match m {
        Matrix::Bt601 => (1634, 401, 832, 2066),
        Matrix::Bt709 => (1836, 218, 546, 2163),
    };
    let mut rgba = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let yy = (sample(yp, x, y, wide) - 16) * 1192;
            let u = sample(up, x >> sx, y >> sy, wide) - 128;
            let v = sample(vp, x >> sx, y >> sy, wide) - 128;
            let o = (y * w + x) * 4;
            rgba[o] = ((yy + rv * v + 512) >> 10).clamp(0, 255) as u8;
            rgba[o + 1] = ((yy - gu * u - gv * v + 512) >> 10).clamp(0, 255) as u8;
            rgba[o + 2] = ((yy + bu * u + 512) >> 10).clamp(0, 255) as u8;
        }
    }
    Image { w, h, rgba }
}

/// A Theora picture: its planes may run bottom-up (negative stride), cropped to the picture region.
fn theora_rgba(buf: &theora_rs::YCbCrBuffer, info: &theora_rs::Info) -> Image {
    let w = if info.pic_width > 0 { info.pic_width as usize } else { buf[0].width.max(1) as usize };
    let h = if info.pic_height > 0 { info.pic_height as usize } else { buf[0].height.max(1) as usize };
    let (ox, oy) = (info.pic_x as usize, info.pic_y as usize);
    let (hd, vd) = match info.pixel_fmt {
        theora_rs::PixelFmt::Pf444 => (0, 0),
        theora_rs::PixelFmt::Pf422 => (1, 0),
        _ => (1, 1),
    };
    let at = |p: &theora_rs::ImgPlane, x: usize, y: usize| -> i32 {
        let x = x.min(p.width.max(1) as usize - 1);
        let y = y.min(p.height.max(1) as usize - 1);
        let i = p.data_offset as isize + y as isize * p.stride as isize + x as isize;
        if i < 0 { 128 } else { p.data.get(i as usize).copied().unwrap_or(128) as i32 }
    };
    let mut rgba = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let yy = (at(&buf[0], ox + x, oy + y) - 16) * 1192;
            let u = at(&buf[1], (ox + x) >> hd, (oy + y) >> vd) - 128;
            let v = at(&buf[2], (ox + x) >> hd, (oy + y) >> vd) - 128;
            let o = (y * w + x) * 4;
            rgba[o] = ((yy + 1634 * v + 512) >> 10).clamp(0, 255) as u8;
            rgba[o + 1] = ((yy - 401 * u - 832 * v + 512) >> 10).clamp(0, 255) as u8;
            rgba[o + 2] = ((yy + 2066 * u + 512) >> 10).clamp(0, 255) as u8;
        }
    }
    Image { w, h, rgba }
}

/// AV1 through rav1d (the Rust port of dav1d), driven through its dav1d-compatible interface.
struct Av1 {
    ctx: Option<rav1d::include::dav1d::dav1d::Dav1dContext>,
}

impl Av1 {
    fn new() -> Result<Av1, String> {
        use rav1d::include::dav1d::dav1d::Dav1dSettings;
        let mut settings = std::mem::MaybeUninit::<Dav1dSettings>::uninit();
        let mut ctx = None;
        unsafe {
            rav1d::src::lib::dav1d_default_settings(NonNull::new(settings.as_mut_ptr()).unwrap());
            let mut s = settings.assume_init();
            s.n_threads = 2;
            s.max_frame_delay = 1;
            let r = rav1d::src::lib::dav1d_open(NonNull::new(&mut ctx), NonNull::new(&mut s));
            if r.0 < 0 || ctx.is_none() {
                return Err("the AV1 decoder could not start".into());
            }
        }
        Ok(Av1 { ctx })
    }

    fn decode(&mut self, data: &[u8]) -> Option<Image> {
        use rav1d::include::dav1d::data::Dav1dData;
        use rav1d::include::dav1d::picture::Dav1dPicture;
        let ctx = self.ctx.clone();
        let mut shown = None;
        unsafe {
            let mut d = Dav1dData::default();
            let buf = rav1d::src::lib::dav1d_data_create(NonNull::new(&mut d), data.len());
            if buf.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
            loop {
                let r = rav1d::src::lib::dav1d_send_data(ctx.clone(), NonNull::new(&mut d));
                let again = r.0 == -libc_eagain();
                // drain pictures; keep feeding while the decoder asks for more room
                loop {
                    let mut pic = Dav1dPicture::default();
                    let g = rav1d::src::lib::dav1d_get_picture(ctx.clone(), NonNull::new(&mut pic));
                    if g.0 < 0 {
                        break;
                    }
                    shown = av1_rgba(&pic);
                    rav1d::src::lib::dav1d_picture_unref(NonNull::new(&mut pic));
                }
                if !again || d.sz == 0 {
                    break;
                }
            }
            if d.sz > 0 {
                rav1d::src::lib::dav1d_data_unref(NonNull::new(&mut d));
            }
        }
        shown
    }
}

impl Drop for Av1 {
    fn drop(&mut self) {
        unsafe { rav1d::src::lib::dav1d_close(NonNull::new(&mut self.ctx)) };
    }
}

const fn libc_eagain() -> i32 {
    35 // EAGAIN on macOS
}

/// # Safety
/// `pic` was just filled by `dav1d_get_picture`.
unsafe fn av1_rgba(pic: &rav1d::include::dav1d::picture::Dav1dPicture) -> Option<Image> {
    use rav1d::include::dav1d::headers::{DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422};
    let (w, h) = (pic.p.w.max(0) as usize, pic.p.h.max(0) as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let bpc = pic.p.bpc.max(8) as u32;
    let bytes = if bpc > 8 { 2 } else { 1 };
    let (sx, sy, mono) = match pic.p.layout {
        l if l == DAV1D_PIXEL_LAYOUT_I400 => (1, 1, true),
        l if l == DAV1D_PIXEL_LAYOUT_I420 => (1, 1, false),
        l if l == DAV1D_PIXEL_LAYOUT_I422 => (1, 0, false),
        _ => (0, 0, false),
    };
    let ys = pic.stride[0] as usize;
    let cs = pic.stride[1] as usize;
    let y = unsafe { std::slice::from_raw_parts(pic.data[0]?.as_ptr() as *const u8, ys * h) };
    let ch = (h + sy as usize) >> sy;
    let neutral = vec![if bytes == 2 { 0 } else { 128 }; cs.max(1) * ch.max(1)];
    let neutral16: Vec<u8> = if bytes == 2 { (0..cs * ch / 2).flat_map(|_| (128u16 << (bpc - 8)).to_le_bytes()).collect() } else { vec![] };
    let (u, v): (&[u8], &[u8]) = if mono {
        if bytes == 2 { (&neutral16, &neutral16) } else { (&neutral, &neutral) }
    } else {
        unsafe {
            (
                std::slice::from_raw_parts(pic.data[1]?.as_ptr() as *const u8, cs * ch),
                std::slice::from_raw_parts(pic.data[2]?.as_ptr() as *const u8, cs * ch),
            )
        }
    };
    let m = if h >= 720 { Matrix::Bt709 } else { Matrix::Bt601 };
    let planes = if bytes == 2 { Planes::Wide { y: (y, ys), u: (u, cs), v: (v, cs), shift: bpc - 8 } } else { Planes::Eight { y: (y, ys), u: (u, cs), v: (v, cs) } };
    Some(yuv_to_rgba(w, h, &planes, (sx, sy), m))
}

// -------------------------------------------------------------------------------------------
// The decoding thread
// -------------------------------------------------------------------------------------------
struct VState {
    frames: VecDeque<(f64, Image)>,
    seek: Option<f64>,
    quit: bool,
}

/// Decodes a video track a few frames ahead; `frame_at(t)` hands out the one due at `t`.
pub struct SoftVideo {
    shared: Arc<(Mutex<VState>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

const AHEAD: usize = 6;

impl SoftVideo {
    pub fn start(track: Arc<VideoTrack>, ctx: Option<egui::Context>) -> SoftVideo {
        let shared = Arc::new((Mutex::new(VState { frames: VecDeque::new(), seek: Some(0.0), quit: false }), Condvar::new()));
        let sh = shared.clone();
        let thread = std::thread::spawn(move || {
            let (lock, cv) = &*sh;
            let mut dec: Option<Dec> = None;
            let mut idx = 0usize;
            let mut skip_until = 0.0f64;
            loop {
                {
                    let mut st = lock.lock().unwrap();
                    if st.quit {
                        return;
                    }
                    if let Some(t) = st.seek.take() {
                        st.frames.clear();
                        idx = track.start_index(t);
                        dec = None;
                        skip_until = t;
                    }
                    if st.frames.len() >= AHEAD || idx >= track.packets.len() {
                        let _ = cv.wait_timeout(st, std::time::Duration::from_millis(40)).unwrap();
                        continue;
                    }
                }
                if dec.is_none() {
                    match Dec::new(&track) {
                        Ok(d) => dec = Some(d),
                        Err(_) => return,
                    }
                }
                let pkt = &track.packets[idx];
                idx += 1;
                if let Some(img) = dec.as_mut().and_then(|d| d.decode(pkt)) {
                    // frames before a seek target are decoded (they are references) but not kept
                    let next = track.packets.get(idx).map(|p| p.pts).unwrap_or(f64::INFINITY);
                    if next <= skip_until {
                        continue;
                    }
                    let mut st = lock.lock().unwrap();
                    if st.seek.is_none() {
                        st.frames.push_back((pkt.pts, img));
                    }
                    drop(st);
                    if let Some(c) = &ctx {
                        c.request_repaint();
                    }
                }
            }
        });
        SoftVideo { shared, thread: Some(thread) }
    }

    pub fn seek(&self, t: f64) {
        let (lock, cv) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            st.seek = Some(t);
            st.frames.clear();
        }
        cv.notify_all();
    }

    /// The newest frame due at `t` (frames before it are dropped).
    pub fn frame_at(&self, t: f64) -> Option<Image> {
        let (lock, cv) = &*self.shared;
        let mut st = lock.lock().ok()?;
        let mut out = None;
        while st.frames.front().is_some_and(|f| f.0 <= t + 0.005) {
            out = st.frames.pop_front().map(|f| f.1);
        }
        drop(st);
        if out.is_some() {
            cv.notify_all();
        }
        out
    }
}

impl Drop for SoftVideo {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        if let Ok(mut st) = lock.lock() {
            st.quit = true;
        }
        cv.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// -------------------------------------------------------------------------------------------
// The player: sound through the system player, pictures from the thread above
// -------------------------------------------------------------------------------------------
struct Loaded {
    wav: Option<(Vec<u8>, f64)>,
    video: Option<Arc<VideoTrack>>,
    duration: f64,
}

pub struct SoftPlayer {
    loading: Option<Receiver<Result<Loaded, String>>>,
    failed: Option<String>,
    #[cfg(target_os = "macos")]
    audio: Option<crate::macos::Player>,
    video: Option<SoftVideo>,
    size: (f32, f32),
    duration: f64,
    // the clock when there is no sound track
    base: f64,
    since: Option<Instant>,
    muted: bool,
    ctx: egui::Context,
}

pub enum State {
    Loading,
    Ready,
    Failed(String),
}

impl SoftPlayer {
    pub fn open(bytes: Arc<Vec<u8>>, ctx: &egui::Context) -> SoftPlayer {
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<Loaded, String> {
                let (video, vdur) = demux_video(&bytes)?;
                let wav = decode_audio_wav(&bytes)?;
                if video.is_none() && wav.is_none() {
                    return Err("no sound or picture this app can play".into());
                }
                let duration = vdur.max(wav.as_ref().map(|w| w.1).unwrap_or(0.0));
                Ok(Loaded { wav, video: video.map(Arc::new), duration })
            })();
            let _ = tx.send(r);
            c.request_repaint();
        });
        SoftPlayer {
            loading: Some(rx), failed: None,
            #[cfg(target_os = "macos")]
            audio: None,
            video: None, size: (0.0, 0.0), duration: 0.0, base: 0.0, since: None, muted: false, ctx: ctx.clone(),
        }
    }

    /// Picks up the decoded file (on the UI thread: the system player is made here).
    pub fn poll(&mut self) {
        let Some(rx) = &self.loading else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.loading = None;
        match r {
            Ok(l) => {
                self.duration = l.duration;
                if let Some(v) = &l.video {
                    self.size = (v.w as f32, v.h as f32);
                    self.video = Some(SoftVideo::start(v.clone(), Some(self.ctx.clone())));
                }
                #[cfg(target_os = "macos")]
                if let Some((wav, _)) = l.wav {
                    match crate::macos::Player::open(Arc::new(wav), "sound.wav", "audio/wav", &self.ctx) {
                        Ok(p) => self.audio = Some(p),
                        Err(e) => self.failed = Some(e),
                    }
                }
            }
            Err(e) => self.failed = Some(e),
        }
    }

    pub fn state(&self) -> State {
        if let Some(e) = &self.failed {
            return State::Failed(e.clone());
        }
        if self.loading.is_some() {
            return State::Loading;
        }
        #[cfg(target_os = "macos")]
        if let Some(a) = &self.audio {
            return match a.state() {
                crate::macos::PlayerState::Ready => State::Ready,
                crate::macos::PlayerState::Loading => State::Loading,
                crate::macos::PlayerState::Failed(e) => State::Failed(e),
            };
        }
        State::Ready
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    pub fn time(&self) -> f64 {
        #[cfg(target_os = "macos")]
        if let Some(a) = &self.audio {
            return a.time();
        }
        let t = self.base + self.since.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        t.min(self.duration)
    }

    pub fn playing(&self) -> bool {
        #[cfg(target_os = "macos")]
        if let Some(a) = &self.audio {
            return a.playing();
        }
        self.since.is_some() && self.time() < self.duration
    }

    pub fn play(&mut self) {
        #[cfg(target_os = "macos")]
        if self.audio.is_some() {
            let at_end = self.audio.as_ref().is_some_and(|a| self.duration > 0.0 && a.time() >= self.duration - 0.05);
            if at_end {
                self.seek(0.0);
            }
            if let Some(a) = &self.audio {
                a.play();
            }
            return;
        }
        if self.time() >= self.duration - 0.01 {
            self.seek(0.0);
        }
        if self.since.is_none() {
            self.since = Some(Instant::now());
        }
    }

    pub fn pause(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(a) = &self.audio {
            a.pause();
            return;
        }
        self.base = self.time();
        self.since = None;
    }

    pub fn seek(&mut self, t: f64) {
        let t = t.clamp(0.0, self.duration.max(0.0));
        if let Some(v) = &self.video {
            v.seek(t);
        }
        #[cfg(target_os = "macos")]
        if let Some(a) = &self.audio {
            a.seek(t);
            return;
        }
        self.base = t;
        if self.since.is_some() {
            self.since = Some(Instant::now());
        }
    }

    pub fn muted(&self) -> bool {
        #[cfg(target_os = "macos")]
        if let Some(a) = &self.audio {
            return a.muted();
        }
        self.muted
    }

    pub fn set_muted(&mut self, m: bool) {
        #[cfg(target_os = "macos")]
        if let Some(a) = &self.audio {
            a.set_muted(m);
        }
        self.muted = m;
    }

    pub fn video_size(&self) -> (f32, f32) {
        self.size
    }

    pub fn frame(&self) -> Option<Image> {
        let v = self.video.as_ref()?;
        let img = v.frame_at(self.time());
        if self.playing() {
            self.ctx.request_repaint_after(std::time::Duration::from_millis(10));
        }
        img
    }
}

// -------------------------------------------------------------------------------------------
// Either player, behind one face
// -------------------------------------------------------------------------------------------
pub enum AnyPlayer {
    #[cfg(target_os = "macos")]
    System(crate::macos::Player),
    Soft(SoftPlayer),
}

impl AnyPlayer {
    /// WebM, Matroska and Ogg here; everything else to the system player.
    pub fn open(bytes: Arc<Vec<u8>>, name: &str, mime: &str, ctx: &egui::Context) -> Result<AnyPlayer, String> {
        if is_soft(&bytes) {
            return Ok(AnyPlayer::Soft(SoftPlayer::open(bytes, ctx)));
        }
        #[cfg(target_os = "macos")]
        return crate::macos::Player::open(bytes, name, mime, ctx).map(AnyPlayer::System);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (name, mime);
            Err("this format is not supported here".into())
        }
    }

    pub fn poll(&mut self) {
        if let AnyPlayer::Soft(p) = self {
            p.poll();
        }
    }

    pub fn state(&self) -> State {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => match p.state() {
                crate::macos::PlayerState::Ready => State::Ready,
                crate::macos::PlayerState::Loading => State::Loading,
                crate::macos::PlayerState::Failed(e) => State::Failed(e),
            },
            AnyPlayer::Soft(p) => p.state(),
        }
    }

    pub fn duration(&self) -> f64 {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.duration(),
            AnyPlayer::Soft(p) => p.duration(),
        }
    }

    pub fn time(&self) -> f64 {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.time(),
            AnyPlayer::Soft(p) => p.time(),
        }
    }

    pub fn playing(&self) -> bool {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.playing(),
            AnyPlayer::Soft(p) => p.playing(),
        }
    }

    pub fn play(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.play(),
            AnyPlayer::Soft(p) => p.play(),
        }
    }

    pub fn pause(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.pause(),
            AnyPlayer::Soft(p) => p.pause(),
        }
    }

    pub fn seek(&mut self, t: f64) {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.seek(t),
            AnyPlayer::Soft(p) => p.seek(t),
        }
    }

    pub fn muted(&self) -> bool {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.muted(),
            AnyPlayer::Soft(p) => p.muted(),
        }
    }

    pub fn set_muted(&mut self, m: bool) {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.set_muted(m),
            AnyPlayer::Soft(p) => p.set_muted(m),
        }
    }

    pub fn video_size(&self) -> (f32, f32) {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.video_size(),
            AnyPlayer::Soft(p) => p.video_size(),
        }
    }

    pub fn frame(&self) -> Option<Image> {
        match self {
            #[cfg(target_os = "macos")]
            AnyPlayer::System(p) => p.frame().map(|f| Image { w: f.w, h: f.h, rgba: f.rgba() }),
            AnyPlayer::Soft(p) => p.frame(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MVAULT_MEDIA_DIR: a folder of sample files (WebM VP8/VP9/AV1, Ogg Theora/Vorbis/Opus).
    #[test]
    #[ignore]
    fn decodes_samples() {
        let dir = std::env::var("MVAULT_MEDIA_DIR").expect("MVAULT_MEDIA_DIR");
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let bytes = Arc::new(std::fs::read(e.path()).unwrap());
            if !is_soft(&bytes) {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let (video, dur) = demux_video(&bytes).unwrap_or_else(|err| panic!("{name}: {err}"));
            let wav = decode_audio_wav(&bytes).unwrap_or_else(|err| panic!("{name}: {err}"));
            let mut shown = 0;
            let mut first = None;
            if let Some(v) = &video {
                let mut d = Dec::new(v).unwrap();
                for p in v.packets.iter().take(40) {
                    if let Some(img) = d.decode(p) {
                        shown += 1;
                        first.get_or_insert((img.w, img.h, img.rgba[..4].to_vec()));
                    }
                }
            }
            println!("{name}: video {:?} {} packets, {shown} of the first 40 shown {:?}; sound {:?}s; duration {dur:.2}s",
                video.as_ref().map(|v| (v.codec, v.w, v.h)), video.as_ref().map(|v| v.packets.len()).unwrap_or(0), first.map(|f| (f.0, f.1)),
                wav.as_ref().map(|w| (w.1 * 100.0).round() / 100.0));
            assert!(video.is_some() || wav.is_some(), "{name}: nothing decoded");
            if video.is_some() {
                assert!(shown > 0, "{name}: no pictures");
            }
        }
    }
}
