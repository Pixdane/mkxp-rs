mod app;
mod error;
mod frame_sync;
mod render_host;
mod runtime;
mod script_host;
mod window_control;

use winit::event_loop::EventLoop;

use mkxp_types::MkxpError;
use tracing::{debug, info};

use crate::app::App;
use crate::runtime::{RuntimeConfig, RuntimeEvent};
use crate::script_host::DemoScriptEngine;

fn main() -> anyhow::Result<()> {
    let config = mkxp_config::load(std::env::args().collect())?;
    mkxp_log::init(mkxp_log::LogConfig::from(&config))?;
    let runtime_config = RuntimeConfig::from(config);
    info!(
        title = %runtime_config.window_title,
        width = runtime_config.window_size.0,
        height = runtime_config.window_size.1,
        target_fps = runtime_config.target_fps,
        vsync = runtime_config.vsync,
        reset_enabled = runtime_config.enable_reset,
        "mkxp-window starting"
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
        info!("mkxp-window stopped cleanly");
        Ok(())
    }
}
