//! # RouteScope Probe Module
//!
//! This module handles the low-level socket operations for sending traceroute probes
//! (ICMP, UDP, and TCP) and capturing responses.
//!
//! ## Unprivileged UDP Probing (IP_RECVERR + MSG_ERRQUEUE)
//! Historically, traceroute required raw socket permissions (`sudo` or `CAP_NET_RAW`)
//! to sniff incoming ICMP "Time Exceeded" replies.
//!
//! RouteScope bypasses this requirement for UDP by using standard unprivileged UDP sockets
//! with the `IP_RECVERR` socket option enabled. When enabled:
//! 1. The kernel intercepts ICMP errors (Time Exceeded, Port Unreachable) belonging to our socket.
//! 2. It queues them inside the socket's internal error queue.
//! 3. We retrieve the remote router's IP and error codes using `recvmsg` with the `MSG_ERRQUEUE` flag.
//!
//! ## ICMP Probing
//! For ICMP, RouteScope first attempts a Raw socket. If that fails (due to lack of privileges),
//! it falls back to unprivileged "ping sockets" (`SOCK_DGRAM` with `IPPROTO_ICMP`), which are
//! supported on many modern Linux distributions when configured via `net.ipv4.ping_group_range`.

use socket2::{Domain, Protocol, Socket, Type};
use std::net::{IpAddr, SocketAddr};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum ProbeMethod {
    ICMP,
    UDP,
    TCP,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub ttl: u8,
    pub ip: Option<IpAddr>,
    pub rtt: Option<Duration>,
    pub reached: bool,
    pub error: Option<String>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct sock_extended_err {
    ee_errno: u32,
    ee_origin: u8,
    ee_type: u8,
    ee_code: u8,
    ee_pad: u8,
    ee_info: u32,
    ee_data: u32,
}

pub fn send_probe(
    dest: IpAddr,
    method: ProbeMethod,
    ttl: u8,
    port: u16,
    timeout: Duration,
) -> ProbeResult {
    match method {
        ProbeMethod::UDP => send_udp_probe(dest, ttl, port, timeout),
        ProbeMethod::ICMP => send_icmp_probe(dest, ttl, port, timeout),
        ProbeMethod::TCP => send_tcp_probe(dest, ttl, port, timeout),
    }
}

fn send_udp_probe(dest: IpAddr, ttl: u8, port: u16, timeout: Duration) -> ProbeResult {
    let ipv6 = dest.is_ipv6();
    let domain = if ipv6 { Domain::IPV6 } else { Domain::IPV4 };

    let socket = match Socket::new(domain, Type::DGRAM, Some(Protocol::UDP)) {
        Ok(s) => s,
        Err(e) => {
            return ProbeResult {
                ttl,
                ip: None,
                rtt: None,
                reached: false,
                error: Some(format!("Socket creation failed: {}", e)),
            }
        }
    };

    // Set TTL
    if ipv6 {
        if let Err(e) = socket.set_unicast_hops_v6(ttl as u32) {
            return ProbeResult {
                ttl,
                ip: None,
                rtt: None,
                reached: false,
                error: Some(format!("Failed to set IPv6 hops: {}", e)),
            };
        }
    } else {
        if let Err(e) = socket.set_ttl(ttl as u32) {
            return ProbeResult {
                ttl,
                ip: None,
                rtt: None,
                reached: false,
                error: Some(format!("Failed to set TTL: {}", e)),
            };
        }
    }

    // Enable IP_RECVERR to get ICMP errors in the error queue
    let fd = socket.as_raw_fd();
    unsafe {
        let optval: libc::c_int = 1;
        if ipv6 {
            libc::setsockopt(
                fd,
                libc::SOL_IPV6,
                libc::IPV6_RECVERR,
                &optval as *const libc::c_int as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        } else {
            libc::setsockopt(
                fd,
                libc::SOL_IP,
                libc::IP_RECVERR,
                &optval as *const libc::c_int as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }

    // Set non-blocking to poll the error queue
    if let Err(e) = socket.set_nonblocking(true) {
        return ProbeResult {
            ttl,
            ip: None,
            rtt: None,
            reached: false,
            error: Some(format!("Failed to set nonblocking: {}", e)),
        };
    }

    // Bind to local address
    let bind_addr = if ipv6 {
        SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 0)
    } else {
        SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0)
    };

    if let Err(e) = socket.bind(&bind_addr.into()) {
        return ProbeResult {
            ttl,
            ip: None,
            rtt: None,
            reached: false,
            error: Some(format!("Bind failed: {}", e)),
        };
    }

    let dest_addr = SocketAddr::new(dest, port);
    if let Err(e) = socket.connect(&dest_addr.into()) {
        return ProbeResult {
            ttl,
            ip: None,
            rtt: None,
            reached: false,
            error: Some(format!("Connect failed: {}", e)),
        };
    }

    let start = Instant::now();
    let payload = [0u8; 8];
    if let Err(e) = socket.send(&payload) {
        return ProbeResult {
            ttl,
            ip: None,
            rtt: None,
            reached: false,
            error: Some(format!("Send failed: {}", e)),
        };
    }

    // Poll the error queue
    let timeout_ms = timeout.as_millis() as libc::c_int;
    let mut fds = [libc::pollfd {
        fd,
        events: libc::POLLERR,
        revents: 0,
    }];

    let poll_res = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
    let rtt = start.elapsed();

    if poll_res > 0 && (fds[0].revents & libc::POLLERR) != 0 {
        if let Some((hop_ip, icmp_type, icmp_code)) = recv_error_queue(fd, ipv6) {
            // Reached destination?
            // UDP Port Unreachable is type=3, code=3 for IPv4, or type=1, code=4 for IPv6.
            let reached = if ipv6 {
                (icmp_type == 1 && icmp_code == 4) || hop_ip == dest
            } else {
                (icmp_type == 3 && icmp_code == 3) || hop_ip == dest
            };
            return ProbeResult {
                ttl,
                ip: Some(hop_ip),
                rtt: Some(rtt),
                reached,
                error: None,
            };
        }
    }

    ProbeResult {
        ttl,
        ip: None,
        rtt: None,
        reached: false,
        error: None, // Timeout / packet drop
    }
}

fn send_icmp_probe(dest: IpAddr, ttl: u8, seq: u16, timeout: Duration) -> ProbeResult {
    let ipv6 = dest.is_ipv6();
    let domain = if ipv6 { Domain::IPV6 } else { Domain::IPV4 };

    let proto = if ipv6 {
        Protocol::ICMPV6
    } else {
        Protocol::ICMPV4
    };
    let mut is_raw = false;
    let socket = match Socket::new(domain, Type::RAW, Some(proto)) {
        Ok(s) => {
            is_raw = true;
            s
        }
        Err(_) => {
            // Fallback to SOCK_DGRAM (ping sockets) if raw socket fails
            match Socket::new(domain, Type::DGRAM, Some(proto)) {
                Ok(s) => s,
                Err(e) => {
                    return ProbeResult {
                        ttl,
                        ip: None,
                        rtt: None,
                        reached: false,
                        error: Some(format!(
                            "Socket creation failed (try running with sudo/cap_net_raw): {}",
                            e
                        )),
                    }
                }
            }
        }
    };

    if ipv6 {
        if let Err(e) = socket.set_unicast_hops_v6(ttl as u32) {
            return ProbeResult {
                ttl,
                ip: None,
                rtt: None,
                reached: false,
                error: Some(format!("Failed to set IPv6 hops: {}", e)),
            };
        }
    } else {
        if let Err(e) = socket.set_ttl(ttl as u32) {
            return ProbeResult {
                ttl,
                ip: None,
                rtt: None,
                reached: false,
                error: Some(format!("Failed to set TTL: {}", e)),
            };
        }
    }

    let fd = socket.as_raw_fd();
    unsafe {
        let optval: libc::c_int = 1;
        if ipv6 {
            libc::setsockopt(
                fd,
                libc::SOL_IPV6,
                libc::IPV6_RECVERR,
                &optval as *const libc::c_int as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        } else {
            libc::setsockopt(
                fd,
                libc::SOL_IP,
                libc::IP_RECVERR,
                &optval as *const libc::c_int as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }

    if let Err(e) = socket.set_nonblocking(true) {
        return ProbeResult {
            ttl,
            ip: None,
            rtt: None,
            reached: false,
            error: Some(format!("Failed to set nonblocking: {}", e)),
        };
    }

    let bind_addr = if ipv6 {
        SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 0)
    } else {
        SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0)
    };

    if let Err(e) = socket.bind(&bind_addr.into()) {
        return ProbeResult {
            ttl,
            ip: None,
            rtt: None,
            reached: false,
            error: Some(format!("Bind failed: {}", e)),
        };
    }

    // Connect to destination to route echo reply packets to us
    let dest_addr = SocketAddr::new(dest, 0);
    if let Err(e) = socket.connect(&dest_addr.into()) {
        return ProbeResult {
            ttl,
            ip: None,
            rtt: None,
            reached: false,
            error: Some(format!("Connect failed: {}", e)),
        };
    }

    // Construct ICMP Echo Request
    let mut packet = vec![0u8; 8 + 32]; // 8-byte header, 32-byte payload
    if ipv6 {
        packet[0] = 128; // Type: ICMPv6 Echo Request
    } else {
        packet[0] = 8; // Type: ICMPv4 Echo Request
    }
    packet[1] = 0; // Code

    // Identifier (bytes 4-5) and Sequence (bytes 6-7)
    let ident = unsafe { libc::getpid() as u16 };
    packet[4..6].copy_from_slice(&ident.to_be_bytes());
    packet[6..8].copy_from_slice(&seq.to_be_bytes());

    // Checksum for IPv4
    if !ipv6 {
        let cs = checksum(&packet);
        packet[2..4].copy_from_slice(&cs.to_be_bytes());
    }

    let start = Instant::now();
    if let Err(e) = socket.send(&packet) {
        return ProbeResult {
            ttl,
            ip: None,
            rtt: None,
            reached: false,
            error: Some(format!("Send failed: {}", e)),
        };
    }

    // Poll for both regular reads (POLLIN - for Echo Reply) and errors (POLLERR - for TTL Expired)
    let timeout_ms = timeout.as_millis() as libc::c_int;
    let mut fds = [libc::pollfd {
        fd,
        events: libc::POLLIN | libc::POLLERR,
        revents: 0,
    }];

    let poll_res = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
    let rtt = start.elapsed();

    if poll_res > 0 {
        // Check for error queue first (Intermediate hops sending TTL Exceeded)
        if (fds[0].revents & libc::POLLERR) != 0 {
            if let Some((hop_ip, _, _)) = recv_error_queue(fd, ipv6) {
                return ProbeResult {
                    ttl,
                    ip: Some(hop_ip),
                    rtt: Some(rtt),
                    reached: hop_ip == dest,
                    error: None,
                };
            }
        }

        // Check for read queue (Destination responding with Echo Reply)
        if (fds[0].revents & libc::POLLIN) != 0 {
            let mut read_buf = [0u8; 512];
            let recv_res = unsafe {
                libc::recv(
                    fd,
                    read_buf.as_mut_ptr() as *mut libc::c_void,
                    read_buf.len(),
                    0,
                )
            };
            if recv_res >= 0 {
                // Verify ICMP sequence number or ident if needed
                // For DGRAM socket, kernel filters by ident automatically. Let's just confirm it is indeed an echo reply.
                // Echo Reply: IPv4 Type = 0, IPv6 Type = 129
                // IPv4 RAW socket includes the IP header (usually 20 bytes), whereas DGRAM and IPv6 do not.
                let type_offset = if !ipv6 && is_raw {
                    let ihl = (read_buf[0] & 0x0f) * 4;
                    ihl as usize
                } else {
                    0
                };
                let reply_type = read_buf[type_offset];
                let reply_seq =
                    u16::from_be_bytes([read_buf[type_offset + 6], read_buf[type_offset + 7]]);

                let is_reply = if ipv6 {
                    reply_type == 129
                } else {
                    reply_type == 0
                };

                if is_reply && reply_seq == seq {
                    return ProbeResult {
                        ttl,
                        ip: Some(dest),
                        rtt: Some(rtt),
                        reached: true,
                        error: None,
                    };
                }
            }
        }
    }

    ProbeResult {
        ttl,
        ip: None,
        rtt: None,
        reached: false,
        error: None,
    }
}

// TCP traceroute sends SYN packets. Requires raw sockets usually to capture SYN/ACK or RST,
// or we can try establishing a non-blocking TCP socket with low TTL and checking error queue.
// Let's implement unprivileged-friendly TCP probe via standard socket with TTL,
// or fallback to UDP.
fn send_tcp_probe(dest: IpAddr, ttl: u8, port: u16, timeout: Duration) -> ProbeResult {
    let ipv6 = dest.is_ipv6();
    let domain = if ipv6 { Domain::IPV6 } else { Domain::IPV4 };

    // We can use standard TCP sockets. When a connection is attempted,
    // we set the TTL. If it gets time-exceeded, it will queue an error.
    let socket = match Socket::new(domain, Type::STREAM, Some(Protocol::TCP)) {
        Ok(s) => s,
        Err(e) => {
            return ProbeResult {
                ttl,
                ip: None,
                rtt: None,
                reached: false,
                error: Some(format!("TCP socket creation failed: {}", e)),
            }
        }
    };

    if ipv6 {
        let _ = socket.set_unicast_hops_v6(ttl as u32);
    } else {
        let _ = socket.set_ttl(ttl as u32);
    }

    let fd = socket.as_raw_fd();
    unsafe {
        let optval: libc::c_int = 1;
        if ipv6 {
            libc::setsockopt(
                fd,
                libc::SOL_IPV6,
                libc::IPV6_RECVERR,
                &optval as *const libc::c_int as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        } else {
            libc::setsockopt(
                fd,
                libc::SOL_IP,
                libc::IP_RECVERR,
                &optval as *const libc::c_int as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }

    let _ = socket.set_nonblocking(true);
    let dest_addr = SocketAddr::new(dest, port);

    let start = Instant::now();
    let _connect_res = socket.connect(&dest_addr.into()); // usually returns EINPROGRESS

    // Poll error queue (POLLERR) and write readiness (POLLOUT - connection complete)
    let timeout_ms = timeout.as_millis() as libc::c_int;
    let mut fds = [libc::pollfd {
        fd,
        events: libc::POLLOUT | libc::POLLERR,
        revents: 0,
    }];

    let poll_res = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
    let rtt = start.elapsed();

    if poll_res > 0 {
        if (fds[0].revents & libc::POLLERR) != 0 {
            if let Some((hop_ip, _, _)) = recv_error_queue(fd, ipv6) {
                return ProbeResult {
                    ttl,
                    ip: Some(hop_ip),
                    rtt: Some(rtt),
                    reached: hop_ip == dest,
                    error: None,
                };
            }
        }
        if (fds[0].revents & libc::POLLOUT) != 0 {
            // TCP connection established, or RST received! Either way, we reached the destination.
            return ProbeResult {
                ttl,
                ip: Some(dest),
                rtt: Some(rtt),
                reached: true,
                error: None,
            };
        }
    }

    ProbeResult {
        ttl,
        ip: None,
        rtt: None,
        reached: false,
        error: None,
    }
}

fn recv_error_queue(fd: libc::c_int, _ipv6: bool) -> Option<(IpAddr, u8, u8)> {
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };

    // Allocate space for control messages (cmsghdr)
    let mut control_buf = [0u8; 512];
    msg.msg_control = control_buf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = control_buf.len() as _;

    let mut iov = libc::iovec {
        iov_base: [0u8; 512].as_mut_ptr() as *mut libc::c_void,
        iov_len: 512,
    };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;

    let mut sockname: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    msg.msg_name = &mut sockname as *mut _ as *mut libc::c_void;
    msg.msg_namelen = std::mem::size_of::<libc::sockaddr_storage>() as _;

    let res = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_ERRQUEUE) };
    if res < 0 {
        return None;
    }

    let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    while !cmsg.is_null() {
        let level = unsafe { (*cmsg).cmsg_level };
        let type_ = unsafe { (*cmsg).cmsg_type };

        if (level == libc::SOL_IP && type_ == libc::IP_RECVERR)
            || (level == libc::SOL_IPV6 && type_ == libc::IPV6_RECVERR)
        {
            let err_ptr = unsafe { libc::CMSG_DATA(cmsg) } as *const sock_extended_err;
            let err = unsafe { &*err_ptr };

            // ee_origin == 2 (SO_EE_ORIGIN_ICMP), ee_origin == 3 (SO_EE_ORIGIN_ICMP6)
            if err.ee_origin == 2 || err.ee_origin == 3 {
                let offender_ptr = unsafe { err_ptr.add(1) } as *const libc::sockaddr;
                let family = unsafe { (*offender_ptr).sa_family };

                if family == libc::AF_INET as libc::sa_family_t {
                    let sin = offender_ptr as *const libc::sockaddr_in;
                    let ip = IpAddr::V4(std::net::Ipv4Addr::from(u32::from_be(unsafe {
                        (*sin).sin_addr.s_addr
                    })));
                    return Some((ip, err.ee_type, err.ee_code));
                } else if family == libc::AF_INET6 as libc::sa_family_t {
                    let sin6 = offender_ptr as *const libc::sockaddr_in6;
                    let ip_bytes = unsafe { (*sin6).sin6_addr.s6_addr };
                    let ip = IpAddr::V6(std::net::Ipv6Addr::from(ip_bytes));
                    return Some((ip, err.ee_type, err.ee_code));
                }
            }
        }
        cmsg = unsafe { libc::CMSG_NXTHDR(&msg, cmsg) };
    }

    None
}

fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in data.chunks(2) {
        let val = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], 0])
        };
        sum += val as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
