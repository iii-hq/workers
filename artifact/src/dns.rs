//! Does a fresh quick-tunnel hostname resolve yet?
//!
//! A brand-new `*.trycloudflare.com` name takes a few seconds to a minute to
//! appear in DNS. Handing it out earlier gives the recipient "site not
//! found", and asking the host's own resolver too early is worse: it caches
//! the miss for minutes. So the check asks a PUBLIC resolver directly with one
//! minimal UDP query, the view a recipient elsewhere gets, and never touches
//! the local cache. Names seen resolving are remembered for the process.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use tokio::net::UdpSocket;

/// How long one query waits for its answer.
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

enum Mode {
    /// Ask this resolver (`ip:port`).
    Public(SocketAddr),
    /// No check: every name counts as resolving.
    Off,
    /// Tests: answer from a fixed verdict.
    Fake(bool),
}

pub struct Dns {
    mode: Mode,
    seen: Mutex<HashSet<String>>,
}

impl Dns {
    /// `resolver` is `ip:port`; empty disables the check.
    pub fn new(resolver: &str) -> Result<Self, String> {
        let mode = if resolver.trim().is_empty() {
            Mode::Off
        } else {
            Mode::Public(
                resolver
                    .trim()
                    .parse()
                    .map_err(|_| format!("dns_check_resolver `{resolver}` is not ip:port"))?,
            )
        };
        Ok(Self::with(mode))
    }

    pub fn fake(resolves: bool) -> Self {
        Self::with(Mode::Fake(resolves))
    }

    fn with(mode: Mode) -> Self {
        Self {
            mode,
            seen: Mutex::new(HashSet::new()),
        }
    }

    /// `true` once `url`'s host has an address in public DNS.
    pub async fn resolves(&self, url: &str) -> bool {
        let Some(host) = host_of(url) else {
            return false;
        };
        if self.seen.lock().expect("dns cache lock").contains(host) {
            return true;
        }
        let ok = match &self.mode {
            Mode::Off => true,
            Mode::Fake(verdict) => *verdict,
            Mode::Public(resolver) => match query_a(*resolver, host).await {
                Ok(ok) => ok,
                Err(e) => {
                    tracing::debug!(error = %e, host, "public dns check failed");
                    false
                }
            },
        };
        if ok {
            self.seen
                .lock()
                .expect("dns cache lock")
                .insert(host.to_string());
        }
        ok
    }
}

/// The host of an `https://host[:port]/...` URL.
fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', ':', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

/// One recursive A query; `Ok(true)` when the answer is NOERROR with records.
async fn query_a(resolver: SocketAddr, host: &str) -> Result<bool, String> {
    let id = u16::from_be_bytes(random_pair()?);
    let packet = build_query(id, host)?;
    let bind: SocketAddr = if resolver.is_ipv4() {
        "0.0.0.0:0".parse().expect("literal v4")
    } else {
        "[::]:0".parse().expect("literal v6")
    };
    let socket = UdpSocket::bind(bind).await.map_err(|e| e.to_string())?;
    socket
        .send_to(&packet, resolver)
        .await
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 1500];
    let len = tokio::time::timeout(QUERY_TIMEOUT, socket.recv(&mut buf))
        .await
        .map_err(|_| "dns query timed out".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(answer_has_records(&buf[..len], id))
}

fn random_pair() -> Result<[u8; 2], String> {
    let mut buf = [0u8; 2];
    getrandom::getrandom(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// A standard query with recursion desired: header, one A/IN question.
fn build_query(id: u16, host: &str) -> Result<Vec<u8>, String> {
    let mut packet = Vec::with_capacity(18 + host.len());
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in host.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("`{host}` is not a valid hostname"));
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&[0, 1, 0, 1]);
    Ok(packet)
}

/// NOERROR, matching id, a response, and at least one A or AAAA record in
/// the answer section. A NOERROR answer may hold only a CNAME (RFC 2308
/// NODATA): that is not an address yet. Truncated packets are rejected.
fn answer_has_records(reply: &[u8], id: u16) -> bool {
    if reply.len() < 12 || reply[..2] != id.to_be_bytes() {
        return false;
    }
    let is_response = reply[2] & 0x80 != 0;
    let truncated = reply[2] & 0x02 != 0;
    let rcode = reply[3] & 0x0f;
    if !is_response || truncated || rcode != 0 {
        return false;
    }
    let questions = u16::from_be_bytes([reply[4], reply[5]]);
    let answers = u16::from_be_bytes([reply[6], reply[7]]);
    let mut at = 12;
    for _ in 0..questions {
        let Some(next) = skip_name(reply, at) else {
            return false;
        };
        at = next + 4;
    }
    for _ in 0..answers {
        let Some(next) = skip_name(reply, at) else {
            return false;
        };
        let Some(fixed) = reply.get(next..next + 10) else {
            return false;
        };
        let rtype = u16::from_be_bytes([fixed[0], fixed[1]]);
        let rdlength = usize::from(u16::from_be_bytes([fixed[8], fixed[9]]));
        let rdata_end = next + 10 + rdlength;
        if rdata_end > reply.len() {
            return false;
        }
        if (rtype == 1 && rdlength == 4) || (rtype == 28 && rdlength == 16) {
            return true;
        }
        at = rdata_end;
    }
    false
}

