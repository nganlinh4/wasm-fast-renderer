use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderOptions {
    pub fps: Option<u32>,
    pub size: Option<Size>,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignEnvelope {
    pub design: Design,
    pub options: Option<RenderOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Design {
    pub id: Option<String>,
    #[serde(default)]
    pub trackItems: Vec<TrackItem>,
    #[serde(default)]
    pub trackItemsMap: HashMap<String, TrackItem>,
    pub size: Option<Size>,
    pub fps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")] // types appear as "video", "image", "audio", "text"
pub enum TrackType { Video, Image, Audio, Text, }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackItem {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: TrackType,
    pub details: Option<Details>,
    #[serde(default)]
    pub trim: Trim,
    #[serde(default)]
    pub display: Trim,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Trim {
    pub from: Option<u64>, // ms
    pub to: Option<u64>,   // ms
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Details {
    pub src: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub opacity: Option<f32>,      // 0-100
    pub volume: Option<f32>,       // 0-100
    pub left: Option<String>,      // e.g., "100px"
    pub top: Option<String>,       // e.g., "200px"
    pub transform: Option<String>, // e.g., "scale(1.25)"
    pub brightness: Option<f32>,   // default 100
    // extended support
    pub flipX: Option<bool>,
    pub flipY: Option<bool>,
    pub rotate: Option<String>,    // e.g., "90deg" or "-45deg"
    // text-only fields
    pub text: Option<String>,
    pub fontFamily: Option<String>,
    pub fontUrl: Option<String>,
    pub fontSize: Option<u32>,
    pub color: Option<String>,
    pub borderColor: Option<String>,
    pub borderWidth: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitResponse { pub jobId: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub status: String,
    pub progress: u32,
    pub url: Option<String>,
    pub error: Option<String>,
}

