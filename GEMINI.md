# Project Overview: stunnel-ios

`stunnel-ios` is a high-performance network proxy application for iOS. It leverages Apple's **Network Extension (NE)** framework to provide system-wide proxying capabilities with advanced split-tunneling features.

## Architecture

### 1. Process Isolation
iOS enforces strict process isolation. The main UI and the proxy engine run in separate processes:
*   **Host App:** The UI process. Handles configuration UI and VPN lifecycle management.
*   **Network Extension:** The "Packet Tunnel Provider" process. Managed by the system as a daemon.

### 2. Rust Proxy Core (Modular Design)
The core logic is implemented in Rust for maximum performance and safety:
*   **`engine`**: Manages the `smoltcp` stack and virtual `TunDevice`.
*   **`tcp` / `udp`**: Implements `TcpStream` and `UdpSocket` abstractions that satisfy `tokio`'s async traits, allowing high-level data forwarding.
*   **`connection`**: Manages persistent outbound tunnels. Specifically optimized for **QUIC connection reuse** via `s2n-quic` handles.
*   **`utils`**: Includes intelligent split-tunneling that automatically bypasses private/intranet IP addresses.

### 3. Data Flow
`TUN Interface` <-> `Swift Bridge` <-> `C-FFI` <-> `smoltcp` <-> `TcpStream/UdpSocket` <-> `stunnel client` <-> `Remote Server`

## Technology Stack

*   **iOS:** Swift 5, SwiftUI, Network Extension.
*   **Rust Core:** Rust 2024 Edition, `smoltcp`, `tokio`, `s2n-quic`, `bytes`.
*   **Build System:** `xcodegen` for Xcode project management, `build.sh` for unified compilation.

## Development Roadmap

### Phase 1: Environment Setup
- [x] Configure Network Extension entitlements.
- [x] Set up Rust cross-compilation toolchain for iOS & Simulator.
- [x] Create declarative Xcode project using `xcodegen`.

### Phase 2: Rust Core Development
- [x] Implement C-FFI wrapper with safe memory management.
- [x] Integrate `smoltcp` user-mode TCP/IP stack.
- [x] Implement `AsyncRead`/`AsyncWrite` bridging for `smoltcp` sockets.
- [x] Implement dynamic TCP/UDP interception and session management.
- [x] Add Intranet Bypass (Private IP) logic.

### Phase 3: Extension Integration
- [x] Implement `NEPacketTunnelProvider` lifecycle and packet loop.
- [x] Bridge TUN packets to Rust via C-FFI callbacks.
- [x] Configure system-level IPv4 routing and DNS.

### Phase 4: UI & Configuration
- [x] Design SwiftUI interface for PEM certificate management.
- [x] Implement App Group-based configuration and certificate sharing.
- [x] Implement `VPNManager` using `NETunnelProviderManager`.

## Building and Running

### Prerequisites
*   macOS with **Xcode 15+**.
*   **Paid Apple Developer Account** (Required for Network Extension entitlements).
*   `xcodegen` and `xcbeautify` (via Homebrew).

### Build Commands
Use the unified build script:
```bash
# Build Release version (default)
./build.sh

# Build Debug version
./build.sh --debug
```

## Post-Development Notes
*   **Signing:** The project uses Ad-hoc signing (`-`) for local builds but requires a valid Development Team ID in Xcode for actual deployment.
*   **Memory:** The Rust core is optimized to stay within the ~15MB limit of the Network Extension.
