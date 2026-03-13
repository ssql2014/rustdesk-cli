//! Screenshot capture helpers over an encrypted RustDesk session.

use std::{
    io::Cursor,
    fs,
    io::{self, Write},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use image::{DynamicImage, GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder};
use prost::Message as ProstMessage;

use crate::crypto::EncryptedStream;
use crate::proto::hbb::{Message, ScreenshotRequest, message};
use crate::session::CaptureRegion;
use crate::transport::Transport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOptions {
    pub format: Option<String>,
    pub quality: Option<u8>,
    pub region: Option<CaptureRegion>,
    pub display: Option<i32>,
}

/// Request a screenshot from display 0 and return the raw image bytes.
pub async fn request_screenshot<T: Transport>(
    stream: &mut EncryptedStream<T>,
    options: &CaptureOptions,
) -> Result<Vec<u8>> {
    let sid = format!(
        "rustdesk-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    request_screenshot_with_sid(stream, options.display.unwrap_or(0), &sid, options).await
}

/// Request a screenshot for the specified display and session id.
pub async fn request_screenshot_with_sid<T: Transport>(
    stream: &mut EncryptedStream<T>,
    display: i32,
    sid: &str,
    options: &CaptureOptions,
) -> Result<Vec<u8>> {
    let msg = Message {
        union: Some(message::Union::ScreenshotRequest(ScreenshotRequest {
            display,
            sid: sid.to_string(),
        })),
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf)?;
    stream.send(&buf).await.context("sending ScreenshotRequest")?;

    loop {
        let raw = stream.recv().await.context("waiting for ScreenshotResponse")?;
        let msg = Message::decode(raw.as_slice()).context("decoding screenshot Message")?;

        match msg.union {
            Some(message::Union::ScreenshotResponse(response)) => {
                if response.sid != sid {
                    continue;
                }
                if !response.msg.is_empty() {
                    bail!("screenshot failed: {}", response.msg);
                }
                return process_screenshot_bytes(&response.data, options);
            }
            _ => continue,
        }
    }
}

pub fn process_screenshot_bytes(bytes: &[u8], options: &CaptureOptions) -> Result<Vec<u8>> {
    let needs_processing = options.region.is_some()
        || options
            .format
            .as_deref()
            .is_some_and(|format| !format.eq_ignore_ascii_case("png"))
        || options.quality.is_some();

    if !needs_processing {
        return Ok(bytes.to_vec());
    }

    let mut image = image::load_from_memory(bytes).context("decoding screenshot image")?;
    if let Some(region) = &options.region {
        image = crop_image(image, region)?;
    }

    encode_image(&image, options)
}

fn crop_image(image: DynamicImage, region: &CaptureRegion) -> Result<DynamicImage> {
    let (width, height) = image.dimensions();
    let x2 = region
        .x
        .checked_add(region.w)
        .context("capture region x overflow")?;
    let y2 = region
        .y
        .checked_add(region.h)
        .context("capture region y overflow")?;
    if x2 > width || y2 > height {
        bail!(
            "capture region {},{},{},{} exceeds image bounds {}x{}",
            region.x,
            region.y,
            region.w,
            region.h,
            width,
            height
        );
    }

    Ok(image.crop_imm(region.x, region.y, region.w, region.h))
}

fn encode_image(image: &DynamicImage, options: &CaptureOptions) -> Result<Vec<u8>> {
    let format = options.format.as_deref().unwrap_or("png");
    let mut out = Cursor::new(Vec::new());

    if format.eq_ignore_ascii_case("jpg") || format.eq_ignore_ascii_case("jpeg") {
        let quality = options.quality.unwrap_or(90);
        let mut encoder = JpegEncoder::new_with_quality(&mut out, quality);
        encoder
            .encode_image(image)
            .context("encoding screenshot as JPEG")?;
    } else {
        image
            .write_to(&mut out, ImageFormat::Png)
            .context("encoding screenshot as PNG")?;
    }

    Ok(out.into_inner())
}

/// Save image bytes to a file, or to stdout if no file path is given.
pub fn write_capture_output(bytes: &[u8], file: Option<&str>) -> Result<()> {
    if let Some(file) = file {
        fs::write(file, bytes).with_context(|| format!("writing screenshot to {file}"))?;
        return Ok(());
    }

    let mut stdout = io::stdout();
    stdout
        .write_all(bytes)
        .and_then(|_| stdout.flush())
        .context("writing screenshot to stdout")?;
    Ok(())
}

/// Minimal base64 encoder for passing bytes through JSON.
pub fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if chunk.len() > 1 {
            out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }

        if chunk.len() > 2 {
            out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Minimal base64 decoder for daemon/CLI screenshot handoff.
pub fn base64_decode(input: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Result<u8> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => bail!("invalid base64 character: {}", c as char),
        }
    }

    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        bail!("invalid base64 length");
    }

    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&b| b == b'=').count();
        let c0 = val(chunk[0])?;
        let c1 = val(chunk[1])?;
        let c2 = if chunk[2] == b'=' { 0 } else { val(chunk[2])? };
        let c3 = if chunk[3] == b'=' { 0 } else { val(chunk[3])? };

        let n = ((c0 as u32) << 18)
            | ((c1 as u32) << 12)
            | ((c2 as u32) << 6)
            | (c3 as u32);
        out.push(((n >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((n & 0xff) as u8);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::EncryptedStream;
    use crate::transport::{FramedTransport, Transport};
    use tokio::io::duplex;

    struct DuplexTransport {
        framed: FramedTransport<tokio::io::DuplexStream>,
    }

    impl DuplexTransport {
        fn pair() -> (Self, Self) {
            let (a, b) = duplex(8192);
            (
                Self {
                    framed: FramedTransport::new(a),
                },
                Self {
                    framed: FramedTransport::new(b),
                },
            )
        }
    }

    impl Transport for DuplexTransport {
        async fn connect(_addr: &str) -> Result<Self> {
            unimplemented!()
        }
        async fn send(&mut self, msg: &[u8]) -> Result<()> {
            self.framed.send(msg).await
        }
        async fn recv(&mut self) -> Result<Vec<u8>> {
            self.framed.recv().await
        }
        async fn close(&mut self) -> Result<()> {
            self.framed.close().await
        }
    }

    #[tokio::test]
    async fn request_screenshot_sends_request_and_returns_response_bytes() -> Result<()> {
        let (client_transport, server_transport) = DuplexTransport::pair();
        let session_key = [7_u8; 32];
        let mut client = EncryptedStream::new(client_transport, &session_key);
        let mut server = EncryptedStream::new(server_transport, &session_key);

        let server_task = tokio::spawn(async move {
            let raw = server.recv().await?;
            let msg = Message::decode(raw.as_slice())?;

            match msg.union {
                Some(message::Union::ScreenshotRequest(request)) => {
                    assert_eq!(request.display, 0);
                    assert_eq!(request.sid, "sid-1");
                }
                other => panic!("expected ScreenshotRequest, got {other:?}"),
            }

            let reply = Message {
                union: Some(message::Union::ScreenshotResponse(
                    crate::proto::hbb::ScreenshotResponse {
                        sid: "sid-1".to_string(),
                        msg: String::new(),
                        data: b"png-bytes".to_vec(),
                    },
                )),
            };
            let mut buf = Vec::new();
            reply.encode(&mut buf)?;
            server.send(&buf).await?;
            Result::<()>::Ok(())
        });

        let bytes = request_screenshot_with_sid(
            &mut client,
            0,
            "sid-1",
            &CaptureOptions {
                format: None,
                quality: None,
                region: None,
                display: Some(0),
            },
        )
        .await?;
        assert_eq!(bytes, b"png-bytes");
        server_task.await.expect("server task should join")?;
        Ok(())
    }

    #[test]
    fn process_screenshot_bytes_crops_png_region() -> Result<()> {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 3, |x, y| {
            image::Rgba([x as u8, y as u8, 0, 255])
        }));
        let mut encoded = Cursor::new(Vec::new());
        image.write_to(&mut encoded, ImageFormat::Png)?;

        let processed = process_screenshot_bytes(
            &encoded.into_inner(),
            &CaptureOptions {
                format: Some("png".to_string()),
                quality: Some(80),
                region: Some(CaptureRegion {
                    x: 1,
                    y: 1,
                    w: 2,
                    h: 1,
                }),
                display: Some(2),
            },
        )?;

        let cropped = image::load_from_memory(&processed)?;
        assert_eq!(cropped.dimensions(), (2, 1));
        Ok(())
    }

    #[test]
    fn process_screenshot_bytes_reencodes_as_jpeg() -> Result<()> {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([10, 20, 30, 255]),
        ));
        let mut encoded = Cursor::new(Vec::new());
        image.write_to(&mut encoded, ImageFormat::Png)?;

        let processed = process_screenshot_bytes(
            &encoded.into_inner(),
            &CaptureOptions {
                format: Some("jpg".to_string()),
                quality: Some(70),
                region: None,
                display: None,
            },
        )?;

        assert!(processed.starts_with(&[0xFF, 0xD8, 0xFF]));
        Ok(())
    }

    #[test]
    fn base64_roundtrip() -> Result<()> {
        let data = b"\x89PNG\r\n\x1a\n";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded)?;
        assert_eq!(decoded, data);
        Ok(())
    }
}
