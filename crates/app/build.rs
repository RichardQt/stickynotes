fn main() {
    println!("cargo:rerun-if-changed=assets/logo.svg");

    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_icon::embed();
    }
}

#[cfg(windows)]
mod windows_icon {
    use resvg::{tiny_skia, usvg};
    use std::path::PathBuf;

    const ICON_SIZES: &[u32] = &[16, 20, 24, 32, 40, 48, 64, 128, 256];

    pub fn embed() {
        let manifest_dir =
            PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

        let svg_bytes =
            std::fs::read(manifest_dir.join("assets/logo.svg")).expect("read assets/logo.svg");
        let tree =
            usvg::Tree::from_data(&svg_bytes, &usvg::Options::default()).expect("parse logo.svg");

        let mut frames: Vec<(u32, Vec<u8>)> = Vec::with_capacity(ICON_SIZES.len());
        for &size in ICON_SIZES {
            let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("allocate icon pixmap");
            let sx = size as f32 / tree.size().width();
            let sy = size as f32 / tree.size().height();
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(sx, sy),
                &mut pixmap.as_mut(),
            );

            // 256×256 uses PNG compression (standard Vista+ jumbo icon format);
            // smaller sizes use 32-bit BGRA DIB + 1bpp AND mask so every Win32
            // shell surface and `LoadImageW` scale renders crisp edges.
            let payload = if size >= 256 {
                pixmap.encode_png().expect("encode 256px PNG icon frame")
            } else {
                encode_bgra_dib(&pixmap)
            };
            frames.push((size, payload));
        }

        let ico_bytes = build_ico(&frames);
        let ico_path = out_dir.join("stickynotes.ico");
        std::fs::write(&ico_path, ico_bytes).expect("write stickynotes.ico");

        // Resource ID `1` is picked up both by Windows Explorer as the primary
        // executable icon and by GPUI's `load_icon()` (`MAKEINTRESOURCEW(1)`)
        // for the window class / taskbar / Alt+Tab icon.
        let escaped_ico = ico_path
            .to_str()
            .expect("UTF-8 OUT_DIR path")
            .replace('\\', "/");
        let rc_path = out_dir.join("stickynotes.rc");
        std::fs::write(&rc_path, format!("1 ICON \"{escaped_ico}\"\n"))
            .expect("write stickynotes.rc");

        embed_resource::compile(&rc_path, embed_resource::NONE)
            .manifest_required()
            .expect("compile Windows icon resource");
    }

    fn encode_bgra_dib(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
        let w = pixmap.width();
        let h = pixmap.height();
        let xor_len = (w * h * 4) as usize;
        let and_stride = (w as usize).div_ceil(32) * 4;
        let and_len = and_stride * h as usize;

        let mut buf = Vec::with_capacity(40 + xor_len + and_len);
        // BITMAPINFOHEADER (40 bytes), biHeight doubled for XOR + AND masks.
        buf.extend_from_slice(&40u32.to_le_bytes());
        buf.extend_from_slice(&(w as i32).to_le_bytes());
        buf.extend_from_slice(&((h as i32) * 2).to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&32u16.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&((xor_len + and_len) as u32).to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());

        // Bottom-up 32-bit straight BGRA pixels.
        for y in (0..h).rev() {
            for x in 0..w {
                let p = pixmap.pixel(x, y).expect("pixel in bounds").demultiply();
                buf.extend_from_slice(&[p.blue(), p.green(), p.red(), p.alpha()]);
            }
        }

        // Bottom-up 1bpp AND transparency mask (DWORD-padded rows).
        for y in (0..h).rev() {
            let row_start = buf.len();
            buf.resize(row_start + and_stride, 0);
            for x in 0..w {
                if pixmap.pixel(x, y).expect("pixel in bounds").alpha() == 0 {
                    buf[row_start + (x as usize / 8)] |= 1 << (7 - (x % 8));
                }
            }
        }

        buf
    }

    fn build_ico(frames: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let count = frames.len() as u16;
        let dir_bytes = 6 + frames.len() * 16;
        let total_payload: usize = frames.iter().map(|(_, b)| b.len()).sum();
        let mut out = Vec::with_capacity(dir_bytes + total_payload);

        // ICONDIR header: reserved = 0, type = 1 (icon), count.
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());

        let mut offset = dir_bytes as u32;
        for (size, payload) in frames {
            let dim = if *size >= 256 { 0u8 } else { *size as u8 };
            let len = payload.len() as u32;
            out.push(dim);
            out.push(dim);
            out.push(0);
            out.push(0);
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            offset += len;
        }

        for (_, payload) in frames {
            out.extend_from_slice(payload);
        }

        out
    }
}
