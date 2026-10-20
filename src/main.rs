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

/// A simple app icon: a red rounded square with a white play triangle.
fn icon() -> egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let r = 12.0;
            let dx = (fx - fx.clamp(r, N as f32 - r)).abs();
            let dy = (fy - fy.clamp(r, N as f32 - r)).abs();
            let inside = dx * dx + dy * dy <= r * r;
            // Play triangle with corners (24,18), (24,46), (46,32).
            let in_triangle = fx >= 24.0 && (fy - 32.0).abs() <= (46.0 - fx) * 14.0 / 22.0;
            let px = &mut rgba[(y * N + x) * 4..][..4];
            if !inside {
                continue;
            }
            px.copy_from_slice(if in_triangle {
                &[255, 255, 255, 255]
            } else {
                &[200, 45, 45, 255]
            });
        }
    }
    egui::IconData {
        rgba,
        width: N as u32,
        height: N as u32,
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
