# Repository Guidelines

## Project Structure & Module Organization
This repository has two main codebases:

- `stunnel-core/`: Rust proxy engine exposed through C FFI. Core modules live in `stunnel-core/src/` (`engine.rs`, `tcp.rs`, `udp.rs`, `connection.rs`, `config.rs`, `utils.rs`), with headers in `stunnel-core/include/`.
- `stunnel-ios/`: iOS app sources managed by XcodeGen. `App/` contains the SwiftUI host app, `PacketTunnel/` contains the `NEPacketTunnelProvider` extension, and `Shared/` contains shared Swift models and the bridging header.

Generated artifacts belong in `stunnel-core/target/`, `stunnel-ios/build/`, and `stunnel-ios/stunnel-ios.xcodeproj/`; do not hand-edit generated project files.

## Build, Test, and Development Commands
- `./build.sh`: Builds the Rust core for simulator and device, regenerates the Xcode project, and performs a simulator app build.
- `./build.sh --debug`: Same workflow using Debug configuration.
- `cd stunnel-core && cargo build --target aarch64-apple-ios-sim`: Rebuild only the Rust library for simulator work.
- `cd stunnel-ios && xcodegen generate`: Regenerate `stunnel-ios.xcodeproj` after editing `project.yml`.
- `cd stunnel-ios && xcodebuild build -scheme stunnel-ios -configuration Debug -destination 'generic/platform=iOS Simulator'`: Run the iOS build directly.

Use macOS with Xcode 15+, `xcodegen`, `xcbeautify`, and the iOS Rust targets.

## Coding Style & Naming Conventions
Follow existing style in each layer: Swift uses 4-space indentation, `UpperCamelCase` for types, and `lowerCamelCase` for properties and methods. Rust uses `snake_case` for modules/functions and keeps FFI entry points prefixed with `stunnel_`. Keep modules small and prefer explicit bridging and configuration code.

No formatter or linter is committed here, so match surrounding code and keep comments brief and technical.

## Testing Guidelines
There is no committed automated test suite yet. At minimum, contributors should:

- run `./build.sh` before opening a PR;
- rebuild the touched layer directly (`cargo build` for Rust, `xcodebuild build` for iOS) when iterating;
- include validation notes for VPN setup, tunnel startup, and config persistence.

When adding tests, place Rust unit tests beside the module they cover and keep test names behavior-focused.

## Commit & Pull Request Guidelines
Recent history uses Conventional Commit prefixes such as `refactor:`, `docs:`, and `chore:`. Keep subjects imperative and specific, for example `refactor: simplify PacketTunnel config loading`.

PRs should include a summary, affected areas (`stunnel-core`, `App`, `PacketTunnel`, `Shared`), build/test results, and screenshots for UI changes. Call out entitlement, signing, or Network Extension behavior changes explicitly.

## Security & Configuration Tips
Do not commit real certificates, private keys, team identifiers, or provisioning data. Network Extension changes often require manual signing verification in Xcode; document any entitlement or bundle ID updates in the PR.
