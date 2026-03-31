import SwiftUI
import NetworkExtension
import Combine

struct ContentView: View {
    @StateObject private var vpnManager = VPNManager.shared
    
    @State private var mode: String = "s2n-quic"
    @State private var serverAddr: String = ""
    @State private var serverName: String = ""
    @State private var cert: String = ""
    @State private var privKey: String = ""
    
    @State private var errorMessage: String?
    @State private var showError: Bool = false
    
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
                        .disableAutocorrection(true)
                    
                    TextField("Server Name (SNI)", text: $serverName)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                }
                
                Section(header: Text("Certificate (PEM Content)")) {
                    TextEditor(text: $cert)
                        .frame(height: 100)
                        .font(.system(.caption, design: .monospaced))
                }
                
                Section(header: Text("Private Key (PEM Content)")) {
                    TextEditor(text: $privKey)
                        .frame(height: 100)
                        .font(.system(.caption, design: .monospaced))
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
                        if vpnManager.status == .connecting || vpnManager.status == .reasserting {
                            ProgressView()
                                .progressViewStyle(CircularProgressViewStyle())
                        } else {
                            Text(vpnManager.status == .disconnected ? "Connect" : "Disconnect")
                                .frame(maxWidth: .infinity)
                                .foregroundColor(vpnManager.status == .disconnected ? .blue : .red)
                        }
                    }
                    .disabled(vpnManager.status == .connecting || vpnManager.status == .reasserting)
                }
            }
            .navigationTitle("stunnel-ios")
            .alert(isPresented: $showError) {
                Alert(title: Text("Error"), message: Text(errorMessage ?? "Unknown error"), dismissButton: .default(Text("OK")))
            }
            .onAppear {
                do {
                    if let config = try VPNConfig.load() {
                        self.mode = config.mode
                        self.serverAddr = config.serverAddr
                        self.serverName = config.serverName
                        self.cert = config.cert
                        self.privKey = config.privKey
                    }
                } catch {
                    self.errorMessage = error.localizedDescription
                    self.showError = true
                }
            }
            .onReceive(vpnManager.$lastErrorMessage.compactMap { $0 }) { message in
                self.errorMessage = message
                self.showError = true
            }
        }
    }
    
    private func toggleVPN() {
        if vpnManager.status == .disconnected {
            if serverAddr.isEmpty || serverName.isEmpty || cert.isEmpty || privKey.isEmpty {
                errorMessage = "Please fill in all configuration fields"
                showError = true
                return
            }
            
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
