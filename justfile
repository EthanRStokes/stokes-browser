name := "stokes-browser"
appid := "com.ethanstokes.StokesBrowser"

# Variables
TARGET := "target/release/stokes-browser"

# Default target
default: build

# Build the project
build *args:
    cargo build --release {{args}}

# Run the project
run: build
    {{TARGET}}

# Clean the project
clean:
    cargo clean

# Install the project
install:
    install -Dm0755 {{TARGET}} /usr/bin/stokes-browser
    install -Dm0644 assets/com.ethanstokes.stokes-browser.desktop /usr/share/applications/com.ethanstokes.stokes-browser.desktop
    install -Dm0644 assets/com.ethanstokes.stokes-browser.png /usr/share/icons/hicolor/256x256/apps/com.ethanstokes.stokes-browser.png

# Uninstall the project
uninstall:
    rm /usr/bin/stokes-browser /usr/share/applications/com.ethanstokes.stokes-browser.desktop /usr/share/icons/hicolor/256x256/apps/com.ethanstokes.stokes-browser.png

# Bootstrap/update a local WPT checkout in third_party/wpt
wpt-bootstrap:
    python tools/wpt/bootstrap.py

# Serve WPT content locally for the runner (run in a second terminal)
wpt-serve:
    python third_party/wpt/wpt serve --config tools/wpt/serve-local.json --no-h2

# Run the Rust WPT harness against the selected manifest
wpt-run manifest="wpt/manifests/smoke.txt" expectations="wpt/expectations/known-failures.txt" output="wpt/results/latest.json" timeout_ms="8000" filter="":
    cargo run --release -- --wpt-run --manifest {{manifest}} --expectations {{expectations}} --output {{output}} --timeout-ms {{timeout_ms}} {{ if filter != "" { "--filter " + filter } else { "" } }}

# Copy the latest run output as the committed baseline
wpt-baseline source="wpt/results/latest.json" dest="wpt/baselines/latest.json":
    python tools/wpt/update_baseline.py --source {{source}} --dest {{dest}}

# Compare latest results to baseline and print regressions/improvements
wpt-diff baseline="wpt/baselines/latest.json" latest="wpt/results/latest.json":
    python tools/wpt/diff_results.py --baseline {{baseline}} --latest {{latest}}
