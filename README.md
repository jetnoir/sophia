# SOPHIA 🌐

**SOPHIA** is a modern, blazingly fast, and highly secure terminal UI (TUI) replacement for the classic `ping` command. Specifically optimized for macOS (Apple Silicon), it provides a stunning `btop`-style interface with real-time Braille charting and comprehensive network metrics.



## ✨ Features

- **Real-time Visualization**: High-resolution latency charting using Braille characters.
- **Comprehensive Metrics**: Tracks Current, Min, Max, Average, Jitter, and Packet Loss.
- **Optimized for macOS**: Built with Apple Silicon M4 in mind, leveraging unprivileged ICMP sockets.
- **Almost Zero Unsafe Code**:  A safe Rust implementation for maximum security.
- **Responsive Design**: Asynchronous networking ensures the UI never freezes.
- **Graceful Terminal Handling**: Seamlessly restores terminal state on exit.

## 🚀 Getting Started

### Prerequisites

- **Rust**: Ensure you have the latest stable Rust toolchain installed.
- **macOS**: Optimized for macOS, but compatible with other Unix-like systems.

### Installation

1. Clone the repository or navigate to the project folder:
   ```bash
   cd sophia
   ```
2. Build the release version:
   ```bash
   cargo build --release
   ```

### Usage

Simply provide a target hostname or IP address:

```bash
./target/release/sophia google.com
```

#### Options

- `-i, --interval <MS>`: Interval between pings (default: 1000ms).
- `-t, --timeout <MS>`: Timeout for each ping (default: 1000ms).

## ⌨️ Keyboard Shortcuts

- `1`: Switch to **Dashboard** view (default).
- `2`: Switch to **Chart** view (maximized chart).
- `3`: Switch to **Log** view (live ping stream).
- `4`: Switch to **Stats** view (detailed metrics).
- `F1`: Toggle the **Help Menu**.
- `q`: Quit the application.
- `Esc`: Close the help menu.
- `Ctrl+C`: Force quit and restore terminal.


## ⚖️ License & Copyright

**Copyright (c) 2026 Stuart Thomas**

- **Non-Commercial Use**: MIT License (included in the `LICENSE` file).
- **Commercial Use**: Requires a separate paid license. Please contact Stuart Thomas for commercial inquiries.

---

Built with some thought and reflection, by Stuart Thomas.
