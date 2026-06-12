//! GUI runtime entry point for the mkxp-rs demo host.
//!
//! This crate exposes the winit application as a library entry so the binary can
//! stay thin and future binaries can choose a different script engine boundary
//! without duplicating the window/render bootstrap.
//!
//! The public entry point loads configuration, initializes logging, creates the
//! winit event loop, and runs `App<DemoScriptEngine>`.
//!
//! ```no_run
//! fn main() -> anyhow::Result<()> {
//!     mkxp_gui::run_demo()
//! }
//! ```

mod app;
mod error;
mod frame_sync;
mod render_host;
mod runtime;
mod script_host;
mod window_control;

use tracing::{debug, info};
use winit::event_loop::EventLoop;

use mkxp_types::MkxpError;

use crate::app::App;
use crate::runtime::{RuntimeConfig, RuntimeEvent};
use crate::script_host::DemoScriptEngine;

/// Run the default mkxp-rs demo window.
///
/// This function owns process-level startup for the demo binary: it loads the
/// runtime configuration, initializes logging, creates the winit event loop, and
/// delegates application lifecycle to `App<DemoScriptEngine>`.
///
/// The function returns only after the winit event loop exits. Fatal script,
/// render, window, or bootstrap errors are converted into the returned
/// `anyhow::Result`.
///
/// ```no_run
/// fn main() -> anyhow::Result<()> {
///     mkxp_gui::run_demo()
/// }
/// ```
pub fn run_demo() -> anyhow::Result<()> {
    let config = mkxp_config::load(std::env::args().collect())?;
    mkxp_log::init(mkxp_log::LogConfig::from(&config))?;
    let runtime_config = RuntimeConfig::from(config);
    info!(
        title = %runtime_config.window_title,
        width = runtime_config.window_size.0,
        height = runtime_config.window_size.1,
        game_width = runtime_config.game_size.0,
        game_height = runtime_config.game_size.1,
        target_fps = runtime_config.target_fps,
        vsync = runtime_config.vsync,
        reset_enabled = runtime_config.enable_reset,
        "mkxp-gui starting"
    );
    debug!(
        scripts_path = ?runtime_config.scripts_path,
        rgss_version = ?runtime_config.rgss_version,
        "runtime config resolved"
    );

    debug!("creating winit event loop");
    let event_loop = EventLoop::<RuntimeEvent>::with_user_event()
        .build()
        .map_err(|error| MkxpError::Init(format!("failed to create event loop: {error}")))?;
    let proxy = event_loop.create_proxy();
    let mut app = App::<DemoScriptEngine>::new(proxy, runtime_config);
    info!("entering winit event loop");
    event_loop
        .run_app(&mut app)
        .map_err(|error| MkxpError::Runtime(format!("event loop error: {error}")))?;

    if let Some(error) = app.take_fatal_error() {
        Err(error.into())
    } else {
        info!("mkxp-gui stopped cleanly");
        Ok(())
    }
}
