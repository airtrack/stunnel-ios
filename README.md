# stunnel-ios

`stunnel-ios` is a high-performance network proxy application for iOS. It leverages Apple's **Network Extension (NE)** framework to provide system-wide proxying capabilities with advanced split-tunneling features.

## Technical Architecture

The project is structured into three main layers:

1.  **Main App (SwiftUI):** A user-facing application for managing configurations, monitoring connection status, and controlling the VPN service.
2.  **Packet Tunnel Provider (Swift Extension):** A dedicated background process that manages the virtual TUN interface, intercepts network traffic, and bridges packets to the Rust core via C-FFI.
3.  **Rust Proxy Core:**
    *   **smoltcp:** A standalone, user-mode TCP/IP stack used to reconstruct streams from raw IP packets.
    *   **stunnel (client):** Integrated proxy library handling outbound QUIC (`s2n-quic`) and TLS/TCP tunnels.
    *   **Bridging:** Uses `TcpStream` and `UdpSocket` abstractions to bridge `smoltcp` with `tokio` asynchronous I/O.
    *   **Routing:** Intelligent split-tunneling logic that automatically bypasses private/intranet IP addresses.

---

## Development Environment Setup

This project requires **macOS** with **Xcode 15+**.

### 1. Install System Dependencies
Install `xcodegen` to manage Xcode project files and `xcbeautify` for cleaner build logs:
```bash
brew install xcodegen xcbeautify
```

### 2. Set Up Rust Toolchain
Install Rust and add the iOS targets:
```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add iOS targets
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

---

## Building and Running

### Quick Build
Use the provided script to build the Rust core and generate the Xcode project:
```bash
# Build release version (default)
./build.sh

# Build debug version
./build.sh --debug
```

### Running in Xcode
1.  Open the generated project:
    ```bash
    open stunnel-ios/stunnel-ios.xcodeproj
    ```
2.  Select the `stunnel-ios` scheme.
3.  **Important:** Due to Network Extension restrictions, you must have a **paid Apple Developer Account** to run this on a device or simulator.
    *   In Xcode, go to `Signing & Capabilities`.
    *   Select your **Team**.
    *   Xcode will automatically configure the Provisioning Profiles and App Group.
4.  Run the project (Cmd + R).

---

## Configuration
In the App UI, you can configure:
*   **Mode:** `s2n-quic` (Recommended) or `tlstcp`.
*   **Server:** Proxy server IP and Port.
*   **SNI:** The Server Name Indication for TLS/QUIC handshake.
*   **Certificates:** Paste your PEM-formatted client certificate and private key directly into the text fields.

---

## Performance Notes
*   **Connection Reuse:** QUIC mode reuses a single connection handle for multiple streams, significantly reducing handshake latency.
*   **Zero-Copy:** Leverages the `bytes` crate to minimize memory allocation and copying across the FFI boundary.
*   **Memory Limit:** The Network Extension is limited to 15-30MB of RAM. The Rust core is optimized for low memory footprint.
