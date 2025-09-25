use crate::types::{Design, TrackItem, TrackType};
use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Clone, Debug)]
pub struct BackendCaps {
    pub nvenc: bool,
}

pub async fn detect_caps() -> BackendCaps {
    // Try to detect h264_nvenc support
    let output = Command::new("ffmpeg").arg("-hide_banner").arg("-encoders").output().await;
    let nvenc = output.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.contains("h264_nvenc"))
        .unwrap_or(false);
    BackendCaps { nvenc }
}

pub async fn download_asset(url: &str, dest_dir: &Path) -> Result<PathBuf> {
    let resp = reqwest::get(url).await.context("download request failed")?;
    if !resp.status().is_success() { return Err(anyhow!("bad status {}", resp.status())); }
    let bytes_stream = resp.bytes_stream();
    let mut hasher = Sha256::new();
    let parsed = url::Url::parse(url).ok();
    let filename = parsed
        .as_ref()
        .and_then(|u| u.path_segments().and_then(|mut s| s.next_back().map(|x| x.to_string())))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "asset.bin".to_string());
    let mut out_path = dest_dir.join(&filename);
    // ensure unique name
    let mut idx = 0;
    while out_path.exists() { idx += 1; out_path = dest_dir.join(format!("{}-{}", idx, filename)); }

    let mut file = tokio::fs::File::create(&out_path).await?;
    use futures_util::StreamExt;
    futures_util::pin_mut!(bytes_stream);
    while let Some(chunk) = bytes_stream.next().await { let b = chunk?; hasher.update(&b); file.write_all(&b).await?; }
    file.flush().await?;

    // rename to include hash prefix for stability
    let hash = hex::encode(hasher.finalize());
    let new_name = format!("{}-{}", &hash[..16], out_path.file_name().unwrap().to_string_lossy());
    let new_path = out_path.with_file_name(new_name);
    tokio::fs::rename(&out_path, &new_path).await?;
    Ok(new_path)
}

pub fn compute_duration_ms(design: &Design) -> u64 {
    let mut max_end = 0u64;
    let items: Vec<&TrackItem> = if !design.trackItems.is_empty() {
        design.trackItems.iter().collect()
    } else {
        design.trackItemsMap.values().collect()
    };
    for it in items {
        let trim_end = it.trim.to.unwrap_or(0);
        if trim_end > max_end { max_end = trim_end; }
        // Consider display window if provided
        let disp_end = it.display.to.unwrap_or(0);
        if disp_end > max_end { max_end = disp_end; }
    }
    // Fallback minimal duration
    if max_end == 0 { 10_000 } else { max_end }
}

fn parse_px(s: &Option<String>) -> i32 { s.as_ref().and_then(|v| v.strip_suffix("px")).and_then(|p| p.parse::<f32>().ok()).map(|f| f.round() as i32).unwrap_or(0) }
fn parse_scale(s: &Option<String>) -> f32 {
    if let Some(t) = s { if let Some(start) = t.find("scale(") { if let Some(end) = t[start+6..].find(')') { return t[start+6..start+6+end].parse::<f32>().unwrap_or(1.0); } } }
    1.0
}
fn opacity_alpha(o: Option<f32>) -> f32 { let v = o.unwrap_or(100.0) / 100.0; v.clamp(0.0, 1.0) }
fn brightness_offset(b: Option<f32>) -> f32 { (b.unwrap_or(100.0) - 100.0) / 100.0 }

pub struct BuiltCommand {
    pub args: Vec<String>,
}

