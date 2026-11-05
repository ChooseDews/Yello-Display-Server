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
    new_element, default_design,
    DESIGN_SCHEMA_VERSION, DEFAULT_PAGE_ID, NavTarget, parse_nav_target,
    validate_design, design_home_page_id, effective_page_layout,
};

pub use protocol::{
    PROTO_MAGIC, PROTO_VERSION, MsgType, TouchType, Orientation as ProtoOrientation,
    DeviceCommandType, TouchEvent, DeviceConfig, DeviceStatus, DeviceCommand,
    FramePatch, FramePatchHeader, ZoneRecord, ResyncRequest, ResyncReason,
    DeviceHello, CAP_RAW_BUNDLE, CAP_DEFLATE, CAP_FRAME_PATCH, CAPS_CURRENT,
    DEVICE_HELLO_BYTES, parse_hello_hint,
    FRAME_PATCH_HEADER_BYTES, ZONE_RECORD_HEADER_BYTES, PATCH_WINDOW_BITS,
    PATCH_FLAG_COMPRESSED, PATCH_FLAG_FULL_FRAME,
    build_raw_bundle, parse_raw_bundle, compress_bundle, decompress_bundle,
    rgb888_to_rgb565_be,
};

pub use frame_diff::dirty_regions;

pub use server::{
    AppState, DeviceSession, Studio, Design, StreamSample, LivePreviewState,
    MAX_ZONE_AREA_PX, STREAM_STATS_WINDOW_S,
    MAX_EXTERNAL_BYTES, MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, FULL_RESYNC_INTERVAL_S,
    wanted_image_sources, decode_image_bytes,
    EXTERNAL_FETCH_TIMEOUT_S, DOT_LIFETIME_S, BUTTON_FLASH_S, IMAGE_FETCH_RETRY_S,
    push_frame, push_design, design_id_for_session, handle_http_request, handle_device_ws,
    start_background_tasks, write_atomic,
    active_target, home_target, apply_navigation, cross_design_link_errors,
    with_embed_images, check_screen_embed,
    all_design_elements, design_image_sources, design_page_exists,
};

pub use renderer::{
    render, fit_image, hit_test_buttons, resolve_button_action, has_clock,
    parse_color, brightness, draw_bitmap_text,
};

pub use home_assistant::{HaClient, HaState, HaError, HomeAssistantError};

pub use safe_http::{SafeHttp, SafeHttpError, UnsafeUrlError, parse_remote_url};

pub use script_runtime::{ScriptError, ScriptOutput, execute_script, SCRIPT_EXAMPLES};
