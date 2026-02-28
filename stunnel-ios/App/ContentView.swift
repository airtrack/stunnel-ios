import SwiftUI
import NetworkExtension

struct ContentView: View {
    @State private var vpnStatus: String = "Disconnected"
    @State private var isConnected: Bool = false
    
    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: isConnected ? "lock.shield.fill" : "lock.shield")
                .font(.system(size: 80))
                .foregroundColor(isConnected ? .green : .gray)
            
            Text("stunnel-ios")
                .font(.largeTitle)
                .fontWeight(.bold)
            
            Text("Status: \(vpnStatus)")
                .font(.headline)
            
            Button(action: toggleVPN) {
                Text(isConnected ? "Disconnect" : "Connect")
                    .frame(width: 200, height: 50)
                    .background(isConnected ? Color.red : Color.blue)
                    .foregroundColor(.white)
                    .cornerRadius(10)
            }
        }
        .padding()
        .onAppear {
            updateVPNStatus()
        }
    }
    
    private func toggleVPN() {
        // Here we would use NETunnelProviderManager to start/stop the tunnel
        isConnected.toggle()
        vpnStatus = isConnected ? "Connected" : "Disconnected"
    }
    
    private func updateVPNStatus() {
        // Periodically check the status from NETunnelProviderManager
    }
}
