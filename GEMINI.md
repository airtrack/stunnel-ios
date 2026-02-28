# Project Overview: stunnel-ios

`stunnel-ios` is a high-performance network proxy application for iOS. It leverages Apple's **Network Extension (NE)** framework to provide system-wide proxying capabilities with advanced split-tunneling features.

The project is structured into three main components:
1.  **Main App (Swift/SwiftUI):** A user-facing application for managing configurations, monitoring connection status, and controlling the proxy service.
2.  **Packet Tunnel Provider (Swift Extension):** A dedicated background process that manages the virtual TUN interface, intercepts network traffic, and routes it through the proxy core.
3.  **Rust Proxy Core:** A high-performance networking engine written in Rust that handles the heavy lifting of TCP/IP stack reconstruction, protocol encapsulation, and rule-based routing.

## Architecture

### 1. Process Isolation
iOS enforces strict process isolation. The main UI and the proxy engine run in separate processes:
*   **Host App:** The UI process. If killed by the system or user, the proxy service remains active.
*   **Network Extension:** The "Packet Tunnel Provider" process. It is managed by the system and runs as a daemon while the VPN is active.

### 2. Traffic Flow
1.  **TUN Interface:** The Extension creates a virtual network interface.
2.  **IP Packet Capture:** All device traffic (matching the routing rules) is routed to the TUN interface as Raw IP packets (Layer 3).
3.  **User-mode TCP/IP Stack:** The Extension passes packets to the Rust Core, which uses a user-mode stack (e.g., `smoltcp`) to reconstruct TCP streams.
4.  **Proxying:** Reconstructed streams are processed by the Rust proxy logic (split-tunneling, encryption, etc.) and sent to the remote proxy server via standard sockets.

## Technology Stack

*   **iOS Development:** Swift, SwiftUI, Network Extension Framework.
*   **Core Logic:** Rust (compiled for `aarch64-apple-ios`).
*   **Inter-Process Communication (IPC):** `NETunnelProviderManager`, App Groups (for shared configuration and status), Darwin Notifications.
*   **Networking Stack:**
    *   **smoltcp:** A standalone, event-driven TCP/IP stack that will be used to reconstruct TCP/UDP streams from raw IP packets received from the TUN interface.
    *   **stunnel (Rust Core):** The existing proxy library (from `https://github.com/airtrack/stunnel.git`) will be integrated to handle the outbound proxying (TLS/QUIC tunnels) and split-tunneling logic.
*   **Networking:** `tokio` for async I/O in the Rust core, bridging `smoltcp` stream data to the `stunnel` client logic.

## Development Roadmap

### Phase 1: Environment Setup
- [ ] Configure Apple Developer Program entitlements for Network Extension.
- [ ] Set up Rust cross-compilation toolchain for iOS.
- [ ] Create Xcode project with App and Extension targets.

### Phase 2: Rust Core Development
- [ ] Implement C-FFI wrapper for Rust core.
- [ ] Integrate user-mode TCP/IP stack.
- [ ] Implement basic TCP split-tunneling logic.

### Phase 3: Extension Integration
- [ ] Implement `NEPacketTunnelProvider` lifecycle methods (`startTunnel`, `stopTunnel`).
- [ ] Bridge TUN packets to the Rust core via FFI.
- [ ] Handle system-level routing and DNS configuration.

### Phase 4: UI & Configuration
- [ ] Design SwiftUI interface for configuration editing.
- [ ] Implement App Group-based configuration sharing.
- [ ] Add connection status monitoring and logging.

## Building and Running

### Prerequisites
*   Xcode 15+
*   Rust toolchain (`rustup target add aarch64-apple-ios`)
*   Apple Developer Account with Network Extension entitlement.

### Build Commands
*   **Rust Core:** `cargo build --target aarch64-apple-ios --release` (or using a dedicated script).
*   **iOS App:** Build via Xcode using the `stunnel-ios` scheme.

## Development Conventions

*   **Memory Management:** The Network Extension is limited to ~15-30MB of RAM. Avoid large buffers and ensure efficient memory usage in the Rust core.
*   **Concurrency:** Use Swift `async/await` for UI/Extension logic and `tokio` for Rust async tasks.
*   **Logging:** Use `os_log` or a shared log file in the App Group container for cross-process debugging.
