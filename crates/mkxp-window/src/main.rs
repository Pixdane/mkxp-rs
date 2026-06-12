mod app;
mod error;
mod frame_sync;
mod render_host;
mod runtime;
mod script_host;
mod window_control;

use winit::event_loop::EventLoop;

use mkxp_types::MkxpError;

use crate::app::App;
use crate::runtime::RuntimeEvent;

fn main() -> anyhow::Result<()> {
    mkxp_log::init(mkxp_log::LogConfig::default())?;

    let event_loop = EventLoop::<RuntimeEvent>::with_user_event()
        .build()
        .map_err(|error| MkxpError::Init(format!("failed to create event loop: {error}")))?;
    let proxy = event_loop.create_proxy();
    let mut app = App::new(proxy);
    event_loop
        .run_app(&mut app)
        .map_err(|error| MkxpError::Runtime(format!("event loop error: {error}")))?;

    if let Some(error) = app.take_fatal_error() {
        Err(error.into())
    } else {
        Ok(())
    }
}
