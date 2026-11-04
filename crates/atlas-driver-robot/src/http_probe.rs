//! How a device package's data reaches an address on the robot network
//! (`atlas_devices::Probe`): GETs with the driver's client, and TCP
//! connects within the probe's timeout.
//!
//! A port found open is remembered, not asked again each scan: a connect
//! that says nothing is what an sshd counts against this computer
//! (OpenSSH's PerSourcePenalties), and every scan would be one.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::sync::Mutex;

use atlas_devices::{HttpAnswer, Probe};

/// The ports already found open, by address.
#[derive(Default)]
pub struct OpenPorts(Mutex<BTreeSet<(Ipv4Addr, u16)>>);

impl OpenPorts {
    fn known(&self, ip: Ipv4Addr, port: u16) -> bool {
        self.0.lock().unwrap().contains(&(ip, port))
    }

    fn note(&self, ip: Ipv4Addr, port: u16) {
        self.0.lock().unwrap().insert((ip, port));
    }
}

pub(crate) struct HttpProbe<'a> {
    pub http: &'a reqwest::Client,
    pub ip: Ipv4Addr,
    pub open: &'a OpenPorts,
}

impl Probe for HttpProbe<'_> {
    async fn get(&self, port: u16, path: &str) -> Option<HttpAnswer> {
        let response = self
            .http
            .get(format!("http://{}:{port}{path}", self.ip))
            .send()
            .await
            .ok()?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
            })
            .collect();
        let body = response.text().await.unwrap_or_default();
        Some(HttpAnswer {
            status,
            headers,
            body,
        })
    }

    async fn open(&self, ports: Vec<u16>) -> Vec<bool> {
        let ip = self.ip;
        let seen = self.open;
        futures::future::join_all(ports.into_iter().map(|port| async move {
            if seen.known(ip, port) {
                return true;
            }
            let connect = tokio::net::TcpStream::connect((ip, port));
            let open = matches!(
                tokio::time::timeout(crate::probe::TIMEOUT, connect).await,
                Ok(Ok(_))
            );
            if open {
                seen.note(ip, port);
            }
            open
        }))
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_open_port_is_connected_to_once_however_many_scans() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = tokio::spawn(async move {
            let mut count = 0;
            while tokio::time::timeout(std::time::Duration::from_millis(300), listener.accept())
                .await
                .is_ok()
            {
                count += 1;
            }
            count
        });
        let http = reqwest::Client::new();
        let open = OpenPorts::default();
        let probe = HttpProbe {
            http: &http,
            ip: Ipv4Addr::LOCALHOST,
            open: &open,
        };
        for _ in 0..5 {
            assert_eq!(probe.open(vec![port]).await, [true]);
        }
        assert_eq!(accepted.await.unwrap(), 1);
    }
}
