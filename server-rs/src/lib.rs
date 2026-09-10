//! Yello Device Server - Rust implementation
//!
//! A web server and WebSocket gateway for the ESP32 "Yellow" dev board.
//! Renders clock + message layouts and streams dirty-region updates to devices.

pub mod protocol;
pub mod layout_model;
pub mod frame_diff;
pub mod home_assistant;
pub mod safe_http;
pub mod script_runtime;
pub mod renderer;
pub mod server;

pub use layout_model::{
    Orientation, DisplayOrientation, LayoutValidationError, LayoutError,
    Layout, DataSource, Action, Frame, Element, ElementStyle,
    Screen, DEFAULT_DESIGN_ID, default_layout, layout_dimensions, validate_layout,
    new_element, migrate_layout,
};

pub use protocol::{
    PROTO_MAGIC, PROTO_VERSION, MsgType, PixelFormat, TouchType, Orientation as ProtoOrientation,
    DeviceCommandType, ZoneUpdate, TouchEvent, DeviceConfig, DeviceStatus, DeviceCommand,
    rgb888_to_rgb565_be,
};

pub use frame_diff::dirty_regions;

pub use server::{
    AppState, DeviceSession, Studio, Design, StreamSample, LivePreviewState,
    MAX_ZONE_AREA_PX, ZONE_HEADER_BYTES, STREAM_STATS_WINDOW_S, ZONE_PACING_S,
    MAX_EXTERNAL_BYTES, MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, FULL_RESYNC_INTERVAL_S,
    wanted_image_sources,
    EXTERNAL_FETCH_TIMEOUT_S, DOT_LIFETIME_S, BUTTON_FLASH_S,
    push_frame, push_design, design_id_for_session, handle_http_request, handle_device_ws,
    start_background_tasks,
};

pub use renderer::{
    render, fit_image, hit_test_buttons, resolve_button_action, has_clock,
    parse_color, brightness, draw_bitmap_text,
};

pub use home_assistant::{HaClient, HaState, HaError, HomeAssistantError};

pub use safe_http::{SafeHttp, SafeHttpError, UnsafeUrlError, parse_remote_url};

pub use script_runtime::{ScriptError, ScriptOutput, execute_script, SCRIPT_EXAMPLES};
