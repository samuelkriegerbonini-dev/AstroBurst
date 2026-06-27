fn main() {
    // Headless server build support (only runs tauri_build for the desktop feature) —
    // contributed by Jae-Joon Lee (https://github.com/leejjoon).
    if std::env::var("CARGO_FEATURE_TAURI").is_ok() {
        tauri_build::build()
    }
}
