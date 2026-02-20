# Termux Setup Deep-Dive: Bypassing Toolchain Issues

Running ZeroClaw on Android via Termux requires specific handling of the Rust toolchain to avoid platform-specific panics.

## 🛠️ The "Rustup Panic" Issue
Standard `rustup` installations on Android may panic with the following error when choosing a toolchain:
`thread 'main' panicked at ... rustls-platform-verifier-0.5.2/src/android.rs:87:10: Expect rustls-platform-verifier to be initialized`

### The Solution: Native Toolchain
Instead of `rustup`, use the Termux-native Rust package. This bypasses the platform verifier issue by using a toolchain specifically patched for the Termux environment.

**Command:**
```bash
pkg install rust binutils-is-llvm pkg-config curl git
```

## 🚀 Building Without Rustup
Even after installing the native `rust` package, `rustup` may still intercept `cargo` or `rustc` calls if it was previously installed. To build successfully, you must explicitly point to the native binaries and unset `rustup` environment variables.

**Build Command:**
```bash
RUSTUP_TOOLCHAIN="" 
RUSTC=/data/data/com.termux/files/usr/bin/rustc 
/data/data/com.termux/files/usr/bin/cargo build --release --locked
```

## 📦 Binary Location
After a successful build, the binary is located at:
`./target/release/zeroclaw`

## ⚠️ Known Platform Constraints
- **USB Access:** USB discovery (`nusb`) will fail unless the device is rooted and Termux has specific permissions.
- **Service Management:** `systemd` is not available; use `tmux` or `screen` to manage the daemon.
