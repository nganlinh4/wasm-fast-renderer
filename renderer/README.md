Renderer (Rust + FFmpeg)

A minimal GPU-accelerated rendering service for Windows (NVENC) with CPU fallback.

Run:
1) Ensure FFmpeg is installed and in PATH. Prefer a build with NVENC.
2) set RENDER_PORT=6108 (optional)
3) cargo run --release

API:
- POST /render { design, options } -> { jobId }
- GET  /render/:id -> { status, progress, url? }
- GET  /render/:id/output -> mp4 bytes

Notes:
- Uses NVENC if detected via `ffmpeg -encoders`.
- Downloads assets to ./render_jobs/<id>/ and writes output.mp4.
- Progress parsed from `-progress pipe:1` using `out_time_ms` vs computed duration.

