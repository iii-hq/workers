//! Parsers for the two listening-socket sources, kept platform-neutral so
//! both are tested everywhere.

use std::net::{Ipv4Addr, Ipv6Addr};

use super::SocketListener;
use crate::model::AddressFamily;

/// One LISTEN row of `/proc/net/tcp` or `/proc/net/tcp6`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetEntry {
    pub inode: u64,
    pub address: String,
    pub port: u16,
    pub uid: u32,
    pub family: AddressFamily,
}

const TCP_LISTEN: &str = "0A";

/// The kernel prints each 32-bit word of the address as the host-endian
/// integer it holds in memory, so `to_ne_bytes` restores network order on
/// any host.
fn decode_address(hex: &str, family: AddressFamily) -> Option<String> {
    let word = |chunk: &str| u32::from_str_radix(chunk, 16).ok().map(u32::to_ne_bytes);
    match family {
        AddressFamily::Ipv4 if hex.len() == 8 => Some(Ipv4Addr::from(word(hex)?).to_string()),
        AddressFamily::Ipv6 if hex.len() == 32 => {
            let mut bytes = [0u8; 16];
            for (i, chunk) in bytes.chunks_mut(4).enumerate() {
                chunk.copy_from_slice(&word(&hex[i * 8..i * 8 + 8])?);
            }
            Some(Ipv6Addr::from(bytes).to_string())
        }
        _ => None,
    }
}

pub fn parse_proc_net_tcp(content: &str, family: AddressFamily) -> Vec<NetEntry> {
    content
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 || fields[3] != TCP_LISTEN {
                return None;
            }
            let (address, port) = fields[1].split_once(':')?;
            Some(NetEntry {
                inode: fields[9].parse().ok().filter(|inode| *inode != 0)?,
                address: decode_address(address, family)?,
                port: u16::from_str_radix(port, 16).ok()?,
                uid: fields[7].parse().ok()?,
                family,
            })
        })
        .collect()
}

/// `lsof -nP -iTCP -sTCP:LISTEN -FpPtn` output: a `p<pid>` line opens a
/// process, then per socket a `t<IPv4|IPv6>` type and an `n<address>:<port>`
/// name (`*:3000`, `127.0.0.1:3000`, `[::1]:3000`).
pub fn parse_lsof(output: &str) -> Vec<SocketListener> {
    let mut out = Vec::new();
    let mut pid = None;
    let mut family = None;
    for line in output.lines() {
        let (tag, value) = line.split_at(line.len().min(1));
        match tag {
            "p" => {
                pid = value.parse().ok();
                family = None;
            }
            "f" => family = None,
            "t" => {
                family = match value {
                    "IPv4" => Some(AddressFamily::Ipv4),
                    "IPv6" => Some(AddressFamily::Ipv6),
                    _ => None,
                }
            }
            "n" => {
                let (Some(pid), Some((host, port))) = (pid, value.rsplit_once(':')) else {
                    continue;
                };
                let Ok(port) = port.parse::<u16>() else {
                    continue;
                };
                let host = host.trim_start_matches('[').trim_end_matches(']');
                let family = family.unwrap_or(if host.contains(':') {
                    AddressFamily::Ipv6
                } else {
                    AddressFamily::Ipv4
                });
                let address = match (host, family) {
                    ("*", AddressFamily::Ipv4) => "0.0.0.0".to_string(),
                    ("*", AddressFamily::Ipv6) => "::".to_string(),
                    (host, _) => host.to_string(),
                };
                let listener = SocketListener {
                    port,
                    address,
                    family,
                    pid,
                };
                if !out.contains(&listener) {
                    out.push(listener);
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:BFEE 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 5550001 1 0000000000000000 100 0 0 10 0
   1: 00000000:0C27 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 5550002 1 0000000000000000 100 0 0 10 0
   2: 0100007F:BFEE 0100007F:A1B2 01 00000000:00000000 00:00000000 00000000  1000        0 5550003 1 0000000000000000 20 4 30 10 -1
";

    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:0BB8 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 6660001 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 6660002 1 0000000000000000 100 0 0 10 0
";

    #[cfg(target_endian = "little")]
    #[test]
    fn proc_net_tcp_keeps_listen_rows_only() {
        let rows = parse_proc_net_tcp(TCP, AddressFamily::Ipv4);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].address.as_str(), rows[0].port),
            ("127.0.0.1", 49134)
        );
        assert_eq!((rows[1].address.as_str(), rows[1].port), ("0.0.0.0", 3111));
        assert_eq!(rows[1].inode, 5550002);
        assert_eq!(rows[1].uid, 1000);
    }

    #[cfg(target_endian = "little")]
    #[test]
    fn proc_net_tcp6_decodes_addresses() {
        let rows = parse_proc_net_tcp(TCP6, AddressFamily::Ipv6);
        assert_eq!((rows[0].address.as_str(), rows[0].port), ("::1", 3000));
        assert_eq!(
            (rows[1].address.as_str(), rows[1].port, rows[1].uid),
            ("::", 8080, 0)
        );
    }

    #[test]
    fn lsof_output_maps_sockets_to_pids() {
        let output = "p101\ncnode\nf23\ntIPv4\nn*:3111\nf24\ntIPv6\nn[::1]:3000\np202\ncade\nf5\ntIPv4\nn127.0.0.1:49134\n";
        let rows = parse_lsof(output);
        assert_eq!(
            rows,
            vec![
                SocketListener {
                    port: 3111,
                    address: "0.0.0.0".into(),
                    family: AddressFamily::Ipv4,
                    pid: 101
                },
                SocketListener {
                    port: 3000,
                    address: "::1".into(),
                    family: AddressFamily::Ipv6,
                    pid: 101
                },
                SocketListener {
                    port: 49134,
                    address: "127.0.0.1".into(),
                    family: AddressFamily::Ipv4,
                    pid: 202
                },
            ]
        );
    }
}
