# Running ZeroClaw on Android via Termux

ZeroClaw is optimized for high-efficiency, low-power environments, making it a perfect fit for **Termux** on Android. This document summarizes the approach taken to run and optimize ZeroClaw in this environment.

## 🚀 Why ZeroClaw in Termux?

- **Ultra-Lean:** Release builds consume **<5MB of RAM**, preventing the Android OOM (Out Of Memory) killer from reaping the process.
- **Fast Cold Starts:** Sub-10ms startup ensures the agent is responsive even on mid-range mobile hardware.
- **Native Execution:** Runs directly on the device's ARM64/ARMv7 architecture without the overhead of heavy runtimes like Node.js or Python.

## 🛠️ Technical Approach

### 1. Environment Preparation
Termux provides a specialized environment with a non-standard filesystem layout (prefixed with `/data/data/com.termux/files/usr/`). ZeroClaw handles this by:
- Relying on `cargo` and `rustc` for portable builds.
- Using the `NativeRuntime` adapter, which executes shell commands via the Termux-provided shell.

### 2. Platform Adaptation
ZeroClaw's codebase contains explicit guards for Android/Termux:
- **Hardware Discovery:** In `src/hardware/discover.rs`, ZeroClaw detects the platform and gracefully bails on USB enumeration (`nusb`), as Android requires root or specific API calls for raw USB access.
- **Storage:** Uses SQLite for memory persistence, which is highly efficient on mobile flash storage.

### 3. Setup Workflow
The following steps are used to initialize ZeroClaw in Termux:

```bash
# 1. Update packages and install dependencies
pkg update && pkg upgrade
pkg install rust git binutils-is-llvm pkg-config curl

# 2. Clone the repository
git clone https://github.com/zeroclaw-labs/zeroclaw.git
cd zeroclaw

# 3. Run the bootstrap script
# Note: bootstrap.sh handles dependency checks and builds the release binary
./bootstrap.sh

# 4. Add to PATH
export PATH="$HOME/.cargo/bin:$PATH"
```

## 📈 Performance Observations (ARM64 Android)

| Metric | Measurement |
|---|---|
| **Binary Size** | ~3.4 MB |
| **Peak RAM (Idle)** | ~3.9 MB |
| **Peak RAM (Agent Loop)** | ~4.5 MB |
| **Startup Latency** | < 15ms |

## ⚠️ Known Limitations in Termux

- **Service Management:** The `zeroclaw service install` command (which targets `systemd` or `launchd`) is currently unavailable. Use a terminal multiplexer like `tmux` or `screen` to keep the `zeroclaw daemon` running in the background.
- **Hardware Tools:** Raw USB/Peripheral access is restricted unless running on a rooted device with specialized permissions.
- **Browser Tools:** Native headless browser support requires `pkg install chromium` and may have higher memory overhead than the core agent.

---
*ZeroClaw — Zero overhead. Zero compromise. Mobile-first.* 🦀
