# Rust GPU Video Rendering Service — Minimal Plan (MVP)

Goal: Replace the remote render API with a local Rust service that renders videos fast, using GPU acceleration where available, and works with the existing editor frontend immediately.

Primary target for MVP: Windows host with NVIDIA GPU (NVENC). Fallbacks: CPU (libx264) if GPU not detected. Later, add Intel QSV / AMD AMF / macOS VideoToolbox.

---

## 1) Architecture (simple, pragmatic)

- Next.js (current app)
  - POST /api/render → forwards to local Rust service
  - GET /api/render/:id → forwards to local Rust service
  - Unchanged polling in the UI store

- Rust render service (HTTP)
  - POST /render: submit design JSON + options → returns `{ jobId }`
  - GET /render/:id: status + progress → `{ status, progress, url? }`
  - GET /render/:id/output: serves the MP4 (or redirects)
  - Internals: in-memory job map (MVP), local temp workspace per job

- Rendering pipeline (MVP)
  - Asset fetcher: download all `src` URLs to a temp folder
  - FFmpeg command builder: map a supported subset of design JSON → filter_complex
  - GPU encode (h264_nvenc) if available; otherwise libx264
  - Progress parser using `-progress pipe:1` (percent from `out_time_ms`)

---

## 2) MVP feature coverage (kept small on purpose)

- Global
  - Output format: mp4
  - Size: width/height from design
  - FPS: from options or design

- Video/Image tracks
  - Timing: trim.from/to
  - Position: `left`/`top` (pixels)
  - Scale: from `transform: scale(x)`
  - Opacity: via alpha (simple constant alpha)
  - Z-order: item order in a track (front-to-back overlays)

- Audio tracks
  - Timing: atrim
  - Volume: from `details.volume`
  - Mix: `amix`

- Exclusions in MVP (add later)
  - Arbitrary angle rotation (beyond 90/180) → later
  - Complex transitions and advanced blurs → later
  - Text rendering → later (pre-rasterize images or SDF in phase 2)

---

## 3) API contracts

- POST /render
  - Request: `{ design: <current-editor-json>, options: { fps, size, format: "mp4" } }`
  - Response: `{ jobId: string }`

- GET /render/:id
  - Response: `{ status: "PENDING"|"RUNNING"|"COMPLETED"|"FAILED", progress: number, url?: string, error?: string }`

- GET /render/:id/output
  - Serves the rendered MP4 or 302-redirects to storage

These match the current frontend’s expectation with minimal change.

---

## 4) Rust service implementation outline

Crates: `axum`, `tokio`, `serde`, `serde_json`, `anyhow`, `tracing`, `reqwest` (asset fetch), `tempfile`, `uuid`.

- Capability detection (MVP)
  - At startup, run `ffmpeg -hide_banner -encoders` and detect `h264_nvenc`
  - Select encoder: `h264_nvenc` else `libx264`

- Job model
  - `Job { id, status, progress, output_path, error, created_at }`
  - In-memory `HashMap<Uuid, Job>` protected by `RwLock`

- Asset fetch
  - For each track `details.src` (http/https), download to `job_dir/assets/<hash>.ext`
  - Simple retry/backoff

- FFmpeg command builder (MVP subset)
  - Inputs: `-i <assetX>` per clip
  - Filter graph per video/image overlay:
    - scale (GPU if available), position with `overlay`
    - opacity via `format=rgba, colorchannelmixer=aa=...`
  - Audio: `atrim`, `volume`, `amix`
  - Encoder: `-c:v h264_nvenc -preset p4` (or `-c:v libx264 -preset veryfast`)
  - Progress: `-progress pipe:1` → parse `out_time_ms`

- Worker
  - Spawn `ffmpeg` with assembled args
  - Parse progress, update `Job`
  - On success: mark `COMPLETED`, store MP4 at `job_dir/output.mp4`
  - On error: capture last stderr, mark `FAILED`

---

## 5) Integration steps in this repo

1. Create `renderer/` Rust project (separate folder) with axum HTTP service
2. Implement endpoints and a dummy transcode path to validate end-to-end
3. Implement capability detection + choose encoder
4. Implement asset fetch + minimal command builder for: one background + one overlay + audio
5. Expand builder for multiple overlays and basic opacity/scale/position
6. Change Next.js `/api/render` routes to call `http://127.0.0.1:<PORT>/render` (POST) and `/render/:id` (GET)
7. Manual sanity test with a small design JSON (use `sample.json` as reference)

---

## 6) Command examples (for reference)

- GPU encode (NVENC) baseline (subject to actual graph):

```
ffmpeg -y -hide_banner -loglevel error -hwaccel cuda \
  -i bg.mp4 -i overlay.png \
  -filter_complex "[0:v]scale_npp=1080:1920,hwupload_cuda[bg];[1:v]scale_npp=640:640,hwupload_cuda[ov];[bg][ov]overlay_cuda=100:200:format=auto,format=yuv420p[v]" \
  -map "[v]" -map 0:a? -c:v h264_nvenc -preset p4 -b:v 8M \
  -progress pipe:1 output.mp4
```

- CPU fallback baseline:

```
ffmpeg -y -hide_banner -loglevel error \
  -i bg.mp4 -i overlay.png \
  -filter_complex "[0:v]scale=1080:1920[bg];[1:v]scale=640:640[ov];[bg][ov]overlay=100:200,format=yuv420p[v]" \
  -map "[v]" -map 0:a? -c:v libx264 -preset veryfast -b:v 8M \
  -progress pipe:1 output.mp4
```

---

## 7) Roadmap after MVP (short)

- Add rotate, blur, brightness mappings; more transitions
- Add Intel QSV / AMD AMF / macOS VideoToolbox backends
- Add persistent job store (SQLite/Redis) + resumable downloads
- Optional: wgpu compositor path for complex effects, then hardware encode

---

## 8) Acceptance for MVP

- Given a typical design from the current editor (video + image + audio), POST /render returns jobId, progress advances, and a playable MP4 is available via GET /render/:id/output.
- When NVIDIA GPU present, encoder is NVENC; otherwise falls back to CPU without crashing.

