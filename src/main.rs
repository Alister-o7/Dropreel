// No console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use dropreel::app::DropReelApp;
use dropreel::paths::AppPaths;
use eframe::egui;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

fn init_logging(paths: &AppPaths) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("dropreel")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&paths.logs)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("DROPREEL_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,eframe=warn,egui_glow=warn"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        default_hook(info);
    }));
    Some(guard)
}

fn fatal(message: &str) {
    tracing::error!("{message}");
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("DropReel")
        .set_description(message)
        .show();
}

/// The window icon (assets/icon.png).
fn icon() -> egui::IconData {
    let decoded = image::load_from_memory(include_bytes!("../assets/icon.png")).map(|img| img.into_rgba8());
    match decoded {
        Ok(img) => egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() },
        Err(_) => egui::IconData::default(),
    }
}

fn main() -> ExitCode {
    let paths = match AppPaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            fatal(&format!(
                "DropReel could not create its data folder:\n{e:#}"
            ));
            return ExitCode::FAILURE;
        }
    };
    let _log_guard = init_logging(&paths);
    tracing::info!(
        "DropReel {} starting, data in {}",
        env!("CARGO_PKG_VERSION"),
        paths.root.display()
    );

    let open_paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DropReel")
            .with_app_id("DropReel")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 480.0])
            .with_drag_and_drop(true)
            .with_icon(icon()),
        ..Default::default()
    };
    let result = eframe::run_native(
        "DropReel",
        options,
        Box::new(move |cc| match DropReelApp::new(cc, paths, open_paths) {
            Ok(app) => Ok(Box::new(app)),
            Err(e) => {
                let message = format!("DropReel could not start:\n{e:#}");
                fatal(&message);
                Err(message.into())
            }
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}
