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
    sudo install -Dm0755 {{TARGET}} /usr/bin/stokes-browser
    sudo install -Dm0644 assets/com.ethanstokes.stokes-browser.desktop /usr/share/applications/com.ethanstokes.stokes-browser.desktop
    sudo install -Dm0644 assets/com.ethanstokes.stokes-browser.png /usr/share/icons/hicolor/256x256/apps/com.ethanstokes.stokes-browser.png

# Uninstall the project
uninstall:
    sudo rm /usr/bin/stokes-browser /usr/share/applications/com.ethanstokes.stokes-browser.desktop /usr/share/icons/hicolor/256x256/apps/com.ethanstokes.stokes-browser.png

replace: build uninstall install