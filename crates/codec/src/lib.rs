//! Video encoding and decoding (H.264 via OpenH264 for the MVP,
//! extensible to Windows Media Foundation hardware encoders in Phase 4).

use openh264::decoder::Decoder;
use openh264::encoder::{Encoder, EncoderConfig, FrameType, UsageType};
use openh264::formats::{YUVBuffer, YUVSource};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("OpenH264 error: {0}")]
    OpenH264(#[from] openh264::Error),
    #[error("invalid frame dimensions: {0}x{1}")]
    InvalidDimensions(u32, u32),
}

pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Low-latency H.264 encoder backed by OpenH264.
pub struct H264Encoder {
    encoder: Encoder,
    width: usize,
    height: usize,
    yuv_buf: Vec<u8>,
}

impl H264Encoder {
    pub fn new(bitrate_bps: u32, max_fps: f32) -> Result<Self, CodecError> {
        let cfg = EncoderConfig::new()
            .set_bitrate_bps(bitrate_bps)
            .max_frame_rate(max_fps)
            .usage_type(UsageType::ScreenContentRealTime);
        let encoder = Encoder::with_api_config(openh264::OpenH264API::from_source(), cfg)?;
        Ok(Self {
            encoder,
            width: 0,
            height: 0,
            yuv_buf: Vec::new(),
        })
    }

    /// Encodes a `BGRA8` frame into an Annex-B H.264 bitstream.
    /// Returns `(bitstream, is_keyframe)`.
    pub fn encode_bgra(
        &mut self,
        width: u32,
        height: u32,
        bgra: &[u8],
        force_keyframe: bool,
    ) -> Result<(Vec<u8>, bool), CodecError> {
        let w = (width as usize) & !1;
        let h = (height as usize) & !1;
        if w < 16 || h < 16 || bgra.len() < (width as usize) * (height as usize) * 4 {
            return Err(CodecError::InvalidDimensions(width, height));
        }

        if force_keyframe {
            self.encoder.force_intra_frame();
        }

        self.width = w;
        self.height = h;
        bgra_to_i420(bgra, width as usize, w, h, &mut self.yuv_buf);

        let yuv = YUVBuffer::from_vec(self.yuv_buf.clone(), w, h);
        let bitstream = self.encoder.encode(&yuv)?;
        let is_keyframe = matches!(bitstream.frame_type(), FrameType::IDR | FrameType::I);

        Ok((bitstream.to_vec(), is_keyframe))
    }
}

/// H.264 decoder backed by OpenH264.
pub struct H264Decoder {
    decoder: Decoder,
}

impl H264Decoder {
    pub fn new() -> Result<Self, CodecError> {
        let decoder = Decoder::new()?;
        Ok(Self { decoder })
    }

    /// Decodes an Annex-B H.264 packet into an `RGBA8` frame if a picture is ready.
    pub fn decode_rgba(&mut self, annex_b: &[u8]) -> Result<Option<DecodedFrame>, CodecError> {
        let Some(yuv) = self.decoder.decode(annex_b)? else {
            return Ok(None);
        };
        let (w, h) = yuv.dimensions();
        let mut rgba = vec![0u8; w * h * 4];
        yuv.write_rgba8(&mut rgba);
        Ok(Some(DecodedFrame {
            width: w as u32,
            height: h as u32,
            rgba,
        }))
    }
}

/// Converts `BGRA8` into packed planar `I420` (`Y` + `U` + `V`) for even `w x h`.
fn bgra_to_i420(bgra: &[u8], src_w: usize, w: usize, h: usize, out: &mut Vec<u8>) {
    let y_size = w * h;
    let uv_size = (w / 2) * (h / 2);
    out.resize(y_size + uv_size * 2, 0);

    let (y_plane, uv_planes) = out.split_at_mut(y_size);
    let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

    for row in 0..h {
        let src_row = &bgra[row * src_w * 4..(row * src_w + w) * 4];
        let y_row = &mut y_plane[row * w..(row + 1) * w];

        for col in 0..w {
            let p = col * 4;
            let b = src_row[p] as i32;
            let g = src_row[p + 1] as i32;
            let r = src_row[p + 2] as i32;

            let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            y_row[col] = y.clamp(0, 255) as u8;

            if (row & 1) == 0 && (col & 1) == 0 {
                let uv_idx = (row / 2) * (w / 2) + (col / 2);
                let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                u_plane[uv_idx] = u.clamp(0, 255) as u8;
                v_plane[uv_idx] = v.clamp(0, 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip_synthetic_frame() {
        let w = 128u32;
        let h = 96u32;
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                bgra[idx] = (x * 2) as u8; // B
                bgra[idx + 1] = (y * 2) as u8; // G
                bgra[idx + 2] = 200; // R
                bgra[idx + 3] = 255; // A
            }
        }

        let mut enc = H264Encoder::new(1_000_000, 30.0).unwrap();
        let (pkt, is_key) = enc.encode_bgra(w, h, &bgra, true).unwrap();
        assert!(is_key);
        assert!(!pkt.is_empty());

        let mut dec = H264Decoder::new().unwrap();
        let decoded = dec.decode_rgba(&pkt).unwrap().unwrap();
        assert_eq!(decoded.width, w);
        assert_eq!(decoded.height, h);
        assert_eq!(decoded.rgba.len(), (w * h * 4) as usize);
    }
}
