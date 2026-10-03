//! Kickwerk KW-1: a modular main-kick machine for techno.
//!
//! * `dx serve --web` (feature `web`): WebAudio AudioWorklet.
//! * `dx serve --desktop` (feature `desktop`): native Linux window,
//!   PulseAudio output.

mod audio;
mod patch;
mod rack;
mod storage;
mod ui;

fn main() {
    #[cfg(feature = "desktop")]
    {
        use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
        dioxus::LaunchBuilder::desktop()
            .with_cfg(
                Config::new()
                    .with_menu(None)
                    .with_background_color((14, 15, 16, 255))
                    .with_window(
                        WindowBuilder::new()
                            .with_title("Kickwerk KW-1")
                            .with_inner_size(LogicalSize::new(1440.0, 900.0))
                            .with_min_inner_size(LogicalSize::new(800.0, 560.0)),
                    ),
            )
            .launch(ui::App);
    }

    #[cfg(not(feature = "desktop"))]
    dioxus::launch(ui::App);
}