/// The offset just past a (possibly compressed) domain name at `at`.
fn skip_name(packet: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let len = *packet.get(at)?;
        match len {
            0 => return Some(at + 1),
            l if l & 0xc0 == 0xc0 => {
                packet.get(at + 1)?;
                return Some(at + 2);
            }
            l if l & 0xc0 == 0 => at += 1 + usize::from(l),
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_is_taken_from_a_url() {
        assert_eq!(
            host_of("https://a-b.trycloudflare.com/s/x"),
            Some("a-b.trycloudflare.com")
        );
        assert_eq!(host_of("https://a.example:8443"), Some("a.example"));
        assert_eq!(host_of("https:///s"), None);
    }

    #[test]
    fn query_encodes_labels_and_one_a_question() {
        let q = build_query(0xabcd, "a.bc").expect("query");
        assert_eq!(&q[..2], &[0xab, 0xcd]);
        assert_eq!(&q[2..4], &[0x01, 0x00], "recursion desired");
        assert_eq!(&q[4..6], &[0, 1], "one question");
        assert_eq!(&q[12..], &[1, b'a', 2, b'b', b'c', 0, 0, 1, 0, 1]);
    }

    #[test]
    fn invalid_hostnames_are_refused() {
        assert!(build_query(1, "a..b").is_err());
        assert!(build_query(1, &"x".repeat(64)).is_err());
    }

    /// A reply to `build_query(id, "a.bc")` with the given answer records.
    fn reply(id: u16, rcode: u8, answers: &[(u16, &[u8])]) -> Vec<u8> {
        let mut packet = build_query(id, "a.bc").expect("query");
        packet[2] = 0x81;
        packet[3] = 0x80 | rcode;
        packet[6..8].copy_from_slice(&(answers.len() as u16).to_be_bytes());
        for (rtype, rdata) in answers {
            packet.extend_from_slice(&[0xc0, 12]);
            packet.extend_from_slice(&rtype.to_be_bytes());
            packet.extend_from_slice(&[0, 1, 0, 0, 0, 60]);
            packet.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
            packet.extend_from_slice(rdata);
        }
        packet
    }

    #[test]
    fn only_an_address_record_counts() {
        assert!(answer_has_records(
            &reply(7, 0, &[(1, &[104, 16, 230, 132])]),
            7
        ));
        assert!(
            answer_has_records(&reply(7, 0, &[(28, &[0; 16])]), 7),
            "AAAA"
        );
        assert!(
            !answer_has_records(&reply(8, 0, &[(1, &[1, 2, 3, 4])]), 7),
            "wrong id"
        );
        assert!(!answer_has_records(&reply(7, 3, &[]), 7), "NXDOMAIN");
        assert!(!answer_has_records(&reply(7, 0, &[]), 7), "NODATA");
        let cname_only = reply(7, 0, &[(5, &[1, b'x', 0])]);
        assert!(
            !answer_has_records(&cname_only, 7),
            "a CNAME alone is not an address"
        );
        let cname_then_a = reply(7, 0, &[(5, &[1, b'x', 0]), (1, &[1, 2, 3, 4])]);
        assert!(answer_has_records(&cname_then_a, 7));
    }

    #[test]
    fn truncated_packets_are_rejected() {
        let full = reply(7, 0, &[(1, &[1, 2, 3, 4])]);
        assert!(
            !answer_has_records(&full[..full.len() - 2], 7),
            "rdata cut short"
        );
        assert!(!answer_has_records(&full[..12], 7), "header only");
        let mut tc = full.clone();
        tc[2] |= 0x02;
        assert!(!answer_has_records(&tc, 7), "TC bit set");
    }

    #[tokio::test]
    async fn a_disabled_check_always_resolves_and_bad_resolvers_are_refused() {
        let dns = Dns::new("").expect("off");
        assert!(dns.resolves("https://new.trycloudflare.com/s/t").await);
        assert!(Dns::new("not-an-addr").is_err());
    }
}
