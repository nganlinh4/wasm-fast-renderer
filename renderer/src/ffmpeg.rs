use crate::types::{Design, Details, TrackItem, TrackType};
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
        let start = it.trim.from.unwrap_or(0);
        let end = it.trim.to.unwrap_or(0);
        if end > max_end { max_end = end; }
        // In case only width/height video with no trim provided, ignore.
    }
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
) -> Result<BuiltCommand> {
    fs::create_dir_all(workdir).ok();
    let out_path = workdir.join("output.mp4");
    let mut args: Vec<String> = vec!["-y".into(), "-hide_banner".into(), "-loglevel".into(), "error".into()];
    if caps.nvenc { args.extend(["-hwaccel".into(), "cuda".into()]); }

    // Inputs
    for (_, item, path) in assets {
        match item.kind {
            TrackType::Image => {
                // images need looping to video duration; loop via -loop 1 for input
                args.extend(["-loop".into(), "1".into()]);
                args.extend(["-t".into(), format!("{}", compute_duration_ms(design) as f32 / 1000.0)]);
            }
            _ => {}
        }
        args.extend(["-i".into(), path.to_string_lossy().to_string()]);
    }

    // Build filter graph
    // Label each input video/image as v{i}, audio as a{i}
    let mut filter_parts: Vec<String> = Vec::new();
    let mut video_labels: Vec<String> = Vec::new();
    let mut audio_labels: Vec<String> = Vec::new();

    for (idx, item, _) in assets {
        match item.kind {
            TrackType::Video | TrackType::Image => {
                let mut chain = format!("[{}:v]", idx);
                // scale
                let (w, h) = (
                    item.details.as_ref().and_then(|d| d.width).unwrap_or_else(|| design.size.as_ref().map(|s| s.width).unwrap_or(1080)),
                    item.details.as_ref().and_then(|d| d.height).unwrap_or_else(|| design.size.as_ref().map(|s| s.height).unwrap_or(1920)),
                );
                let scale = parse_scale(&item.details.as_ref().and_then(|d| d.transform.clone()));
                let (sw, sh) = (((w as f32) * scale) as i32, ((h as f32) * scale) as i32);
                chain.push_str(&format!("scale={}:{},format=rgba", sw.max(1), sh.max(1)));
                // brightness (eq)
                let b = brightness_offset(item.details.as_ref().and_then(|d| d.brightness));
                if b.abs() > 0.001 { chain.push_str(&format!(",eq=brightness={}", b)); }
                // opacity
                let a = opacity_alpha(item.details.as_ref().and_then(|d| d.opacity));
                if a < 0.999 { chain.push_str(&format!(",colorchannelmixer=aa={}", a)); }
                let vlabel = format!("v{}", idx);
                chain.push_str(&format!("[{}]", vlabel));
                filter_parts.push(chain);
                video_labels.push(vlabel);
            }
            TrackType::Audio => {
                let alabel = format!("a{}", idx);
                let vol = item.details.as_ref().and_then(|d| d.volume).unwrap_or(100.0) / 100.0;
                filter_parts.push(format!("[{}:a]volume={}[{}]", idx, vol, alabel));
                audio_labels.push(alabel);
            }
            _ => {}
        }
    }

    // Compose overlays: take first video as base; overlay others by left/top
    if video_labels.is_empty() { return Err(anyhow!("No video/image tracks provided")); }
    let mut last = video_labels[0].clone();
    for (i, (idx, item, _)) in assets.iter().enumerate() {
        if i == 0 { continue; }
        match item.kind {
            TrackType::Video | TrackType::Image => {
                let x = parse_px(&item.details.as_ref().and_then(|d| d.left.clone()));
                let y = parse_px(&item.details.as_ref().and_then(|d| d.top.clone()));
                let cur = video_labels.iter().find(|l| *l == &format!("v{}", idx)).unwrap().clone();
                let out = format!("m{}", i);
                // CPU overlay for broad compatibility; can switch to overlay_cuda later
                filter_parts.push(format!("[{}][{}]overlay={}:{}[{}]", last, cur, x, y, out));
                last = out;
            }
            _ => {}
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
            let inputs = audio_labels.join("");
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