pub fn build_ffmpeg_command(
    workdir: &Path,
    design: &Design,
    assets: &[(usize, &TrackItem, PathBuf)],
    caps: &BackendCaps,
    font_map: &std::collections::HashMap<String, PathBuf>,
) -> Result<BuiltCommand> {
    fs::create_dir_all(workdir).ok();
    let out_path = workdir.join("output.mp4");
    let mut args: Vec<String> = vec!["-y".into(), "-hide_banner".into(), "-loglevel".into(), "error".into()];
    if caps.nvenc { args.extend(["-hwaccel".into(), "cuda".into()]); }
    let fps = design.fps.unwrap_or(30);
    let (out_w, out_h) = (
        design.size.as_ref().map(|s| s.width).unwrap_or(1080),
        design.size.as_ref().map(|s| s.height).unwrap_or(1920),
    );
    let duration_ms = compute_duration_ms(design);
    let duration_s = (duration_ms as f64) / 1000.0;

    // Base black canvas as input 0
    args.extend([
        "-f".into(), "lavfi".into(),
        "-i".into(), format!("color=c=black:s={}x{}:r={}:d={}", out_w, out_h, fps, duration_s),
    ]);


    // Asset inputs start from index 1 (index 0 is the base canvas)
    for (_, item, path) in assets {
        match item.kind {
            TrackType::Image => {
                // images need looping to act as a video stream; visibility is gated in filters
                args.extend(["-loop".into(), "1".into()]);
                // ensure finite duration to avoid infinite streams that stall the graph
                args.extend(["-t".into(), format!("{:.3}", duration_s)]);
            }
            _ => {}
        }
        args.extend(["-i".into(), path.to_string_lossy().to_string()]);
    }

    // Build filter graph
    // Label each input video/image as v{i}, audio as a{i}
    let mut filter_parts: Vec<String> = Vec::new();
    let mut audio_labels: Vec<String> = Vec::new();

    // Start from base canvas as the initial video
    let mut last = String::from("0:v");

    for (idx0, item, _) in assets {
        match item.kind {
            TrackType::Video | TrackType::Image => {
                let ff_idx = idx0 + 1; // account for base canvas at 0
                let mut chain = format!("[{}:v]format=rgba", ff_idx);
                // scale
                let (w, h) = (
                    item.details.as_ref().and_then(|d| d.width).unwrap_or_else(|| design.size.as_ref().map(|s| s.width).unwrap_or(1080)),
                    item.details.as_ref().and_then(|d| d.height).unwrap_or_else(|| design.size.as_ref().map(|s| s.height).unwrap_or(1920)),
                );
                let scale = parse_scale(&item.details.as_ref().and_then(|d| d.transform.clone()));
                let (sw, sh) = (((w as f32) * scale) as i32, ((h as f32) * scale) as i32);
                chain.push_str(&format!(",scale={}:{}", sw.max(1), sh.max(1)));
                // rotate (degrees to radians)
                if let Some(rot) = item.details.as_ref().and_then(|d| d.rotate.clone()) {
                    if let Ok(deg) = rot.trim().trim_end_matches("deg").parse::<f32>() { if deg.abs() > 0.01 { chain.push_str(&format!(",rotate={:.6}*PI/180", deg)); } }
                }
                // brightness (eq)
                let b = brightness_offset(item.details.as_ref().and_then(|d| d.brightness));
                if b.abs() > 0.001 { chain.push_str(&format!(",eq=brightness={}", b)); }
                // opacity
                let a = opacity_alpha(item.details.as_ref().and_then(|d| d.opacity));
                if a < 0.999 { chain.push_str(&format!(",colorchannelmixer=aa={}", a)); }
                let vlabel = format!("v{}", ff_idx);
                chain.push_str(&format!("[{}]", vlabel));
                filter_parts.push(chain);

                // overlay onto last with timing window
                let x = parse_px(&item.details.as_ref().and_then(|d| d.left.clone()));
                let y = parse_px(&item.details.as_ref().and_then(|d| d.top.clone()));
                let start = item.display.from.or(item.trim.from).unwrap_or(0) as f64 / 1000.0;
                let end_ms = item.display.to.or(item.trim.to).unwrap_or(duration_ms);
                let end = (end_ms as f64) / 1000.0;
                let out = format!("m{}", ff_idx);
                filter_parts.push(format!("[{}][{}]overlay={}:{}:format=auto:enable='between(t,{:.3},{:.3})'[{}]", last, vlabel, x, y, start, end, out));
                last = out;
            }
            TrackType::Audio => {
                // Will be handled in audio mixing section, collect labels then
                let ff_idx = idx0 + 1;
                let vol = item.details.as_ref().and_then(|d| d.volume).unwrap_or(100.0) / 100.0;
                let start_ms = item.display.from.or(item.trim.from).unwrap_or(0);
                let mut alabel = format!("a{}", ff_idx);
                let mut chain = format!("[{}:a]volume={}", ff_idx, vol);
                if let Some(from) = item.trim.from { chain.push_str(&format!(",atrim=start={:.3}", (from as f64)/1000.0)); chain.push_str(",asetpts=PTS-STARTPTS"); }
                chain.push_str(&format!(",adelay={}:all=1", start_ms));
                chain.push_str(&format!(",atrim=0:{:.3},asetpts=PTS-STARTPTS[{}]", duration_ms as f64 / 1000.0, alabel));
                filter_parts.push(chain);
                audio_labels.push(alabel);
            }
            _ => {}
        }
    }

    // Text overlays
    let items_all: Vec<&TrackItem> = if !design.trackItems.is_empty() { design.trackItems.iter().collect() } else { design.trackItemsMap.values().collect() };
    for it in items_all {
        if let TrackType::Text = it.kind {
            if let Some(id) = &it.id {
                if let Some(font_path) = font_map.get(id) {
                    let mut text = it.details.as_ref().and_then(|d| d.text.clone()).unwrap_or_default();
                    text = text.replace("\\", "\\\\").replace("'", "\\'").replace(":", "\\:");
                    let fontsize = it.details.as_ref().and_then(|d| d.fontSize).unwrap_or(48);
                    let x = parse_px(&it.details.as_ref().and_then(|d| d.left.clone()));
                    let y = parse_px(&it.details.as_ref().and_then(|d| d.top.clone()));
                    let alpha = opacity_alpha(it.details.as_ref().and_then(|d| d.opacity));
                    let color = it.details.as_ref().and_then(|d| d.color.clone()).unwrap_or("white".into());
                    let fontcolor = if color.starts_with('#') { format!("0x{}@{}", color.trim_start_matches('#'), alpha) } else { format!("{}@{}", color, alpha) };
                    let borderw = it.details.as_ref().and_then(|d| d.borderWidth).unwrap_or(0);
                    let bordercolor = it.details.as_ref().and_then(|d| d.borderColor.clone()).unwrap_or("black".into());
                    let bordercolor = if bordercolor.starts_with('#') { format!("0x{}", bordercolor.trim_start_matches('#')) } else { bordercolor };
                    let start = it.display.from.unwrap_or(0) as f64 / 1000.0;
                    let end = it.display.to.unwrap_or(duration_ms) as f64 / 1000.0;
                    let out = format!("txt{}", id);
                    filter_parts.push(format!(
                        "[{}]drawtext=fontfile={}:text='{}':fontsize={}:fontcolor={}:borderw={}:bordercolor={}:x={}:y={}:enable='between(t,{:.3},{:.3})'[{}]",
                        last, font_path.to_string_lossy(), text, fontsize, fontcolor, borderw, bordercolor, x, y, start, end, out
                    ));
                    last = out;
                }
            }
        }
    }

    // Audio mix (optional)
    let mut maps: Vec<(String, String)> = Vec::new();
    let vout = last;
    maps.push((vout.clone(), "v".into()));

    if !audio_labels.is_empty() {
        if audio_labels.len() == 1 {
            filter_parts.push(format!("[{}]anull[aout]", audio_labels[0]));
        } else {
            let list = audio_labels.iter().map(|l| format!("[{}]", l)).collect::<String>();
            filter_parts.push(format!("{}amix=inputs={}:normalize=0[aout]", list, audio_labels.len()));
        }
        maps.push(("aout".into(), "a".into()));
    }

    let filter_complex = filter_parts.join(";");
    if !filter_complex.is_empty() { args.extend(["-filter_complex".into(), filter_complex]); }

    // Map outputs
    let mut mapped_audio = false;
    for (src, kind) in &maps {
        args.extend(["-map".into(), format!("[{}]", src)]);
        if kind == "v" {
            if caps.nvenc { args.extend(["-c:v".into(), "h264_nvenc".into(), "-preset".into(), "p4".into()]); }
            else { args.extend(["-c:v".into(), "libx264".into(), "-preset".into(), "veryfast".into()]); }
            args.extend(["-pix_fmt".into(), "yuv420p".into()]);
            // Honor desired fps from design/options
            args.extend(["-r".into(), fps.to_string()]);
        } else if kind == "a" {
            mapped_audio = true;
            args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
        }
    }
    // If no explicit audio items, attempt to map base input's audio if present
    if !mapped_audio {
        args.extend(["-map".into(), "0:a?".into(), "-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    }

    args.extend(["-progress".into(), "pipe:1".into()]);
    args.push(out_path.to_string_lossy().to_string());

    Ok(BuiltCommand { args })
}

