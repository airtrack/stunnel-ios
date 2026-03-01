use smoltcp::wire::Ipv4Address;

pub fn is_private_v4(addr: Ipv4Address) -> bool {
    let octets = addr.octets();
    // 10.0.0.0/8
    if octets[0] == 10 {
        return true;
    }
    // 172.16.0.0/12
    if octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31 {
        return true;
    }
    // 192.168.0.0/16
    if octets[0] == 192 && octets[1] == 168 {
        return true;
    }
    // 127.0.0.0/8
    if octets[0] == 127 {
        return true;
    }
    // 169.254.0.0/16
    if octets[0] == 169 && octets[1] == 254 {
        return true;
    }
    false
}
