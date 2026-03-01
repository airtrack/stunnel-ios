import SwiftUI
import NetworkExtension

struct ContentView: View {
    @StateObject private var vpnManager = VPNManager.shared
    
    @State private var mode: String = "s2n-quic"
    @State private var serverAddr: String = ""
    @State private var serverName: String = ""
    @State private var cert: String = ""
    @State private var privKey: String = ""
    
    let modes = ["s2n-quic", "tlstcp"]
    
    var body: some View {
        NavigationView {
            Form {
                Section(header: Text("Configuration")) {
                    Picker("Mode", selection: $mode) {
                        ForEach(modes, id: \.self) {
                            Text($0)
                        }
                    }
                    
                    TextField("Server Address (IP:Port)", text: $serverAddr)
                        .autocapitalization(.none)
                    
                    TextField("Server Name (SNI)", text: $serverName)
                        .autocapitalization(.none)
                    
                    TextField("Certificate Path", text: $cert)
                        .autocapitalization(.none)
                    
                    TextField("Private Key Path", text: $privKey)
                        .autocapitalization(.none)
                }
                
                Section(header: Text("Status")) {
                    HStack {
                        Text("VPN Status")
                        Spacer()
                        statusBadge(status: vpnManager.status)
                    }
                }
                
                Section {
                    Button(action: toggleVPN) {
                        Text(vpnManager.status == .disconnected ? "Connect" : "Disconnect")
                            .frame(maxWidth: .infinity)
                            .foregroundColor(vpnManager.status == .disconnected ? .blue : .red)
                    }
                }
            }
            .navigationTitle("stunnel-ios")
            .onAppear {
                if let config = VPNConfig.load() {
                    self.mode = config.mode
                    self.serverAddr = config.serverAddr
                    self.serverName = config.serverName
                    self.cert = config.cert
                    self.privKey = config.privKey
                }
            }
        }
    }
    
    private func toggleVPN() {
        if vpnManager.status == .disconnected {
            let config = VPNConfig(
                mode: mode,
                serverAddr: serverAddr,
                serverName: serverName,
                cert: cert,
                privKey: privKey
            )
            vpnManager.setupAndConnect(config: config)
        } else {
            vpnManager.disconnect()
        }
    }
    
    private func statusBadge(status: NEVPNStatus) -> some View {
        let text: String
        let color: Color
        
        switch status {
        case .connected:
            text = "Connected"
            color = .green
        case .connecting:
            text = "Connecting"
            color = .orange
        case .disconnecting:
            text = "Disconnecting"
            color = .orange
        case .disconnected:
            text = "Disconnected"
            color = .gray
        case .invalid:
            text = "Invalid"
            color = .red
        case .reasserting:
            text = "Reasserting"
            color = .blue
        @unknown default:
            text = "Unknown"
            color = .gray
        }
        
        return Text(text)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(color)
            .foregroundColor(.white)
            .cornerRadius(4)
    }
}
