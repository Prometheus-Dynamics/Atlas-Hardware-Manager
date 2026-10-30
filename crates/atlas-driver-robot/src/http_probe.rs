//! How a device package's data reaches an address on the robot network
//! (`atlas_devices::Probe`): GETs with the driver's client, and TCP
//! connects within the probe's timeout.

use std::net::Ipv4Addr;

use atlas_devices::{HttpAnswer, Probe};

pub(crate) struct HttpProbe<'a> {
    pub http: &'a reqwest::Client,
    pub ip: Ipv4Addr,
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
        futures::future::join_all(ports.into_iter().map(|port| async move {
            let connect = tokio::net::TcpStream::connect((ip, port));
            matches!(
                tokio::time::timeout(crate::probe::TIMEOUT, connect).await,
                Ok(Ok(_))
            )
        }))
        .await
    }
}
